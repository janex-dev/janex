// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Project and shell selection, environment rendering, and process leases.

use super::{SdkManager, java_name};
use crate::{Result, error::invalid};
use janex_java::runtime::{JavaOptions, JavaRuntime};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

/// Shell syntax used for explicit environment activation.
#[derive(Clone, Copy, Debug)]
pub enum Shell {
    /// POSIX-compatible sh, bash, or zsh.
    Sh,
    /// PowerShell.
    PowerShell,
    /// Fish shell.
    Fish,
}

/// Selected SDK homes retaining uninstall-prevention leases throughout child execution.
#[derive(Debug)]
pub struct SdkExecution {
    /// Selected SDKs keyed by family.
    selections: BTreeMap<String, SelectedSdk>,
    /// Shared leases; unregistered external homes do not have a lease.
    _leases: Vec<fs::File>,
}

/// One effective SDK selection and the origin that determined it.
#[derive(Debug, Serialize)]
pub struct SelectedSdk {
    /// Absolute SDK home.
    pub home: PathBuf,
    /// Concrete registered selector, absent for an unregistered environment or system home.
    pub target: Option<String>,
    /// Registered installation ID, absent for an unregistered home.
    pub installation: Option<String>,
    /// Selection origin, following execution precedence.
    pub source: SelectionSource,
}

/// The input that selected an SDK home for execution.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SelectionSource {
    /// A target explicitly supplied for this operation.
    Explicit,
    /// An inherited SDK home variable, including values set by shell integration.
    Environment(String),
    /// The project toolchain file containing the request.
    Project(PathBuf),
    /// The user's persistent family/platform default.
    Default,
    /// An automatically discovered Java installation.
    System,
}

impl SdkExecution {
    /// Returns the selected SDKs and their origins, keyed by family.
    pub fn selections(&self) -> &BTreeMap<String, SelectedSdk> {
        &self.selections
    }

    /// Executes a command with selected SDK environment variables and bin directories on PATH.
    /// Waits for the child with inherited standard streams; the parent environment is unchanged.
    /// Windows batch launchers use standard command escaping and may reject unrepresentable arguments.
    pub fn execute(&self, command: &OsStr, arguments: &[OsString]) -> Result<ExitStatus> {
        let mut executable = command.to_owned();
        if Path::new(command).components().count() == 1 {
            'homes: for selected in self.selections.values() {
                let home = &selected.home;
                let suffixes: &[&str] = if cfg!(windows) && Path::new(command).extension().is_none()
                {
                    &[".exe", ".cmd", ".bat"]
                } else {
                    &[""]
                };
                for suffix in suffixes {
                    let mut name = command.to_owned();
                    name.push(suffix);
                    let local = home.join("bin").join(name);
                    if local.is_file() {
                        executable = local.into_os_string();
                        break 'homes;
                    }
                }
            }
        }
        let mut child = Command::new(executable);
        child.args(arguments).env("PATH", self.path()?);
        for (family, selected) in &self.selections {
            child.env(home_variable(family), &selected.home);
        }
        Ok(child.status()?)
    }

    /// Produces shell assignments; callers must evaluate the output in their own shell.
    pub fn environment(&self, shell: Shell) -> Result<String> {
        let quote = |s: &str| -> String {
            match shell {
                Shell::Sh => format!("'{}'", s.replace('\'', "'\\''")),
                Shell::PowerShell => format!("'{}'", s.replace('\'', "''")),
                Shell::Fish => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
            }
        };
        let mut output = String::new();
        for (family, selected) in &self.selections {
            let name = home_variable(family);
            let value = quote(
                selected
                    .home
                    .to_str()
                    .ok_or_else(|| invalid("SDK home is not Unicode"))?,
            );
            output.push_str(&match shell {
                Shell::Sh => format!("export {name}={value}\n"),
                Shell::PowerShell => format!("$env:{name} = {value}\n"),
                Shell::Fish => format!("set -gx {name} {value}\n"),
            });
        }
        let path = self.path()?;
        output.push_str(&match shell {
            Shell::Sh => format!(
                "export PATH={}\n",
                quote(
                    path.to_str()
                        .ok_or_else(|| invalid("PATH is not Unicode"))?
                )
            ),
            Shell::PowerShell => format!(
                "$env:PATH = {}\n",
                quote(
                    path.to_str()
                        .ok_or_else(|| invalid("PATH is not Unicode"))?
                )
            ),
            Shell::Fish => {
                let paths = std::env::split_paths(&path)
                    .map(|p| {
                        p.to_str()
                            .map(quote)
                            .ok_or_else(|| invalid("PATH is not Unicode"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                format!("set -gx PATH {}\n", paths.join(" "))
            }
        });
        Ok(output)
    }

    /// Prepends selected SDK bins, removing duplicate selected paths from the inherited PATH.
    fn path(&self) -> Result<OsString> {
        let bins = self
            .selections
            .values()
            .map(|selected| selected.home.join("bin"))
            .collect::<Vec<_>>();
        let current = std::env::var_os("PATH").unwrap_or_default();
        std::env::join_paths(
            bins.iter()
                .cloned()
                .chain(std::env::split_paths(&current).filter(|p| !bins.contains(p))),
        )
        .map_err(|e| invalid(format!("invalid SDK PATH: {e}")))
    }
}

/// Returns the conventional environment variable for a validated SDK family.
pub(super) fn home_variable(family: &str) -> &'static str {
    match family {
        "java" => "JAVA_HOME",
        "gradle" => "GRADLE_HOME",
        "maven" => "MAVEN_HOME",
        _ => unreachable!(),
    }
}

impl SdkManager {
    /// Selects installed SDKs without downloading. Explicit targets precede each family's home
    /// variable, project request, and global default. Duplicate explicit families are rejected.
    /// Java discovery is a final optional fallback; other commands do not require Java to exist.
    pub fn execution(&self, targets: &[String], directory: Option<&Path>) -> Result<SdkExecution> {
        let _lock = self.lock(false)?;
        let state = self.read()?;
        let project_path = directory.and_then(project_file);
        let project = project_path
            .as_deref()
            .map(|path| parse_project(&read_project(path)?))
            .transpose()?
            .unwrap_or_default();
        let mut explicit = BTreeMap::new();
        for target in targets {
            let installed = self.resolve_in(&state, target)?.0;
            if explicit.insert(installed.sdk.family(), installed).is_some() {
                return Err(invalid(
                    "only one explicit selection per SDK family is allowed",
                ));
            }
        }
        let mut selections = BTreeMap::new();
        let mut leases = Vec::new();
        let families = super::PRODUCTS
            .iter()
            .map(|p| p.family)
            .collect::<std::collections::BTreeSet<_>>();
        for family in families {
            let (selected, source) = if let Some(installed) = explicit.remove(family) {
                (Some(installed), SelectionSource::Explicit)
            } else if let Some(path) =
                std::env::var_os(home_variable(family)).filter(|s| !s.is_empty())
            {
                let home = PathBuf::from(path).canonicalize()?;
                validate_home(&home, family)?;
                let installed = state.installations.iter().find(|i| {
                    i.sdk.family() == family
                        && self
                            .home(i)
                            .ok()
                            .and_then(|p| p.canonicalize().ok())
                            .as_ref()
                            == Some(&home)
                });
                if let Some(installed) = installed {
                    installed.sdk.check_host()?;
                    leases.push(self.lease(&installed.id)?);
                }
                selections.insert(
                    family.into(),
                    SelectedSdk {
                        home: janex_java::runtime::java_path(&home),
                        target: installed.map(|i| i.sdk.target()),
                        installation: installed.map(|i| i.id.clone()),
                        source: SelectionSource::Environment(home_variable(family).into()),
                    },
                );
                continue;
            } else if let Some(target) = project.get(family) {
                (
                    Some(self.resolve_in(&state, target)?.0),
                    SelectionSource::Project(project_path.clone().unwrap()),
                )
            } else {
                (self.default_in(&state, family)?, SelectionSource::Default)
            };
            if let Some(installed) = selected {
                if installed.sdk.family() != family {
                    return Err(invalid("selected SDK belongs to a different family"));
                }
                installed.sdk.check_host()?;
                let home = self.home(&installed)?;
                validate_home(&home, family)?;
                leases.push(self.lease(&installed.id)?);
                selections.insert(
                    family.into(),
                    SelectedSdk {
                        home: janex_java::runtime::java_path(&home),
                        target: Some(installed.sdk.target()),
                        installation: Some(installed.id),
                        source,
                    },
                );
            } else if family == "java"
                && let Ok(runtimes) = janex_java::runtime::runtimes(&JavaOptions::default())
                && let Some(runtime) = runtimes.into_iter().next()
            {
                selections.insert(
                    family.into(),
                    SelectedSdk {
                        home: janex_java::runtime::java_path(&runtime.home),
                        target: None,
                        installation: None,
                        source: SelectionSource::System,
                    },
                );
            }
        }
        Ok(SdkExecution {
            selections,
            _leases: leases,
        })
    }

    /// Atomically writes project selections after resolving every target, preserving other families.
    /// Requires at least one target and rejects duplicate families. Exact pins use local installation IDs.
    pub fn use_project(&self, targets: &[String], directory: &Path, pin: bool) -> Result<PathBuf> {
        if targets.is_empty() {
            return Err(invalid("use --project requires at least one target"));
        }
        let _lock = self.lock(false)?;
        let state = self.read()?;
        let mut updates = BTreeMap::new();
        for target in targets {
            let (installation, request) = self.resolve_in(&state, target)?;
            let value = if pin || *target == installation.id {
                installation.id
            } else {
                target.clone()
            };
            if updates.insert(request.family().to_owned(), value).is_some() {
                return Err(invalid(
                    "only one project selection per SDK family is allowed",
                ));
            }
        }
        let path = directory.join(".janex-toolchains.toml");
        let mut project = if path.exists() {
            parse_project(&read_project(&path)?)?
        } else {
            BTreeMap::new()
        };
        project.extend(updates);
        let mut text = String::new();
        for (family, target) in project {
            // Validated ASCII targets use JSON quoting compatible with TOML basic strings.
            text.push_str(&format!(
                "{family} = {}\n",
                serde_json::to_string(&target).map_err(|e| invalid(e.to_string()))?
            ));
        }
        super::state::publish(&path, text.as_bytes())?;
        Ok(path)
    }
}

/// Checks the platform entry point before exposing a selected home.
pub(super) fn validate_home(home: &Path, family: &str) -> Result<()> {
    let name = match family {
        "java" => java_name(),
        "gradle" if cfg!(windows) => "gradle.bat",
        "gradle" => "gradle",
        "maven" if cfg!(windows) => "mvn.cmd",
        "maven" => "mvn",
        _ => return Err(invalid("unknown SDK family")),
    };
    if home.join("bin").join(name).is_file() {
        Ok(())
    } else {
        Err(invalid(format!(
            "selected {family} home has no {name} executable"
        )))
    }
}

/// Parses known project keys and rejects non-string or mismatched family requests.
fn parse_project(text: &str) -> Result<BTreeMap<String, String>> {
    let project: BTreeMap<String, String> =
        toml::from_str(text).map_err(|e| invalid(format!("invalid project toolchains: {e}")))?;
    for (family, target) in &project {
        if !matches!(family.as_str(), "java" | "gradle" | "maven") {
            return Err(invalid("unknown project SDK family"));
        }
        if target.len() == 64
            && target
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            continue;
        }
        if super::SdkRequest::parse(target)?.family() != family {
            return Err(invalid("project SDK family mismatch"));
        }
    }
    Ok(project)
}

/// Reads the nearest project file, stopping at a repository boundary.
pub(super) fn project_requests(directory: &Path) -> Result<BTreeMap<String, String>> {
    project_file(directory)
        .map(|path| parse_project(&read_project(&path)?))
        .transpose()
        .map(Option::unwrap_or_default)
}

/// Finds the nearest project toolchain file without crossing a repository boundary.
fn project_file(directory: &Path) -> Option<PathBuf> {
    for directory in directory.ancestors() {
        let path = directory.join(".janex-toolchains.toml");
        if path.is_file() {
            return Some(path);
        }
        if directory.join(".git").exists() {
            break;
        }
    }
    None
}

/// Reads a small UTF-8 project file, rejecting unexpectedly large input.
fn read_project(path: &Path) -> Result<String> {
    let mut text = String::new();
    fs::File::open(path)?
        .take(65537)
        .read_to_string(&mut text)?;
    if text.len() > 65536 {
        return Err(invalid("project toolchains file is too large"));
    }
    Ok(text)
}

/// Returns application runtimes with managed-installation leases, ignoring project configuration.
pub(crate) fn application_runtimes(
    options: &JavaOptions,
) -> Result<Vec<(JavaRuntime, Option<fs::File>)>> {
    let manager = match SdkManager::user() {
        Ok(manager) => manager,
        Err(_) => {
            return Ok(janex_java::runtime::runtimes(options)?
                .into_iter()
                .map(|runtime| (runtime, None))
                .collect());
        }
    };
    let _lock = manager.lock(false)?;
    let state = manager.read()?;
    let mut paths = janex_java::runtime::candidates(options)?;
    if !options.is_explicit() {
        let mut managed = Vec::new();
        if let Some(default) = manager.default_in(&state, "java")? {
            managed.push(manager.home(&default)?.join("bin").join(java_name()));
        }
        let native = janex_platform::native_architecture().ok();
        let mut installed = state
            .installations
            .iter()
            .filter(|i| {
                i.sdk
                    .java()
                    .is_some_and(|j| j.platform.as_ref().unwrap().os == std::env::consts::OS)
            })
            .collect::<Vec<_>>();
        installed.sort_by(|a, b| {
            (Some(&a.sdk.platform.as_ref().unwrap().arch) != native.as_ref())
                .cmp(&(Some(&b.sdk.platform.as_ref().unwrap().arch) != native.as_ref()))
                .then_with(|| {
                    super::version_order(
                        b.java_version.as_deref().unwrap_or(b.sdk.version()),
                        a.java_version.as_deref().unwrap_or(a.sdk.version()),
                    )
                })
        });
        for i in installed {
            managed.push(manager.home(i)?.join("bin").join(java_name()));
        }
        let position = usize::from(std::env::var_os("JAVA_HOME").is_some() && !paths.is_empty());
        paths.splice(position..position, managed);
    }
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut failures = Vec::new();
    for path in paths {
        if !seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())) {
            continue;
        }
        let lease = state
            .installations
            .iter()
            .find_map(|i| {
                manager
                    .home(i)
                    .ok()
                    .filter(|home| {
                        path.canonicalize().ok()
                            == home.join("bin").join(java_name()).canonicalize().ok()
                            && path.exists()
                    })
                    .map(|_| i)
            })
            .map(|i| manager.lease(&i.id))
            .transpose()?;
        match JavaRuntime::probe_cached(&path) {
            Ok(runtime) => result.push((runtime, lease)),
            Err(e) if options.is_explicit() => return Err(e.into()),
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    if result.is_empty() {
        return Err(invalid(format!(
            "no usable Java runtime; install a JDK or specify --java-home\n{}",
            failures.join("\n")
        )));
    }
    if !options.is_explicit()
        && manager.default_in(&state, "java")?.is_none()
        && let Ok(native) = janex_platform::native_architecture()
    {
        result.sort_by_key(|(runtime, _)| runtime.architecture != native);
    }
    Ok(result)
}

/// Exposes the project request for explicit environment installation.
pub fn project_java(directory: &Path) -> Result<Option<String>> {
    Ok(project_requests(directory)?.remove("java"))
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Project and shell selection, environment rendering, and process leases.

use super::{SdkManager, java_name};
use crate::{Result, error::invalid};
use janex_java::runtime::{JavaOptions, JavaRuntime};
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
    /// Java and optional portable tool homes keyed by family.
    homes: BTreeMap<String, PathBuf>,
    /// Shared leases; unregistered external homes do not have a lease.
    _leases: Vec<fs::File>,
}

impl SdkExecution {
    /// Returns the selected Java home without altering the calling process.
    pub fn home(&self) -> &Path {
        &self.homes["java"]
    }

    /// Returns selected homes keyed by SDK family.
    pub fn homes(&self) -> &BTreeMap<String, PathBuf> {
        &self.homes
    }

    /// Executes a command with selected SDK environment variables and bin directories on PATH.
    /// Waits for the child with inherited standard streams; the parent environment is unchanged.
    /// Windows batch launchers use standard command escaping and may reject unrepresentable arguments.
    pub fn execute(&self, command: &OsStr, arguments: &[OsString]) -> Result<ExitStatus> {
        let mut executable = command.to_owned();
        if Path::new(command).components().count() == 1 {
            'homes: for home in self.homes.values() {
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
        for (family, home) in &self.homes {
            child.env(home_variable(family), home);
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
        for (family, home) in &self.homes {
            let name = home_variable(family);
            let value = quote(
                home.to_str()
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
            .homes
            .values()
            .map(|home| home.join("bin"))
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
    /// Selects Java and configured tools without downloading. Explicit Java selection precedes
    /// JAVA_HOME, project selection, the Java default, and system Java discovery.
    pub fn execution(
        &self,
        target: Option<&str>,
        directory: Option<&Path>,
    ) -> Result<SdkExecution> {
        self.execution_with(target, None, None, directory)
    }

    /// Selects Java, Gradle, and Maven independently. Explicit selection precedes each family's
    /// environment variable, project requirement, and global default. Only Java uses PATH discovery.
    pub fn execution_with(
        &self,
        java: Option<&str>,
        gradle: Option<&str>,
        maven: Option<&str>,
        directory: Option<&Path>,
    ) -> Result<SdkExecution> {
        let _lock = self.lock(false)?;
        let state = self.read()?;
        let project = directory
            .map(project_requests)
            .transpose()?
            .unwrap_or_default();
        let mut homes = BTreeMap::new();
        let mut leases = Vec::new();
        for (family, explicit) in [("java", java), ("gradle", gradle), ("maven", maven)] {
            let selected = if let Some(target) = explicit {
                Some(self.resolve_in(&state, target)?.0)
            } else if let Some(path) =
                std::env::var_os(home_variable(family)).filter(|s| !s.is_empty())
            {
                let home = PathBuf::from(path).canonicalize()?;
                validate_home(&home, family)?;
                if let Some(installed) = state.installations.iter().find(|i| {
                    i.sdk.family() == family
                        && self
                            .home(i)
                            .ok()
                            .and_then(|p| p.canonicalize().ok())
                            .as_ref()
                            == Some(&home)
                }) {
                    leases.push(self.lease(&installed.id)?);
                }
                homes.insert(family.into(), home);
                continue;
            } else if let Some(target) = project.get(family) {
                Some(self.resolve_in(&state, target)?.0)
            } else {
                self.default_in(&state, family)?
            };
            if let Some(installed) = selected {
                if installed.sdk.family() != family {
                    return Err(invalid("selected SDK belongs to a different family"));
                }
                installed.sdk.check_host()?;
                let home = self.home(&installed)?;
                validate_home(&home, family)?;
                leases.push(self.lease(&installed.id)?);
                homes.insert(family.into(), home);
            } else if family == "java" {
                let runtime = janex_java::runtime::runtimes(&JavaOptions::default())?.remove(0);
                homes.insert(family.into(), runtime.home);
            }
        }
        for home in homes.values_mut() {
            *home = janex_java::runtime::java_path(home);
        }
        Ok(SdkExecution {
            homes,
            _leases: leases,
        })
    }

    /// Writes one project selection, preserving other SDK families. Exact pins use local installation IDs.
    pub fn use_project(&self, target: &str, directory: &Path, pin: bool) -> Result<PathBuf> {
        let _lock = self.lock(false)?;
        let state = self.read()?;
        let (installation, request) = self.resolve_in(&state, target)?;
        let value = if pin || target == installation.id {
            installation.id
        } else {
            target.to_owned()
        };
        let path = directory.join(".janex-toolchains.toml");
        let mut project = if path.exists() {
            parse_project(&read_project(&path)?)?
        } else {
            BTreeMap::new()
        };
        project.insert(request.family().into(), value);
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
    for directory in directory.ancestors() {
        let path = directory.join(".janex-toolchains.toml");
        if path.is_file() {
            return parse_project(&read_project(&path)?);
        }
        if directory.join(".git").exists() {
            break;
        }
    }
    Ok(BTreeMap::new())
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
                    .is_some_and(|j| j.platform.os == std::env::consts::OS)
            })
            .collect::<Vec<_>>();
        installed.sort_by(|a, b| {
            (Some(&a.sdk.java().unwrap().platform.arch) != native.as_ref())
                .cmp(&(Some(&b.sdk.java().unwrap().platform.arch) != native.as_ref()))
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

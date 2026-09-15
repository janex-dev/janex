// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Project and shell selection, environment rendering, and process leases.

use super::{SdkManager, java_name};
use crate::{Result, error::invalid};
use janex_java::runtime::{JavaOptions, JavaRuntime};
use serde::Deserialize;
use std::{
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

/// Selected Java home retaining an SDK lease for the lifetime of child execution.
#[derive(Debug)]
pub struct SdkExecution {
    /// Selected SDK root; includes development tools for JDK installations.
    home: PathBuf,
    /// Shared uninstall-prevention lease, absent for unregistered external Java.
    _lease: Option<fs::File>,
}

impl SdkExecution {
    /// Returns the selected Java home without altering the calling process.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Executes a command with this Java home and its bin directory first on PATH.
    /// Waits for the child with inherited standard streams; the parent environment is unchanged.
    pub fn execute(&self, command: &OsStr, arguments: &[OsString]) -> Result<ExitStatus> {
        let executable = if Path::new(command).components().count() == 1 {
            let name = if cfg!(windows) && Path::new(command).extension().is_none() {
                let mut name = command.to_owned();
                name.push(".exe");
                name
            } else {
                command.to_owned()
            };
            let local = self.home.join("bin").join(name);
            if local.is_file() {
                local.into_os_string()
            } else {
                command.to_owned()
            }
        } else {
            command.to_owned()
        };
        Ok(Command::new(executable)
            .args(arguments)
            .env("JAVA_HOME", &self.home)
            .env("PATH", self.path()?)
            .status()?)
    }

    /// Produces shell assignments; callers must evaluate the output in their own shell.
    pub fn environment(&self, shell: Shell) -> Result<String> {
        let home = self
            .home
            .to_str()
            .ok_or_else(|| invalid("Java home cannot be represented in shell output"))?;
        let path = self.path()?;
        let path = path
            .to_str()
            .ok_or_else(|| invalid("PATH cannot be represented in shell output"))?;
        let quote = |s: &str| -> String {
            match shell {
                Shell::Sh => format!("'{}'", s.replace('\'', "'\\''")),
                Shell::PowerShell => format!("'{}'", s.replace('\'', "''")),
                Shell::Fish => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
            }
        };
        Ok(match shell {
            Shell::Sh => format!(
                "export JAVA_HOME={}\nexport PATH={}\n",
                quote(home),
                quote(path)
            ),
            Shell::PowerShell => format!(
                "$env:JAVA_HOME = {}\n$env:PATH = {}\n",
                quote(home),
                quote(path)
            ),
            Shell::Fish => {
                let paths = std::env::split_paths(&self.path()?)
                    .map(|p| {
                        p.to_str()
                            .map(quote)
                            .ok_or_else(|| invalid("PATH is not Unicode"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                format!(
                    "set -gx JAVA_HOME {}\nset -gx PATH {}\n",
                    quote(home),
                    paths.join(" ")
                )
            }
        })
    }

    /// Prepends SDK tools while avoiding repeated copies of this same bin directory.
    fn path(&self) -> Result<OsString> {
        let bin = self.home.join("bin");
        let current = std::env::var_os("PATH").unwrap_or_default();
        let paths = std::iter::once(bin.clone())
            .chain(std::env::split_paths(&current).filter(|p| p != &bin));
        std::env::join_paths(paths).map_err(|e| invalid(format!("invalid SDK PATH: {e}")))
    }
}

/// Declarative project selection, intentionally independent of any build DSL.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    /// Java requirement or exact installed ID.
    java: String,
}

impl SdkManager {
    /// Selects an explicit target, shell JAVA_HOME, project requirement, global default, or system Java.
    /// No implicit downloading occurs. Project discovery is enabled only when `directory` is supplied.
    pub fn execution(
        &self,
        target: Option<&str>,
        directory: Option<&Path>,
    ) -> Result<SdkExecution> {
        let _lock = self.lock(false)?;
        let state = self.read()?;
        let selected = if let Some(target) = target {
            Some(self.resolve_in(&state, target)?.0)
        } else if let Some(home) = std::env::var_os("JAVA_HOME").filter(|v| !v.is_empty()) {
            let home = PathBuf::from(home).canonicalize()?;
            if !home.join("bin").join(java_name()).is_file() {
                return Err(invalid("JAVA_HOME has no Java executable"));
            }
            let lease = state
                .installations
                .iter()
                .find_map(|i| {
                    self.home(i)
                        .ok()
                        .filter(|p| p.canonicalize().ok().as_ref() == Some(&home))
                        .map(|_| i)
                })
                .map(|i| self.lease(&i.id))
                .transpose()?;
            return Ok(SdkExecution {
                home,
                _lease: lease,
            });
        } else if let Some(target) = directory.map(project_request).transpose()?.flatten() {
            Some(self.resolve_in(&state, &target)?.0)
        } else {
            self.default_in(&state)?
        };
        if let Some(installed) = selected {
            let home = self.home(&installed)?;
            if !home.join("bin").join(java_name()).is_file() {
                return Err(invalid("selected SDK home is missing"));
            }
            return Ok(SdkExecution {
                home,
                _lease: Some(self.lease(&installed.id)?),
            });
        }
        let runtime = janex_java::runtime::runtimes(&JavaOptions::default())?.remove(0);
        Ok(SdkExecution {
            home: runtime.home,
            _lease: None,
        })
    }

    /// Writes a project requirement after resolving an installed SDK. Existing unrelated files are not overwritten.
    pub fn use_project(&self, target: &str, directory: &Path, pin: bool) -> Result<PathBuf> {
        let state = self.read()?;
        let (installation, request) = self.resolve_in(&state, target)?;
        let target = if pin || target == installation.id {
            installation.id
        } else {
            request.target()
        };
        let path = directory.join(".janex-toolchains.toml");
        if path.exists() {
            let text = read_project(&path)?;
            let _: Project = toml::from_str(&text)
                .map_err(|e| invalid(format!("invalid existing project toolchains: {e}")))?;
        }
        super::state::publish(&path, format!("java = \"{target}\"\n").as_bytes())?;
        Ok(path)
    }
}

/// Reads only declarative toolchain requirements while ascending project directories.
fn project_request(directory: &Path) -> Result<Option<String>> {
    for directory in directory.ancestors() {
        let path = directory.join(".janex-toolchains.toml");
        if path.is_file() {
            let project: Project = toml::from_str(&read_project(&path)?)
                .map_err(|e| invalid(format!("invalid project toolchains: {e}")))?;
            return Ok(Some(project.java));
        }
        if directory.join(".git").exists() {
            break;
        }
    }
    Ok(None)
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
        if let Some(default) = manager.default_in(&state)? {
            managed.push(manager.home(&default)?.join("bin").join(java_name()));
        }
        let native = janex_platform::native_architecture().ok();
        let mut installed = state.installations.iter().collect::<Vec<_>>();
        installed.sort_by(|a, b| {
            (Some(&a.java.architecture) != native.as_ref())
                .cmp(&(Some(&b.java.architecture) != native.as_ref()))
                .then_with(|| super::version_order(&b.java.version, &a.java.version))
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
        match JavaRuntime::probe(&path) {
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
        && state.default.is_none()
        && let Ok(native) = janex_platform::native_architecture()
    {
        result.sort_by_key(|(runtime, _)| runtime.architecture != native);
    }
    Ok(result)
}

/// Exposes the project request for explicit environment installation.
pub fn project_java(directory: &Path) -> Result<Option<String>> {
    project_request(directory)
}

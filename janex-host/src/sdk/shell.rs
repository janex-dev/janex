// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Transactional shell environment rendering without mutating the Host environment.

use super::{
    SdkManager, Shell,
    selection::{home_variable, project_requests, validate_home},
};
use crate::{Result, error::invalid};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Environment variable carrying this shell's baseline and explicit selections.
const STATE: &str = "JANEX_SHELL_STATE";
/// Environment variables restored when integration is deactivated.
const VARIABLES: [&str; 4] = ["PATH", "JAVA_HOME", "GRADLE_HOME", "MAVEN_HOME"];

/// Serializable shell-local state; no global SDK registry mutation is required.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    /// Original environment, preserving the distinction between empty and absent values.
    original: BTreeMap<String, Option<String>>,
    /// Exact installed IDs explicitly chosen for this shell.
    selected: BTreeMap<String, String>,
    /// Paths prepended by the previous application.
    bins: Vec<PathBuf>,
    /// PATH most recently emitted by Janex.
    applied_path: Option<String>,
    /// User PATH before the latest Janex overlay.
    base_path: Option<String>,
}

impl SdkManager {
    /// Returns shell assignments after resolving all selections successfully. Targets replace
    /// only their own families; `reset` clears manual selections before reading the project.
    /// Selection order is explicit shell choice, project, global default, then the original home.
    /// No JVM or download is required. The caller must evaluate the complete successful result.
    pub fn shell_environment(
        &self,
        targets: &[String],
        directory: &Path,
        shell: Shell,
        reset: bool,
    ) -> Result<String> {
        let mut session = read_session()?.unwrap_or_else(|| Session {
            original: BTreeMap::new(),
            selected: BTreeMap::new(),
            bins: Vec::new(),
            applied_path: None,
            base_path: None,
        });
        if session.original.is_empty() {
            for name in VARIABLES {
                session.original.insert(name.into(), environment(name)?);
            }
            session.base_path = session.original["PATH"].clone();
        }
        if reset {
            session.selected.clear();
        }
        let _lock = self.lock(false)?;
        let registry = self.read()?;
        let mut changed = std::collections::BTreeSet::new();
        for target in targets {
            let installed = self.resolve_in(&registry, target)?.0;
            let family = installed.sdk.family();
            if !changed.insert(family) {
                return Err(invalid(
                    "only one shell selection per SDK family is allowed",
                ));
            }
            session.selected.insert(family.into(), installed.id);
        }
        let project = project_requests(directory)?;
        let mut homes = BTreeMap::new();
        let mut bins = Vec::new();
        let mut excluded = Vec::new();
        for family in ["java", "gradle", "maven"] {
            let selected = if let Some(target) =
                session.selected.get(family).or_else(|| project.get(family))
            {
                Some(self.resolve_in(&registry, target)?.0)
            } else {
                self.default_in(&registry, family)?
            };
            let home = if let Some(installed) = selected {
                if installed.sdk.family() != family {
                    return Err(invalid("shell SDK family mismatch"));
                }
                installed.sdk.check_host()?;
                let home = janex_java::runtime::java_path(&self.home(&installed)?);
                validate_home(&home, family)?;
                bins.push(home.join("bin"));
                for installed in registry
                    .installations
                    .iter()
                    .filter(|i| i.sdk.family() == family)
                {
                    excluded
                        .push(janex_java::runtime::java_path(&self.home(installed)?).join("bin"));
                }
                if let Some(original) = &session.original[home_variable(family)] {
                    excluded.push(janex_java::runtime::java_path(Path::new(original)).join("bin"));
                }
                Some(
                    home.to_str()
                        .ok_or_else(|| invalid("SDK home is not Unicode"))?
                        .to_owned(),
                )
            } else {
                session.original[home_variable(family)].clone()
            };
            homes.insert(home_variable(family), home);
        }
        let current_path = environment("PATH")?;
        if session.applied_path.is_some() && current_path != session.applied_path {
            session.base_path = current_path
                .as_ref()
                .map(|path| {
                    std::env::join_paths(
                        std::env::split_paths(path).filter(|p| !session.bins.contains(p)),
                    )
                    .map_err(|e| invalid(format!("invalid shell PATH: {e}")))?
                    .into_string()
                    .map_err(|_| invalid("PATH is not Unicode"))
                })
                .transpose()?;
        }
        let path = if bins.is_empty() {
            session.base_path.clone()
        } else {
            let base = session
                .base_path
                .as_ref()
                .map(|p| std::env::split_paths(p).collect::<Vec<_>>())
                .unwrap_or_default();
            Some(
                std::env::join_paths(
                    bins.iter()
                        .cloned()
                        .chain(base.into_iter().filter(|p| !excluded.contains(p))),
                )
                .map_err(|e| invalid(format!("invalid shell PATH: {e}")))?
                .into_string()
                .map_err(|_| invalid("PATH is not Unicode"))?,
            )
        };
        session.applied_path = path.clone();
        session.bins = bins;
        let mut output = String::new();
        for (name, value) in homes {
            output.push_str(&assignment(shell, name, value.as_deref()));
        }
        output.push_str(&assignment(shell, "PATH", path.as_deref()));
        let state = serde_json::to_string(&session).map_err(|e| invalid(e.to_string()))?;
        if state.len() > 24 * 1024 {
            return Err(invalid("shell integration state exceeds 24 KiB"));
        }
        output.push_str(&assignment(shell, STATE, Some(&state)));
        if matches!(shell, Shell::Fish) {
            output.push_str("true\n");
        }
        Ok(output)
    }

    /// Returns assignments restoring the activation-time PATH and SDK home variables.
    /// Subsequent manual edits to these variables are replaced by that original snapshot.
    pub fn shell_deactivate(&self, shell: Shell) -> Result<String> {
        let session =
            read_session()?.ok_or_else(|| invalid("Janex shell integration is not active"))?;
        let mut output = String::new();
        for name in VARIABLES {
            output.push_str(&assignment(shell, name, session.original[name].as_deref()));
        }
        output.push_str(&assignment(shell, STATE, None));
        if matches!(shell, Shell::Fish) {
            output.push_str("true\n");
        }
        Ok(output)
    }
}

/// Reads a Unicode environment value without confusing absence with empty content.
fn environment(name: &str) -> Result<Option<String>> {
    std::env::var_os(name)
        .map(|v| {
            v.into_string()
                .map_err(|_| invalid(format!("{name} is not Unicode")))
        })
        .transpose()
}

/// Validates inherited state before rendering any shell code.
fn read_session() -> Result<Option<Session>> {
    let Some(text) = environment(STATE)? else {
        return Ok(None);
    };
    if text.len() > 24 * 1024 {
        return Err(invalid("shell integration state exceeds 24 KiB"));
    }
    let session: Session = serde_json::from_str(&text)
        .map_err(|e| invalid(format!("invalid shell integration state: {e}")))?;
    if session.original.len() != VARIABLES.len()
        || VARIABLES
            .iter()
            .any(|key| !session.original.contains_key(*key))
        || session
            .selected
            .keys()
            .any(|k| !matches!(k.as_str(), "java" | "gradle" | "maven"))
        || session.bins.len() > 3
    {
        return Err(invalid("invalid shell integration snapshot"));
    }
    Ok(Some(session))
}

/// Quotes one literal value for a supported shell without interpreting its contents.
pub fn quote(shell: Shell, text: &str) -> String {
    match shell {
        Shell::Sh => format!("'{}'", text.replace('\'', "'\\''")),
        Shell::PowerShell => format!("'{}'", text.replace('\'', "''")),
        Shell::Fish => format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'")),
    }
}

/// Emits assignment or removal of a fixed, internally selected variable name.
fn assignment(shell: Shell, name: &str, value: Option<&str>) -> String {
    match (shell, value) {
        (Shell::Sh, Some(value)) => format!("export {name}={}\n", quote(shell, value)),
        (Shell::Sh, None) => format!("unset {name}\n"),
        (Shell::PowerShell, Some(value)) => format!("$env:{name} = {}\n", quote(shell, value)),
        (Shell::PowerShell, None) => {
            format!("Remove-Item Env:{name} -ErrorAction SilentlyContinue\n")
        }
        (Shell::Fish, Some(value)) if name == "PATH" => {
            let paths = std::env::split_paths(value)
                .map(|p| quote(shell, p.to_str().expect("Unicode PATH")))
                .collect::<Vec<_>>();
            format!("set -gx PATH {}\n", paths.join(" "))
        }
        (Shell::Fish, Some(value)) => format!("set -gx {name} {}\n", quote(shell, value)),
        (Shell::Fish, None) => format!("set -e {name}\n"),
    }
}

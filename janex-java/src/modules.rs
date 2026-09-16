// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Module inventory checks performed without starting a JVM or resolving a module graph.

use crate::{Result, error::invalid, runtime::JavaRuntime};
use std::collections::BTreeSet;

/// Selected module identity and mandatory descriptor dependencies.
#[derive(Clone, Debug)]
pub struct Module {
    /// Exact descriptor or automatic module name.
    pub name: String,
    /// Exact descriptor version, when present.
    pub version: Option<String>,
    /// Non-static requires names; compiled versions do not constrain runtime resolution.
    pub requires: Vec<String>,
}

/// Checks selected module identities, explicit version requirements, and mandatory dependencies.
///
/// Returns system-module roots needed by the selected descriptors and main module. This does not
/// resolve readability, service binding, package ownership, or access options; the application JVM
/// performs those checks while initializing its module layer, before invoking Java agent premain
/// methods or main.
pub fn system_roots(
    runtime: &JavaRuntime,
    modules: &[Module],
    requirements: &[(String, Option<String>)],
    main: Option<&str>,
) -> Result<BTreeSet<String>> {
    let mut available = runtime.modules.clone();
    for module in modules {
        if available
            .insert(module.name.clone(), module.version.clone())
            .is_some()
        {
            return Err(invalid(format!(
                "duplicate or shadowed module: {}",
                module.name
            )));
        }
    }
    for (name, version) in requirements {
        let actual = available
            .get(name)
            .ok_or_else(|| invalid(format!("required module is unavailable: {name}")))?;
        if let Some(version) = version
            && actual.as_ref() != Some(version)
        {
            return Err(invalid(format!(
                "required module version is unavailable: {name}@{version}"
            )));
        }
    }
    let mut roots = BTreeSet::new();
    for name in modules
        .iter()
        .flat_map(|module| &module.requires)
        .map(String::as_str)
        .chain(main)
    {
        if !available.contains_key(name) {
            return Err(invalid(format!("required module is unavailable: {name}")));
        }
        if runtime.modules.contains_key(name) {
            roots.insert(name.to_owned());
        }
    }
    Ok(roots)
}

/// Derives an automatic module identity from its original JAR filename and optional manifest name.
///
/// This extracts identity only. The JVM checks Java identifiers, packages, and service declarations.
pub fn automatic(file: &str, manifest_name: Option<&str>) -> Module {
    let stem = file.strip_suffix(".jar").unwrap_or(file);
    let bytes = stem.as_bytes();
    let boundary = (0..bytes.len()).find(|&i| {
        if bytes[i] != b'-' {
            return false;
        }
        let end = bytes[i + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
            + i
            + 1;
        end > i + 1 && (end == bytes.len() || bytes[end] == b'.')
    });
    let version = boundary
        .map(|i| &stem[i + 1..])
        .filter(|version| {
            let Some(first) = version.find(['-', '+']) else {
                return true;
            };
            if first + 1 == version.len() {
                return false;
            }
            version[first + 1..]
                .find('+')
                .is_none_or(|next| first + next + 2 < version.len())
        })
        .map(str::to_owned);
    let stem = boundary.map_or(stem, |i| &stem[..i]);
    let name = manifest_name.map(str::to_owned).unwrap_or_else(|| {
        stem.split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(".")
    });
    Module {
        name,
        version,
        requires: Vec::new(),
    }
}

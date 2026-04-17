// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::condition::platform::OperatingSystem;
use crate::error::Error;
use serde::Serialize;

/// Information about a candidate Java runtime environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Java {
    /// The parsed version of the Java runtime.
    pub version: JavaVersion,
    /// The vendor of the Java runtime.
    pub vendor: String,
    /// The operating system for which the Java runtime was built.
    pub os: OperatingSystem,
    /// The normalized CPU architecture name of the Java runtime.
    pub arch: String,
}

impl Java {
    /// Creates a Java runtime descriptor for CEL condition evaluation.
    pub fn new(
        version: JavaVersion,
        vendor: impl Into<String>,
        os: OperatingSystem,
        arch: impl Into<String>,
    ) -> Self {
        let arch = arch.into();
        Self {
            version,
            vendor: vendor.into(),
            os,
            arch: normalize_arch_name(&arch).to_string(),
        }
    }
}

/// The parsed version of a Java runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JavaVersion {
    /// The full, unparsed version string.
    pub full: String,
    /// The feature release number.
    pub feature: u64,
    /// The interim release number.
    pub interim: u64,
    /// The update release number.
    pub update: u64,
    /// The patch release number.
    pub patch: u64,
    /// The optional pre-release identifier.
    pub pre: String,
    /// The build number.
    pub build: u64,
    /// Optional additional build metadata.
    pub optional: String,
}

impl JavaVersion {
    /// Parses a Java version string using the component model described by `FileFormat.md`.
    pub fn parse(full: impl Into<String>) -> Result<Self, Error> {
        let full = full.into();
        let trimmed = full.trim().to_string();
        if trimmed.is_empty() {
            return Err(Error::InvalidValueMessage(
                "Java version string must not be empty".to_string(),
            ));
        }

        let (main, build_and_optional) = match trimmed.as_str().split_once('+') {
            Some((main, build_and_optional)) => (main, Some(build_and_optional)),
            None => (trimmed.as_str(), None),
        };
        let (version_part, pre) = match main.split_once('-') {
            Some((version_part, pre)) => (version_part, pre),
            None => (main, ""),
        };

        let mut numbers = [0u64; 4];
        let components: Vec<&str> = version_part.split('.').collect();
        if components.is_empty() || components.len() > 4 {
            return Err(Error::InvalidValueMessage(format!(
                "invalid Java version '{trimmed}': expected 1 to 4 numeric components"
            )));
        }

        for (index, component) in components.iter().enumerate() {
            if component.is_empty() {
                return Err(Error::InvalidValueMessage(format!(
                    "invalid Java version '{trimmed}': empty numeric component"
                )));
            }
            numbers[index] = component.parse::<u64>().map_err(|_| {
                Error::InvalidValueMessage(format!(
                    "invalid Java version '{trimmed}': non-numeric component '{component}'"
                ))
            })?;
        }

        let (build, optional) = match build_and_optional {
            Some(value) => {
                let (build, optional) = match value.split_once('-') {
                    Some((build, optional)) => (build, optional),
                    None => (value, ""),
                };
                if build.is_empty() {
                    return Err(Error::InvalidValueMessage(format!(
                        "invalid Java version '{trimmed}': empty build number"
                    )));
                }
                let build = build.parse::<u64>().map_err(|_| {
                    Error::InvalidValueMessage(format!(
                        "invalid Java version '{trimmed}': invalid build number '{build}'"
                    ))
                })?;
                (build, optional.to_string())
            }
            None => (0, String::new()),
        };

        Ok(Self {
            full,
            feature: numbers[0],
            interim: numbers[1],
            update: numbers[2],
            patch: numbers[3],
            pre: pre.to_string(),
            build,
            optional,
        })
    }
}

/// Normalizes CPU architecture spellings to the names described by `FileFormat.md`.
pub(crate) fn normalize_arch_name(arch: &str) -> &str {
    match arch {
        "x86_64" => "x86-64",
        "i386" | "i486" | "i586" | "i686" | "x86" => "x86",
        other => other,
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Native, persistent SDK installation and version selection.

mod archive;
mod catalog;
mod request;
mod selection;
mod state;
mod tools;

pub use catalog::{AvailableSdk, CatalogOptions};
pub use request::SdkRequest;
pub(crate) use selection::application_runtimes;
pub use selection::project_java;
pub use selection::{SdkExecution, Shell};
pub use state::{Installation, SdkManager, SdkStatus, Selection};

use crate::{Result, error::invalid};
use serde::{Deserialize, Serialize};

/// A Java distribution requirement and its platform variant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JavaRequest {
    /// Disco distribution identifier, with `bellsoft` normalized to `liberica`.
    pub vendor: String,
    /// Numeric feature, release, complete release plus build number, or `latest`.
    pub version: String,
    /// Normalized JVM architecture; independent of the Janex process architecture.
    pub architecture: String,
    /// `jdk` or `jre`.
    pub kind: String,
    /// Whether the package includes JavaFX.
    pub javafx: bool,
    /// Linux `glibc` or `musl`, or `c_std_lib` on other platforms.
    pub libc: String,
}

impl JavaRequest {
    /// Parses `java:<vendor>@<version>` or a recognized shorthand vendor request.
    /// Defaults to a JDK for the native operating-system architecture and libc.
    pub fn parse(text: &str) -> Result<Self> {
        let explicit = text.starts_with("java:");
        let text = text.strip_prefix("java:").unwrap_or(text);
        let (vendor, version) = text.split_once('@').ok_or_else(|| {
            invalid("expected java:<vendor>@<version>, for example java:bellsoft@21")
        })?;
        if !explicit
            && !matches!(
                vendor,
                "bellsoft"
                    | "liberica"
                    | "temurin"
                    | "zulu"
                    | "corretto"
                    | "microsoft"
                    | "oracle"
                    | "semeru"
                    | "sap_machine"
                    | "dragonwell"
                    | "graalvm_community"
            )
        {
            return Err(invalid(
                "unknown target; use java:<vendor>@<version> for a Java SDK",
            ));
        }
        let request = Self {
            vendor: if vendor == "bellsoft" {
                "liberica"
            } else {
                vendor
            }
            .into(),
            version: version.into(),
            architecture: janex_platform::native_architecture()?,
            kind: "jdk".into(),
            javafx: false,
            libc: if cfg!(target_os = "linux") {
                if std::path::Path::new("/etc/alpine-release").exists() {
                    "musl"
                } else {
                    "glibc"
                }
            } else {
                "c_std_lib"
            }
            .into(),
        };
        request.validate()?;
        Ok(request)
    }

    /// Returns a stable, human-readable requirement without platform options.
    pub fn target(&self) -> String {
        format!(
            "java:{}@{}",
            if self.vendor == "liberica" {
                "bellsoft"
            } else {
                &self.vendor
            },
            self.version
        )
    }

    /// Validates persisted or caller-constructed requirements before using them.
    pub fn validate(&self) -> Result<()> {
        if self.vendor.is_empty()
            || self.vendor.len() > 64
            || !self
                .vendor
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c == b'_')
        {
            return Err(invalid("invalid Java distribution identifier"));
        }
        if self.version != "latest" {
            numeric_version(&self.version)?;
        }
        if !matches!(
            self.architecture.as_str(),
            "x86" | "x86-64" | "aarch64" | "arm" | "ppc64le" | "s390x" | "riscv64"
        ) || !matches!(self.kind.as_str(), "jdk" | "jre")
            || !matches!(self.libc.as_str(), "glibc" | "musl" | "c_std_lib")
        {
            return Err(invalid(
                "unsupported SDK architecture, package kind, or libc",
            ));
        }
        Ok(())
    }

    /// Tests a GA numeric version. A feature request matches its series; other releases are exact.
    pub fn matches(&self, version: &str) -> bool {
        if self.version == "latest" {
            return numeric_version(version).is_ok();
        }
        let Ok((wanted, build)) = numeric_version(&self.version) else {
            return false;
        };
        let Ok((actual, actual_build)) = numeric_version(version) else {
            return false;
        };
        if wanted.len() == 1 && build.is_none() {
            return actual.first() == wanted.first();
        }
        let size = wanted.len().max(actual.len());
        (0..size).all(|i| wanted.get(i).unwrap_or(&0) == actual.get(i).unwrap_or(&0))
            && build.is_none_or(|b| actual_build == Some(b))
    }
}

/// Parses normalized GA numeric releases with an optional build number.
pub(crate) fn numeric_version(text: &str) -> Result<(Vec<u32>, Option<u32>)> {
    if text.is_empty() || text.len() > 80 {
        return Err(invalid("invalid Java version"));
    }
    let (release, build) = text
        .split_once('+')
        .map_or((text, None), |(a, b)| (a, Some(b)));
    let number = |s: &str| -> Result<u32> {
        if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
            return Err(invalid("expected a numeric GA Java version"));
        }
        s.parse()
            .map_err(|_| invalid("Java version component exceeds u32"))
    };
    let parts: Vec<_> = release.split('.').map(number).collect::<Result<_>>()?;
    if parts.len() > 5 || parts[0] < 8 {
        return Err(invalid("expected a Java 8-or-later numeric version"));
    }
    Ok((parts, build.map(number).transpose()?))
}

/// Orders GA versions numerically, including build numbers.
pub(crate) fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| {
        let (mut parts, build) = numeric_version(s).expect("validated Java version");
        parts.resize(5, 0);
        (parts, build.unwrap_or(0))
    };
    key(a).cmp(&key(b))
}

/// Returns the current operating system's Disco name.
pub(crate) fn operating_system() -> Result<&'static str> {
    match std::env::consts::OS {
        "windows" => Ok("windows"),
        "linux" => Ok("linux"),
        "macos" => Ok("macos"),
        "freebsd" => Ok("free_bsd"),
        _ => Err(invalid(
            "SDK downloads are unsupported on this operating system",
        )),
    }
}

/// Returns the Java executable name on this operating system.
pub(crate) fn java_name() -> &'static str {
    if cfg!(windows) { "java.exe" } else { "java" }
}

/// Encodes a digest as lowercase hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_requirements_and_build_order() {
        let mut request = JavaRequest::parse("bellsoft@21").unwrap();
        assert!(request.matches("21.0.8+12"));
        assert!(!request.matches("22+1"));
        request.version = "21.0.8".into();
        assert!(request.matches("21.0.8+12"));
        assert!(!request.matches("21.0.8.1+1"));
        assert!(!request.matches("21.0.9+1"));
        request.version = "21.0.8+12".into();
        assert!(!request.matches("21.0.8+13"));
        assert!(version_order("21.0.10+1", "21.0.9+30").is_gt());
        assert!(version_order("21.0.8+12", "21.0.8+9").is_gt());
        assert!(
            JavaRequest::parse("bellsoft@latest")
                .unwrap()
                .matches("25.0.1+8")
        );
        for target in [
            "bellsoft@../21",
            "bellsoft@21-ea",
            "bellsoft@21+",
            "java:../bad@21",
            "example@21",
        ] {
            assert!(JavaRequest::parse(target).is_err(), "{target}");
        }
    }
}

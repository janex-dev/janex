// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Provider-independent SDK requirements.

use super::JavaRequest;
use crate::{Result, error::invalid};
use serde::{Deserialize, Serialize};

/// An SDK requirement with provider-specific version and platform semantics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "request", rename_all = "lowercase")]
pub enum SdkRequest {
    /// A Java distribution and platform variant.
    Java(JavaRequest),
    /// A portable Gradle binary distribution.
    Gradle(String),
    /// A portable Apache Maven binary distribution.
    Maven(String),
}

impl SdkRequest {
    /// Parses Java targets, `gradle@<version>`, or `maven@<version>`.
    pub fn parse(text: &str) -> Result<Self> {
        let request = if let Some(version) = text.strip_prefix("gradle@") {
            Self::Gradle(version.into())
        } else if let Some(version) = text.strip_prefix("maven@") {
            Self::Maven(version.into())
        } else {
            Self::Java(JavaRequest::parse(text)?)
        };
        request.validate()?;
        Ok(request)
    }

    /// Returns the SDK family used for storage and independent defaults.
    pub fn family(&self) -> &'static str {
        match self {
            Self::Java(_) => "java",
            Self::Gradle(_) => "gradle",
            Self::Maven(_) => "maven",
        }
    }

    /// Returns the requested version or `latest`.
    pub fn version(&self) -> &str {
        match self {
            Self::Java(java) => &java.version,
            Self::Gradle(version) | Self::Maven(version) => version,
        }
    }

    /// Replaces the version while retaining provider and platform selection.
    pub(super) fn with_version(&self, version: &str) -> Self {
        let mut exact = self.clone();
        match &mut exact {
            Self::Java(java) => java.version = version.into(),
            Self::Gradle(value) | Self::Maven(value) => *value = version.into(),
        }
        exact
    }

    /// Returns a human-readable request, excluding Java platform overrides.
    pub fn target(&self) -> String {
        match self {
            Self::Java(java) => java.target(),
            _ => format!("{}@{}", self.family(), self.version()),
        }
    }

    /// Returns Java-specific information when this is a Java requirement.
    pub fn java(&self) -> Option<&JavaRequest> {
        match self {
            Self::Java(java) => Some(java),
            _ => None,
        }
    }

    /// Validates a requirement received from callers or persistent metadata.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Java(java) => java.validate(),
            _ if self.version() == "latest" => Ok(()),
            _ => tool_version(self.version()).map(|_| ()),
        }
    }

    /// Tests both provider variant and version constraints against a concrete installation.
    pub(super) fn accepts(&self, actual: &Self) -> bool {
        match (self, actual) {
            (Self::Java(wanted), Self::Java(actual)) => {
                let mut variant = actual.clone();
                variant.version.clone_from(&wanted.version);
                variant == *wanted && wanted.matches(&actual.version)
            }
            _ => self.family() == actual.family() && self.matches(actual.version()),
        }
    }

    /// Matches stable versions. Tool requirements with one or two components select a series;
    /// three components select an exact release. Java retains its own release rules.
    pub fn matches(&self, version: &str) -> bool {
        if let Self::Java(java) = self {
            return java.matches(version);
        }
        let Ok(actual) = tool_version(version) else {
            return false;
        };
        if self.version() == "latest" {
            return true;
        }
        let Ok(wanted) = tool_version(self.version()) else {
            return false;
        };
        if wanted.len() < 3 {
            actual.starts_with(&wanted)
        } else {
            wanted == actual
        }
    }

    /// Compares validated concrete versions belonging to the same SDK family.
    pub(super) fn compare(&self, other: &Self) -> std::cmp::Ordering {
        match self {
            Self::Java(_) => super::version_order(self.version(), other.version()),
            _ => tool_version(self.version())
                .unwrap()
                .cmp(&tool_version(other.version()).unwrap()),
        }
    }

    /// Returns the platform entry point whose presence is required for an installed SDK.
    pub(super) fn executable(&self) -> &'static str {
        match self {
            Self::Java(_) => super::java_name(),
            Self::Gradle(_) if cfg!(windows) => "gradle.bat",
            Self::Gradle(_) => "gradle",
            Self::Maven(_) if cfg!(windows) => "mvn.cmd",
            Self::Maven(_) => "mvn",
        }
    }
}

/// Parses one to three canonical numeric release components, excluding preview versions.
pub(super) fn tool_version(text: &str) -> Result<Vec<u32>> {
    if text.is_empty() || text.len() > 40 {
        return Err(invalid("invalid SDK version"));
    }
    let parts = text
        .split('.')
        .map(|part| {
            if part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(invalid("expected a numeric stable SDK version"));
            }
            part.parse::<u32>()
                .map_err(|_| invalid("SDK version component exceeds u32"))
        })
        .collect::<Result<Vec<_>>>()?;
    if parts.len() > 3 || parts[0] == 0 {
        return Err(invalid("invalid SDK version"));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_series_and_exact_releases_are_distinct() {
        assert!(SdkRequest::parse("gradle@8").unwrap().matches("8.14.3"));
        assert!(SdkRequest::parse("maven@3.9").unwrap().matches("3.9.9"));
        assert!(!SdkRequest::parse("maven@3.9.9").unwrap().matches("3.9.10"));
        assert!(
            !SdkRequest::parse("maven@latest")
                .unwrap()
                .matches("4.0.0-rc-1")
        );
        assert!(
            !SdkRequest::parse("gradle@8")
                .unwrap()
                .accepts(&SdkRequest::parse("maven@8").unwrap())
        );
        assert!(SdkRequest::parse("gradle@../8").is_err());
    }
}

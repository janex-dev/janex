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
    /// Parses a product selector with optional `[variant=...,os=...,arch=...,libc=...]` qualifiers.
    /// Missing versions mean `latest`; missing platform components use the native platform.
    pub fn parse(text: &str) -> Result<Self> {
        let (base, qualifiers) = split_selector(text)?;
        let (id, version) = base.split_once('@').unwrap_or((base, "latest"));
        let product = super::product::product(id)?;
        let mut request = match product.family {
            "gradle" => Self::Gradle(version.into()),
            "maven" => Self::Maven(version.into()),
            _ => Self::Java(JavaRequest::parse_base(base)?),
        };
        if let Self::Java(java) = &mut request {
            if let Some(os) = qualifiers.get("os") {
                java.platform.os = os.clone();
                java.platform.libc = super::SdkPlatform::default_libc(os).into();
            }
            if let Some(arch) = qualifiers.get("arch") {
                java.platform.arch = normalize_arch(arch).into();
            }
            if let Some(libc) = qualifiers.get("libc") {
                java.platform.libc = libc.clone();
            }
            if let Some(variant) = qualifiers.get("variant") {
                java.variant = variant.clone();
            }
        } else if qualifiers
            .iter()
            .any(|(key, value)| key != "variant" || value != "standard")
        {
            return Err(invalid(
                "platform and variant qualifiers do not apply to portable SDKs",
            ));
        }
        request.validate()?;
        Ok(request)
    }

    /// Merges explicit qualifiers into a selector without freezing omitted native platform values.
    /// Installation IDs cannot be combined with qualifiers.
    pub fn qualify(text: &str, values: &[(&str, Option<&str>)]) -> Result<String> {
        let (base, mut qualifiers) = split_selector(text)?;
        for (key, value) in values {
            if let Some(value) = value {
                qualifiers.insert((*key).into(), (*value).into());
            }
        }
        if qualifiers.is_empty() {
            return Ok(text.into());
        }
        let result = format!(
            "{}[{}]",
            base,
            qualifiers
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        Self::parse(&result)?;
        Ok(result)
    }

    /// Returns the default slot for this family and target, or its portable family.
    pub fn default_key(&self) -> String {
        match self {
            Self::Java(java) => format!("java/{}", java.platform.key()),
            _ => self.family().into(),
        }
    }

    /// Returns the canonical product identity.
    pub fn product(&self) -> &str {
        match self {
            Self::Java(java) => &java.product,
            Self::Gradle(_) => "gradle/gradle",
            Self::Maven(_) => "apache/maven",
        }
    }

    /// Rejects activation of foreign-OS SDKs; CPU emulation remains the operating system's responsibility.
    pub(super) fn check_host(&self) -> Result<()> {
        if let Self::Java(java) = self
            && java.platform.os != std::env::consts::OS
        {
            return Err(invalid(
                "cannot activate an SDK for another operating system",
            ));
        }
        Ok(())
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

    /// Returns a lossless selector including Java product, variant and platform qualifiers.
    pub fn target(&self) -> String {
        match self {
            Self::Java(java) => java.target(),
            _ => format!("{}@{}", self.product(), self.version()),
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
    pub(super) fn executable(&self) -> String {
        match self {
            Self::Java(java) => java.platform.executable("java"),
            Self::Gradle(_) if cfg!(windows) => "gradle.bat".into(),
            Self::Gradle(_) => "gradle".into(),
            Self::Maven(_) if cfg!(windows) => "mvn.cmd".into(),
            Self::Maven(_) => "mvn".into(),
        }
    }
}

/// Splits a selector and rejects duplicate or unknown qualifier keys.
fn split_selector(text: &str) -> Result<(&str, std::collections::BTreeMap<String, String>)> {
    let mut values = std::collections::BTreeMap::new();
    let Some((base, rest)) = text.split_once('[') else {
        return Ok((text, values));
    };
    let rest = rest
        .strip_suffix(']')
        .ok_or_else(|| invalid("unterminated SDK qualifiers"))?;
    for item in rest.split(',') {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| invalid("expected SDK qualifier key=value"))?;
        if !matches!(key, "variant" | "os" | "arch" | "libc")
            || value.is_empty()
            || values.insert(key.into(), value.into()).is_some()
        {
            return Err(invalid("invalid or duplicate SDK qualifier"));
        }
    }
    Ok((base, values))
}

/// Normalizes common CPU aliases in user selections.
fn normalize_arch(arch: &str) -> &str {
    match arch {
        "amd64" | "x64" | "x86_64" => "x86-64",
        "arm64" => "aarch64",
        other => other,
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
    #[test]
    fn products_variants_and_platforms_have_lossless_selectors() {
        let full =
            SdkRequest::parse("bellsoft/liberica-jdk@21[variant=full,arch=arm64,os=windows]")
                .unwrap();
        assert_eq!(SdkRequest::parse(&full.target()).unwrap(), full);
        assert_eq!(full.java().unwrap().platform.arch, "aarch64");
        let nik = SdkRequest::parse("bellsoft/liberica-nik@21[os=windows,arch=aarch64]").unwrap();
        assert!(!full.accepts(&nik));
        assert_eq!(
            SdkRequest::parse("bellsoft/liberica-jdk")
                .unwrap()
                .version(),
            "latest"
        );
        for invalid in [
            "bellsoft@21",
            "java:bellsoft@21",
            "bellsoft/python@3",
            "adoptium/temurin-jdk@21[variant=full]",
            "bellsoft/liberica-jdk@21[arch=x86,arch=aarch64]",
            "bellsoft/liberica-jdk@21[os=windows,libc=musl]",
        ] {
            assert!(SdkRequest::parse(invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            SdkRequest::qualify("bellsoft/liberica-jdk@21", &[("variant", Some("full"))]).unwrap(),
            "bellsoft/liberica-jdk@21[variant=full]"
        );
    }
}

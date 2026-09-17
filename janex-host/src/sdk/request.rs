// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Provider-independent SDK requirements.

use super::{Product, SdkPlatform};
use crate::{Result, error::invalid};
use serde::{Deserialize, Serialize};

/// A product version and variant, optionally bound to a target platform.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdkRequest {
    /// Canonical publisher/product identity.
    pub product: String,
    /// Product-specific version requirement, or `latest`.
    pub version: String,
    /// Product-defined variant, including the product's default when omitted by the user.
    pub variant: String,
    /// Target platform; `None` denotes a platform-independent distribution.
    pub platform: Option<SdkPlatform>,
}

impl SdkRequest {
    /// Parses a product selector with optional `[variant=...,os=...,arch=...,libc=...]` qualifiers.
    /// Missing versions mean `latest`; the product defines its default variant and platform support.
    pub fn parse(text: &str) -> Result<Self> {
        let (base, qualifiers) = split_selector(text)?;
        let (id, version) = base.split_once('@').unwrap_or((base, "latest"));
        let product = super::product::product(id)?;
        let mut platform = if product.platform_specific {
            Some(SdkPlatform::native()?)
        } else {
            None
        };
        if let Some(platform) = &mut platform {
            if let Some(os) = qualifiers.get("os") {
                platform.os = os.clone();
                platform.libc = SdkPlatform::default_libc(os).into();
            }
            if let Some(arch) = qualifiers.get("arch") {
                platform.arch = normalize_arch(arch).into();
            }
            if let Some(libc) = qualifiers.get("libc") {
                platform.libc = libc.clone();
            }
        } else if qualifiers.keys().any(|key| key != "variant") {
            return Err(invalid("platform qualifiers do not apply to this product"));
        }
        let request = Self {
            product: product.id.into(),
            version: version.into(),
            variant: qualifiers
                .get("variant")
                .cloned()
                .unwrap_or_else(|| product.variants[0].into()),
            platform,
        };
        request.validate()?;
        Ok(request)
    }

    /// Returns the descriptor for this product, or an error for an unknown identity.
    pub(super) fn descriptor(&self) -> Result<&'static Product> {
        super::product::product(&self.product)
    }

    /// Returns the default slot for a validated request's family and optional target platform.
    pub fn default_key(&self) -> String {
        match &self.platform {
            Some(platform) => format!("{}/{}", self.family(), platform.key()),
            None => self.family().into(),
        }
    }

    /// Returns the product identity stored in this request.
    pub fn product(&self) -> &str {
        &self.product
    }

    /// Rejects foreign-OS activation; CPU emulation remains the operating system's responsibility.
    pub(super) fn check_host(&self) -> Result<()> {
        if let Some(platform) = &self.platform
            && platform.os != std::env::consts::OS
        {
            return Err(invalid(
                "cannot activate an SDK for another operating system",
            ));
        }
        Ok(())
    }

    /// Returns the tool family of a validated product request.
    pub fn family(&self) -> &'static str {
        self.descriptor().expect("validated SDK product").family
    }

    /// Returns the requested version or `latest`.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Replaces the version while retaining product, variant and platform selection.
    pub(super) fn with_version(&self, version: &str) -> Self {
        let mut exact = self.clone();
        exact.version = version.into();
        exact
    }

    /// Returns a lossless selector including the product variant and any target platform.
    pub fn target(&self) -> String {
        let mut result = format!("{}@{}[variant={}", self.product, self.version, self.variant);
        if let Some(platform) = &self.platform {
            result.push_str(&format!(
                ",os={},arch={},libc={}",
                platform.os, platform.arch, platform.libc
            ));
        }
        result.push(']');
        result
    }

    /// Returns this validated request if the product belongs to the Java tool family.
    pub fn java(&self) -> Option<&Self> {
        (self.family() == "java").then_some(self)
    }

    /// Validates canonical product identity, its variant, version and platform applicability.
    pub fn validate(&self) -> Result<()> {
        let product = self.descriptor()?;
        if product.id != self.product || !product.variants.contains(&self.variant.as_str()) {
            return Err(invalid("unsupported variant or noncanonical SDK product"));
        }
        match (&self.platform, product.platform_specific) {
            (Some(platform), true) => platform.validate()?,
            (None, false) => {}
            _ => {
                return Err(invalid(
                    "SDK platform does not match the product's platform support",
                ));
            }
        }
        if self.version != "latest" {
            if product.family == "java" {
                super::numeric_version(&self.version)?;
            } else {
                tool_version(&self.version)?;
            }
        }
        Ok(())
    }

    /// Tests product, variant, platform and version constraints against an installation.
    pub(super) fn accepts(&self, actual: &Self) -> bool {
        self.product == actual.product
            && self.variant == actual.variant
            && self.platform == actual.platform
            && self.matches(&actual.version)
    }

    /// Matches stable versions using the selected product family's version rules.
    pub fn matches(&self, version: &str) -> bool {
        if self.family() == "java" {
            return super::java_matches(&self.version, version);
        }
        let Ok(actual) = tool_version(version) else {
            return false;
        };
        if self.version == "latest" {
            return true;
        }
        let Ok(wanted) = tool_version(&self.version) else {
            return false;
        };
        if wanted.len() < 3 {
            actual.starts_with(&wanted)
        } else {
            actual == wanted
        }
    }

    /// Compares validated concrete versions belonging to the same SDK family.
    pub(super) fn compare(&self, other: &Self) -> std::cmp::Ordering {
        if self.family() == "java" {
            super::version_order(&self.version, &other.version)
        } else {
            tool_version(&self.version)
                .unwrap()
                .cmp(&tool_version(&other.version).unwrap())
        }
    }

    /// Returns the platform entry point required for an installed SDK of this family.
    pub(super) fn executable(&self) -> String {
        match self.family() {
            "java" => self
                .platform
                .as_ref()
                .expect("Java target platform")
                .executable("java"),
            "gradle" if cfg!(windows) => "gradle.bat".into(),
            "gradle" => "gradle".into(),
            "maven" if cfg!(windows) => "mvn.cmd".into(),
            "maven" => "mvn".into(),
            _ => unreachable!("supported SDK family"),
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
        assert_eq!(full.platform.as_ref().unwrap().arch, "aarch64");
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
    }
    #[test]
    fn portable_variants_are_part_of_the_common_product_request() {
        let bin = SdkRequest::parse("gradle@9").unwrap();
        let all = SdkRequest::parse("gradle@9[variant=all]").unwrap();
        assert_eq!(bin.variant, "bin");
        assert_eq!(all.variant, "all");
        assert!(bin.platform.is_none() && all.platform.is_none());
        assert_eq!(bin.default_key(), all.default_key());
        assert!(!bin.accepts(&all.with_version("9.1.0")));
        assert!(all.accepts(&all.with_version("9.1.0")));
        assert_eq!(all.with_version("9.1.0").variant, "all");
        assert_eq!(SdkRequest::parse(&all.target()).unwrap(), all);
        assert_eq!(SdkRequest::parse("maven@3.9").unwrap().variant, "standard");
        for invalid in [
            "gradle@9[variant=standard]",
            "gradle@9[variant=full]",
            "gradle@9[arch=aarch64]",
            "maven@3.9[variant=all]",
        ] {
            assert!(SdkRequest::parse(invalid).is_err(), "{invalid}");
        }
        let value = serde_json::to_value(&all).unwrap();
        assert_eq!(value["variant"], "all");
        assert!(value["platform"].is_null());
        assert_eq!(serde_json::from_value::<SdkRequest>(value).unwrap(), all);
    }
}

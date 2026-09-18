// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! GitHub release selection bound to the native host platform and asset digest.

use super::MAX_BYTES;
use crate::{
    Result,
    dependency::{DependencyOptions, download},
    error::invalid,
};
use janex_format::checksum::{Algorithm, Checksum};
use semver::Version;
use serde::Deserialize;
use url::Url;

/// One matching official release asset.
pub(super) struct Release {
    /// Version encoded by the release tag.
    pub version: Version,
    /// Platform distribution filename.
    pub name: String,
    /// HTTPS URL supplied by GitHub.
    url: Url,
    /// Required GitHub asset SHA-256.
    checksum: Checksum,
}

/// Fields used from the GitHub Releases response.
#[derive(Deserialize)]
struct Metadata {
    /// Published tag.
    tag_name: String,
    /// Whether the release is unpublished.
    draft: bool,
    /// Whether GitHub treats this as a preview.
    prerelease: bool,
    /// Uploaded distribution assets.
    assets: Vec<Asset>,
}

/// GitHub asset identity and integrity metadata.
#[derive(Deserialize)]
struct Asset {
    /// Exact uploaded filename.
    name: String,
    /// Direct download link.
    browser_download_url: String,
    /// Digest with an algorithm prefix; older assets can omit it.
    digest: Option<String>,
    /// Uploaded byte count.
    size: u64,
}

/// Maps native architectures to the public distribution names, independently of process emulation.
pub(super) fn platform(os: &str, arch: &str) -> Result<String> {
    let arch = match janex_platform::normalize_architecture(arch) {
        "x86-64" => "x86_64",
        "aarch64" => "arm64",
        "x86" if os == "windows" => "x86",
        _ => return Err(invalid("no Janex distribution for this architecture")),
    };
    if !matches!(os, "windows" | "linux" | "macos" | "freebsd") {
        return Err(invalid("no Janex distribution for this operating system"));
    }
    Ok(format!("{os}-{arch}"))
}

/// Fetches uncached release metadata, requiring an exact tag when a version was requested.
pub(super) fn resolve(version: Option<&str>, platform: &str) -> Result<Release> {
    let requested = version
        .map(|v| Version::parse(v.strip_prefix('v').unwrap_or(v)))
        .transpose()
        .map_err(|e| invalid(format!("invalid release version: {e}")))?;
    let endpoint = requested
        .as_ref()
        .map_or_else(|| "latest".into(), |v| format!("tags/v{v}"));
    let url = Url::parse(&format!(
        "https://api.github.com/repos/janex-dev/janex/releases/{endpoint}"
    ))
    .unwrap();
    let bytes = download(
        url,
        &DependencyOptions {
            max_bytes: 4 * 1024 * 1024,
            ..Default::default()
        },
    )
    .map_err(|e| invalid(format!("cannot fetch a published Janex release: {e}")))?;
    select(&bytes, requested.as_ref(), platform)
}

/// Validates the release identity and requires one complete asset for the selected platform.
fn select(bytes: &[u8], requested: Option<&Version>, platform: &str) -> Result<Release> {
    let metadata: Metadata = serde_json::from_slice(bytes)
        .map_err(|e| invalid(format!("invalid Janex release metadata: {e}")))?;
    let version = metadata
        .tag_name
        .strip_prefix('v')
        .and_then(|v| Version::parse(v).ok())
        .ok_or_else(|| invalid("Janex release tag must be v<version>"))?;
    if metadata.draft
        || requested.is_some_and(|v| *v != version)
        || requested.is_none() && (metadata.prerelease || !version.pre.is_empty())
    {
        return Err(invalid(
            "Janex release does not match the requested version",
        ));
    }
    let extension = if platform.starts_with("windows-") {
        "zip"
    } else {
        "tar.xz"
    };
    let name = format!("janex-{platform}.{extension}");
    let mut matches = metadata.assets.into_iter().filter(|a| a.name == name);
    let asset = matches.next().ok_or_else(|| {
        invalid(format!(
            "release has no {name}; its platform build may still be pending"
        ))
    })?;
    if matches.next().is_some() || asset.size == 0 || asset.size > MAX_BYTES {
        return Err(invalid("invalid Janex distribution asset"));
    }
    let checksum = asset
        .digest
        .as_deref()
        .and_then(|s| s.strip_prefix("sha256:"))
        .ok_or_else(|| invalid("GitHub release asset has no SHA-256 digest"))?;
    let url = Url::parse(&asset.browser_download_url)
        .map_err(|_| invalid("invalid release asset URL"))?;
    if url.as_str()
        != format!(
            "https://github.com/janex-dev/janex/releases/download/{}/{name}",
            metadata.tag_name
        )
    {
        return Err(invalid("release asset URL does not match its identity"));
    }
    Ok(Release {
        version,
        name,
        url,
        checksum: self::checksum(checksum)?,
    })
}

impl Release {
    /// Downloads and verifies the complete archive before any extraction or execution.
    pub fn download(&self) -> Result<Vec<u8>> {
        let bytes = download(
            self.url.clone(),
            &DependencyOptions {
                max_bytes: MAX_BYTES,
                ..Default::default()
            },
        )?;
        self.checksum.verify(bytes.as_slice())?;
        Ok(bytes)
    }
}

/// Parses a full SHA-256 hexadecimal digest.
pub(super) fn checksum(text: &str) -> Result<Checksum> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("expected a 64-digit SHA-256 digest"));
    }
    let mut encoded = vec![Algorithm::Sha256 as u8];
    for pair in text.as_bytes().as_chunks::<2>().0 {
        encoded.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
    }
    Ok(Checksum::decode(&encoded)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_platforms_match_distribution_names() {
        for (os, arch, expected) in [
            ("windows", "x86", "windows-x86"),
            ("windows", "AMD64", "windows-x86_64"),
            ("windows", "ARM64", "windows-arm64"),
            ("linux", "x86_64", "linux-x86_64"),
            ("linux", "aarch64", "linux-arm64"),
            ("freebsd", "aarch64", "freebsd-arm64"),
            ("freebsd", "amd64", "freebsd-x86_64"),
            ("macos", "arm64", "macos-arm64"),
            ("macos", "x86_64", "macos-x86_64"),
        ] {
            assert_eq!(platform(os, arch).unwrap(), expected);
        }
        assert!(platform("linux", "x86").is_err());
    }

    #[test]
    fn release_selection_binds_versions_platforms_urls_and_digests() {
        let mut value = serde_json::json!({"tag_name":"v1.2.3", "draft":false, "prerelease":false, "assets":[{
            "name":"janex-windows-arm64.zip", "size":123,
            "browser_download_url":"https://github.com/janex-dev/janex/releases/download/v1.2.3/janex-windows-arm64.zip",
            "digest":format!("sha256:{}", "ab".repeat(32))
        }]});
        let parse = |value: &serde_json::Value, version: Option<Version>| {
            select(
                &serde_json::to_vec(value).unwrap(),
                version.as_ref(),
                "windows-arm64",
            )
        };
        assert_eq!(parse(&value, None).unwrap().version.to_string(), "1.2.3");
        assert!(parse(&value, Some(Version::new(1, 2, 4))).is_err());
        value["prerelease"] = true.into();
        assert!(parse(&value, None).is_err());
        assert!(parse(&value, Some(Version::new(1, 2, 3))).is_ok());
        value["prerelease"] = false.into();
        value["assets"][0]["digest"] = serde_json::Value::Null;
        assert!(parse(&value, None).is_err());
        value["assets"][0]["digest"] = format!("sha256:{}", "ab".repeat(32)).into();
        value["assets"][0]["browser_download_url"] = "https://example.com/janex.zip".into();
        assert!(parse(&value, None).is_err());
    }
}

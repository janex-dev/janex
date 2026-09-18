// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Official portable Gradle and Apache Maven distribution providers.

use super::{AvailableSdk, CatalogOptions, SdkRequest, catalog, request::tool_version};
use crate::{Result, error::invalid};
use janex_format::checksum::Algorithm;
use std::path::Path;

/// Official stable and preview Gradle release metadata, filtered before selection.
const GRADLE_VERSIONS: &str = "https://services.gradle.org/versions/all";
/// Maven Central coordinates for Apache Maven binary distributions.
const MAVEN_BASE: &str = "https://repo.maven.apache.org/maven2/org/apache/maven/apache-maven";

/// Lists stable releases matching the requested tool series, newest first.
pub(super) fn available(
    root: &Path,
    request: &SdkRequest,
    options: &CatalogOptions,
) -> Result<Vec<AvailableSdk>> {
    let mut packages = match request.family() {
        "gradle" => {
            let value = catalog::metadata(root, GRADLE_VERSIONS, options)?;
            gradle_packages(&value, request)?
        }
        "maven" => {
            let bytes =
                catalog::cached_bytes(root, &format!("{MAVEN_BASE}/maven-metadata.xml"), options)?;
            maven_packages(&bytes, request)?
        }
        _ => return Err(invalid("unsupported portable SDK provider")),
    };
    packages.sort_by(|a, b| {
        tool_version(&b.version)
            .unwrap()
            .cmp(&tool_version(&a.version).unwrap())
    });
    packages.dedup_by(|a, b| a.version == b.version);
    Ok(packages)
}

/// Filters release flags and numeric versions independently of provider ordering.
fn gradle_packages(value: &serde_json::Value, request: &SdkRequest) -> Result<Vec<AvailableSdk>> {
    let rows = value
        .as_array()
        .ok_or_else(|| invalid("invalid Gradle release catalog"))?;
    let mut packages = Vec::new();
    for row in rows {
        let version = row
            .get("version")
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid("missing Gradle version"))?;
        if ["snapshot", "nightly", "releaseNightly", "broken"]
            .iter()
            .any(|name| row.get(name).and_then(|v| v.as_bool()) != Some(false))
            || !request.matches(version)
        {
            continue;
        }
        packages.push(package(request, version));
    }
    Ok(packages)
}

/// Parses only the expected Maven metadata version list, with DTDs disabled and bounded nodes.
fn maven_packages(bytes: &[u8], request: &SdkRequest) -> Result<Vec<AvailableSdk>> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("Maven metadata is not UTF-8"))?;
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
            ..Default::default()
        },
    )
    .map_err(|e| invalid(format!("invalid Maven release metadata: {e}")))?;
    let root = document.root_element();
    let child_text = |name: &str| {
        root.children()
            .find(|node| node.has_tag_name(name))
            .and_then(|node| node.text())
    };
    if !root.has_tag_name("metadata")
        || child_text("groupId") != Some("org.apache.maven")
        || child_text("artifactId") != Some("apache-maven")
    {
        return Err(invalid("Maven catalog has unexpected coordinates"));
    }
    let versions = root
        .children()
        .find(|n| n.has_tag_name("versioning"))
        .and_then(|n| n.children().find(|n| n.has_tag_name("versions")))
        .ok_or_else(|| invalid("Maven catalog has no versions"))?;
    Ok(versions
        .children()
        .filter(|n| n.has_tag_name("version"))
        .filter_map(|n| n.text())
        .filter(|version| request.matches(version))
        .map(|version| package(request, version))
        .collect())
}

/// Constructs canonical archive names rather than accepting arbitrary paths from metadata.
fn package(request: &SdkRequest, version: &str) -> AvailableSdk {
    let (filename, archive_type) = match request.family() {
        "gradle" => (format!("gradle-{version}-{}.zip", request.variant), "zip"),
        _ if cfg!(windows) => (format!("apache-maven-{version}-bin.zip"), "zip"),
        _ => (format!("apache-maven-{version}-bin.tar.gz"), "tar.gz"),
    };
    AvailableSdk {
        id: filename.clone(),
        version: version.into(),
        filename,
        archive_type: archive_type.into(),
        request: request.clone(),
    }
}

/// Binds a canonical distribution URL to its official SHA-256 or SHA-512 digest.
pub(super) fn artifact(
    root: &Path,
    package: &AvailableSdk,
    options: &CatalogOptions,
) -> Result<catalog::Artifact> {
    if !package.request.matches(&package.version) {
        return Err(invalid("SDK package does not match request"));
    }
    let canonical = self::package(&package.request, &package.version);
    if canonical.filename != package.filename || canonical.archive_type != package.archive_type {
        return Err(invalid("invalid SDK archive identity"));
    }
    let (url, algorithm, suffix) = match package.request.family() {
        "gradle" => (
            format!(
                "https://services.gradle.org/distributions/{}",
                package.filename
            ),
            Algorithm::Sha256,
            "sha256",
        ),
        "maven" => (
            format!("{MAVEN_BASE}/{}/{}", package.version, package.filename),
            Algorithm::Sha512,
            "sha512",
        ),
        _ => return Err(invalid("unsupported portable SDK provider")),
    };
    let bytes = catalog::cached_bytes(root, &format!("{url}.{suffix}"), options)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid("SDK checksum is not UTF-8"))?;
    let checksum = catalog::parse_checksum(algorithm, text.trim())?;
    Ok(catalog::Artifact { url, checksum })
}

/// Finds the canonical distribution home and verifies its version-bearing core library.
pub(super) fn find_home(root: &Path, request: &SdkRequest) -> Result<std::path::PathBuf> {
    let version = request.version();
    tool_version(version)?;
    let (directory, library) = match request.family() {
        "gradle" => (
            format!("gradle-{version}"),
            format!("gradle-core-{version}.jar"),
        ),
        "maven" => (
            format!("apache-maven-{version}"),
            format!("maven-core-{version}.jar"),
        ),
        _ => return Err(invalid("unsupported portable SDK provider")),
    };
    let home = root.join(directory);
    if !home.join("bin").join(request.executable()).is_file()
        || !home.join("lib").join(library).is_file()
    {
        return Err(invalid(
            "SDK archive is missing its versioned core library or launcher",
        ));
    }
    validate_variant(&home, request)?;
    Ok(home)
}

/// Identifies an existing tool distribution from its versioned core library without executing scripts.
pub(super) fn external_home(
    root: &Path,
    request: &SdkRequest,
) -> Result<(std::path::PathBuf, SdkRequest)> {
    let prefix = match request.family() {
        "gradle" => "gradle-core-",
        "maven" => "maven-core-",
        _ => return Err(invalid("unsupported portable SDK provider")),
    };
    let mut matches = Vec::new();
    if root.join("bin").join(request.executable()).is_file() {
        for entry in std::fs::read_dir(root.join("lib"))? {
            let entry = entry?;
            let name = entry.file_name();
            if let Some(version) = name
                .to_str()
                .and_then(|n| n.strip_prefix(prefix))
                .and_then(|n| n.strip_suffix(".jar"))
                && request.matches(version)
                && entry.path().is_file()
            {
                matches.push(request.with_version(version));
            }
        }
    }
    if matches.len() != 1 {
        return Err(invalid(
            "external SDK must contain one matching versioned core library and launcher",
        ));
    }
    validate_variant(root, request)?;
    Ok((root.to_owned(), matches.remove(0)))
}

/// Distinguishes complete Gradle distributions from binaries without scanning their contents.
fn validate_variant(home: &Path, request: &SdkRequest) -> Result<()> {
    if request.family() == "gradle" {
        let complete = home.join("docs").is_dir() && home.join("src").is_dir();
        let actual = if complete { "all" } else { "bin" };
        if request.variant != actual {
            return Err(invalid(format!(
                "Gradle home contains variant {actual}, expected {}",
                request.variant
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_exclude_previews_and_check_maven_coordinates() {
        let request = SdkRequest::parse("sdk:gradle@latest").unwrap();
        let rows = serde_json::json!([
            {"version":"9.0.0", "snapshot":false,"nightly":false,"releaseNightly":false,"broken":false},
            {"version":"9.1.0-rc-1", "snapshot":false,"nightly":false,"releaseNightly":false,"broken":false},
            {"version":"9.2.0", "snapshot":false,"nightly":false,"releaseNightly":false,"broken":true}
        ]);
        assert_eq!(gradle_packages(&rows, &request).unwrap().len(), 1);
        let xml = b"<metadata><groupId>org.apache.maven</groupId><artifactId>apache-maven</artifactId><versioning><versions><version>3.9.9</version><version>4.0.0-rc-1</version></versions></versioning></metadata>";
        assert_eq!(
            maven_packages(xml, &SdkRequest::parse("sdk:maven@latest").unwrap())
                .unwrap()
                .len(),
            1
        );
        assert!(maven_packages(b"<metadata/>", &request).is_err());
    }
    #[test]
    fn gradle_variants_resolve_distinct_archives_and_checksums() {
        let temp = tempfile::tempdir().unwrap();
        let metadata = temp.path().join("cache/sdk");
        std::fs::create_dir_all(&metadata).unwrap();
        let mut packages = Vec::new();
        for (variant, digit) in [("bin", "a"), ("all", "b")] {
            let request = SdkRequest::parse(&format!("sdk:gradle@9[variant={variant}]")).unwrap();
            let package = package(&request, "9.1.0");
            let url =
                format!("https://services.gradle.org/distributions/gradle-9.1.0-{variant}.zip");
            let checksum_url = format!("{url}.sha256");
            let key = super::super::hex(
                janex_format::checksum::Checksum::compute(
                    Algorithm::Sha256,
                    checksum_url.as_bytes(),
                )
                .unwrap()
                .digest(),
            );
            std::fs::write(metadata.join(format!("{key}.metadata")), digit.repeat(64)).unwrap();
            let resolved = artifact(
                temp.path(),
                &package,
                &CatalogOptions {
                    offline: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(resolved.url, url);
            assert_eq!(
                super::super::hex(resolved.checksum.digest()),
                digit.repeat(64)
            );
            packages.push(package);
        }
        assert_ne!(packages[0].id, packages[1].id);
        let mut mixed = packages[0].clone();
        mixed.request = packages[1].request.clone();
        assert!(
            artifact(
                temp.path(),
                &mixed,
                &CatalogOptions {
                    offline: true,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}

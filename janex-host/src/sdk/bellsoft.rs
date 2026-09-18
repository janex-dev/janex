// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! BellSoft's product API keeps JDK editions and NIK product versions distinct.

use super::{AvailableSdk, CatalogOptions, SdkRequest, catalog, version_order};
use crate::{Result, error::invalid};
use serde_json::Value;
use std::path::Path;
use url::Url;

/// Returns the provider product path and precise bundle type.
fn bundle(request: &SdkRequest) -> Result<(&'static str, String)> {
    let product = request.descriptor()?;
    if product.is_nik() {
        return Ok(("nik", request.variant.clone()));
    }
    let kind = product.kind();
    Ok((
        "liberica",
        if request.variant == "standard" {
            kind.into()
        } else {
            format!("{kind}-{}", request.variant)
        },
    ))
}

/// Converts normalized architectures to BellSoft's architecture and bitness pair.
fn architecture(arch: &str) -> Result<(&'static str, u32)> {
    match arch {
        "x86" => Ok(("x86", 32)),
        "x86-64" => Ok(("x86", 64)),
        "arm" => Ok(("arm", 32)),
        "aarch64" => Ok(("arm", 64)),
        "ppc64le" => Ok(("ppc", 64)),
        "riscv64" => Ok(("riscv", 64)),
        _ => Err(invalid("BellSoft does not support this SDK architecture")),
    }
}

/// Lists stable archives using named bundle variants rather than a JavaFX approximation.
pub(super) fn available(
    root: &Path,
    request: &SdkRequest,
    options: &CatalogOptions,
) -> Result<Vec<AvailableSdk>> {
    let (product, bundle) = bundle(request)?;
    let (arch, bits) = architecture(&request.platform.as_ref().unwrap().arch)?;
    let os = if request.platform.as_ref().unwrap().os == "linux"
        && request.platform.as_ref().unwrap().libc == "musl"
    {
        "linux-musl"
    } else {
        &request.platform.as_ref().unwrap().os
    };
    let format = if os == "windows" { "zip" } else { "tar.gz" };
    let mut url = Url::parse(&format!("https://api.bell-sw.com/v1/{product}/releases")).unwrap();
    url.query_pairs_mut().extend_pairs([
        ("os", os),
        ("arch", arch),
        ("bitness", &bits.to_string()),
        ("package-type", format),
        ("bundle-type", &bundle),
    ]);
    if product == "nik" {
        url.query_pairs_mut().append_pair("components", "nik");
    }
    let value = catalog::metadata(root, url.as_str(), options)?;
    parse_packages(&value, request)
}

/// Validates platform, product component, edition and version before advertising an archive.
fn parse_packages(value: &Value, request: &SdkRequest) -> Result<Vec<AvailableSdk>> {
    let (product, bundle) = bundle(request)?;
    let (arch, bits) = architecture(&request.platform.as_ref().unwrap().arch)?;
    let os = if request.platform.as_ref().unwrap().os == "linux"
        && request.platform.as_ref().unwrap().libc == "musl"
    {
        "linux-musl"
    } else {
        &request.platform.as_ref().unwrap().os
    };
    let format = if os == "windows" { "zip" } else { "tar.gz" };
    let rows = value
        .as_array()
        .ok_or_else(|| invalid("invalid BellSoft release catalog"))?;
    let mut packages = Vec::new();
    for row in rows {
        if row["GA"] != true
            || row["os"] != os
            || row["architecture"] != arch
            || row["bitness"].as_u64() != Some(u64::from(bits))
            || row["packageType"] != format
            || row["bundleType"] != bundle
            || (product == "nik" && row["component"] != "nik")
        {
            continue;
        }
        let version = field(row, "version")?;
        if !request.matches(version) {
            continue;
        }
        let filename = field(row, "filename")?;
        super::archive::safe_component(filename)?;
        packages.push(AvailableSdk {
            id: filename.into(),
            version: version.into(),
            filename: filename.into(),
            archive_type: format.into(),
            request: request.clone(),
        });
    }
    packages.sort_by(|a, b| version_order(&b.version, &a.version).then(a.id.cmp(&b.id)));
    packages.dedup_by(|a, b| a.id == b.id);
    Ok(packages)
}

/// Revalidates the exact release record and binds its download to a secure checksum.
pub(super) fn artifact(
    root: &Path,
    package: &AvailableSdk,
    options: &CatalogOptions,
) -> Result<catalog::Artifact> {
    let request = package
        .request
        .java()
        .ok_or_else(|| invalid("expected BellSoft Java product"))?;
    let (product, _) = bundle(request)?;
    super::archive::safe_component(&package.filename)?;
    let mut url = Url::parse(&format!("https://api.bell-sw.com/v1/{product}/releases/")).unwrap();
    url.path_segments_mut().unwrap().push(&package.filename);
    let row = catalog::metadata(root, url.as_str(), options)?;
    let matches = parse_packages(&Value::Array(vec![row.clone()]), request)?;
    if !matches.iter().any(|p| {
        p.id == package.id && p.version == package.version && p.archive_type == package.archive_type
    }) {
        return Err(invalid("BellSoft download does not match selected SDK"));
    }
    let mut checksum = Value::Object(Default::default());
    for algorithm in ["sha512", "sha256"] {
        if let Some(value) = row
            .get(algorithm)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            checksum["checksum_type"] = algorithm.into();
            checksum["checksum"] = value.into();
            break;
        }
    }
    catalog::secure_artifact(
        root,
        field(&row, "downloadUrl")?,
        &package.filename,
        &checksum,
        options,
    )
}

/// Reads a required catalog string.
fn field<'a>(row: &'a Value, name: &str) -> Result<&'a str> {
    row.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("missing BellSoft {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_keeps_product_editions_platforms_and_nik_versions_distinct() {
        let request =
            SdkRequest::parse("sdk:bellsoft/liberica-jdk@21[os=windows,arch=aarch64,variant=full]")
                .unwrap();
        let row = serde_json::json!({"GA":true,"os":"windows","architecture":"arm","bitness":64,"packageType":"zip","bundleType":"jdk-full","version":"21.0.8+12","filename":"jdk-full.zip"});
        let mut other = row.clone();
        other["bundleType"] = "jdk".into();
        let mut x64 = row.clone();
        x64["architecture"] = "x86".into();
        let packages = parse_packages(
            &serde_json::json!([row, other, x64]),
            request.java().unwrap(),
        )
        .unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].version, "21.0.8+12");
        let request =
            SdkRequest::parse("sdk:bellsoft/liberica-nik@24[os=windows,arch=x86-64]").unwrap();
        let row = serde_json::json!({"GA":true,"os":"windows","architecture":"x86","bitness":64,"packageType":"zip","bundleType":"standard","version":"24.0.2+1","component":"nik","filename":"nik.zip","components":[{"component":"liberica","version":"21.0.8+12"}]});
        let packages = parse_packages(&serde_json::json!([row]), request.java().unwrap()).unwrap();
        assert_eq!(packages[0].version, "24.0.2+1");
        assert_eq!(packages[0].request.product(), "bellsoft/liberica-nik");
    }
}

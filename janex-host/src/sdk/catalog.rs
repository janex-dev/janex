// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Bounded catalog queries and authenticated HTTPS artifact transport.

use super::{JavaRequest, SdkRequest, hex, operating_system, version_order};
use crate::{Result, error::invalid};
use janex_format::checksum::{Algorithm, Checksum};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    time::{Duration, Instant},
};
use url::Url;

/// Options for catalog metadata and archive acquisition.
#[derive(Clone, Debug)]
pub struct CatalogOptions {
    /// Requires an existing metadata cache and prohibits downloads.
    pub offline: bool,
    /// Revalidates catalog metadata instead of using its 24-hour cache.
    pub refresh: bool,
    /// Maximum archive download size in bytes.
    pub max_download_bytes: u64,
    /// Maximum total extracted regular-file content in bytes.
    pub max_extracted_bytes: u64,
    /// Maximum elapsed time for an individual HTTP operation, including its redirects.
    pub timeout: Duration,
    /// Optional byte-progress callback, invoked synchronously during archive downloads.
    pub progress: Option<fn(u64, Option<u64>)>,
}

impl Default for CatalogOptions {
    fn default() -> Self {
        Self {
            offline: false,
            refresh: false,
            max_download_bytes: 2 * 1024 * 1024 * 1024,
            max_extracted_bytes: 4 * 1024 * 1024 * 1024,
            timeout: Duration::from_secs(1800),
            progress: None,
        }
    }
}

/// A matching SDK archive advertised by a provider; not yet downloaded or authenticated.
#[derive(Clone, Debug, Serialize)]
pub struct AvailableSdk {
    /// Opaque catalog package identifier.
    pub id: String,
    /// Complete release including a Java build number when advertised.
    pub version: String,
    /// Original archive filename.
    pub filename: String,
    /// `zip` or `tar.gz`.
    pub archive_type: String,
    /// Selection whose variant this archive satisfies.
    pub request: SdkRequest,
}

/// A download URL bound to a secure digest by the catalog or release service.
pub(super) struct Artifact {
    /// Direct HTTPS URL, with redirects checked during acquisition.
    pub url: String,
    /// SHA-256 or SHA-512 digest advertised by an HTTPS service.
    pub checksum: Checksum,
}

/// Lists matching GA archives in descending version order.
pub(super) fn available(
    root: &Path,
    request: &SdkRequest,
    options: &CatalogOptions,
) -> Result<Vec<AvailableSdk>> {
    request.validate()?;
    let SdkRequest::Java(request) = request else {
        return super::tools::available(root, request, options);
    };
    let mut url = Url::parse("https://api.foojay.io/disco/v3.0/packages").unwrap();
    let archive = if cfg!(windows) { "zip" } else { "tar.gz" };
    let arch = match request.architecture.as_str() {
        "x86-64" => "x64",
        "x86" => "x32",
        other => other,
    };
    url.query_pairs_mut().extend_pairs([
        ("distribution", request.vendor.as_str()),
        ("version", request.version.split('+').next().unwrap()),
        ("operating_system", operating_system()?),
        ("architecture", arch),
        ("archive_type", archive),
        ("package_type", request.kind.as_str()),
        ("release_status", "ga"),
        ("directly_downloadable", "true"),
        (
            "javafx_bundled",
            if request.javafx { "true" } else { "false" },
        ),
        ("lib_c_type", request.libc.as_str()),
    ]);
    if request.version == "latest" {
        let pairs = url
            .query_pairs()
            .filter(|(name, _)| name != "version")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<Vec<_>>();
        url.set_query(None);
        url.query_pairs_mut().extend_pairs(pairs);
    }
    let value = metadata(root, url.as_str(), options)?;
    parse_packages(&value, request, archive)
}

/// Checks every returned variant rather than trusting that the service applied its filters.
fn parse_packages(
    value: &Value,
    request: &JavaRequest,
    archive: &str,
) -> Result<Vec<AvailableSdk>> {
    let rows = value
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("invalid Disco package response"))?;
    let mut result = Vec::new();
    for row in rows {
        let architecture = text(row, "architecture")?;
        let normalized = if architecture == "x32" {
            "x86"
        } else {
            janex_platform::normalize_architecture(architecture)
        };
        if text(row, "distribution")? != request.vendor
            || text(row, "operating_system")? != operating_system()?
            || normalized != request.architecture
            || text(row, "package_type")? != request.kind
            || text(row, "archive_type")? != archive
            || text(row, "release_status")? != "ga"
            || row.get("javafx_bundled").and_then(Value::as_bool) != Some(request.javafx)
            || row.get("directly_downloadable").and_then(Value::as_bool) != Some(true)
            || (cfg!(target_os = "linux") && text(row, "lib_c_type")? != request.libc)
        {
            continue;
        }
        let version = text(row, "java_version")?;
        if !request.matches(version) {
            continue;
        }
        let id = text(row, "id")?;
        if id.len() > 128 || id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err(invalid("invalid Disco package ID"));
        }
        let filename = text(row, "filename")?;
        super::archive::safe_component(filename)?;
        result.push(AvailableSdk {
            id: id.into(),
            version: version.into(),
            filename: filename.into(),
            archive_type: archive.into(),
            request: SdkRequest::Java(request.clone()),
        });
    }
    result.sort_by(|a, b| version_order(&b.version, &a.version).then(a.id.cmp(&b.id)));
    result.dedup_by(|a, b| a.id == b.id);
    Ok(result)
}

/// Resolves an advertised package to a URL and secure checksum, including GitHub SHA-256 fallback.
pub(super) fn artifact(
    root: &Path,
    package: &AvailableSdk,
    options: &CatalogOptions,
) -> Result<Artifact> {
    if package.request.java().is_none() {
        return super::tools::artifact(root, package, options);
    }
    let value = metadata(
        root,
        &format!("https://api.foojay.io/disco/v3.0/ids/{}", package.id),
        options,
    )?;
    let rows = value
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("invalid Disco download response"))?;
    if rows.len() != 1 {
        return Err(invalid("expected exactly one Disco download"));
    }
    let row = &rows[0];
    if text(row, "filename")? != package.filename {
        return Err(invalid("catalog download filename mismatch"));
    }
    let url = text(row, "direct_download_uri")?.to_owned();
    let parsed = secure_url(&url)?;
    let algorithm = match row.get("checksum_type").and_then(Value::as_str) {
        Some("sha256") => Some(Algorithm::Sha256),
        Some("sha512") => Some(Algorithm::Sha512),
        _ => None,
    };
    if let Some(algorithm) = algorithm {
        if let Some(digest) = row
            .get("checksum")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return Ok(Artifact {
                url,
                checksum: parse_checksum(algorithm, digest)?,
            });
        }
        if let Some(checksum_url) = row
            .get("checksum_uri")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            let bytes = fetch_bytes(checksum_url, 8192, options)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid("invalid checksum text"))?;
            let checksum = text
                .split_whitespace()
                .next()
                .ok_or_else(|| invalid("empty checksum"))?;
            return Ok(Artifact {
                url,
                checksum: parse_checksum(algorithm, checksum)?,
            });
        }
    }
    if parsed.host_str() == Some("github.com") {
        let parts: Vec<_> = parsed
            .path_segments()
            .ok_or_else(|| invalid("invalid GitHub asset URL"))?
            .collect();
        if parts.len() == 6 && parts[2] == "releases" && parts[3] == "download" {
            let api = format!(
                "https://api.github.com/repos/{}/{}/releases/tags/{}",
                parts[0], parts[1], parts[4]
            );
            let release = metadata(root, &api, options)?;
            if let Some(assets) = release.get("assets").and_then(Value::as_array) {
                for asset in assets {
                    if asset.get("name").and_then(Value::as_str) == Some(&package.filename) {
                        let actual_url = secure_url(text(asset, "browser_download_url")?)?;
                        if actual_url.origin() != parsed.origin()
                            || actual_url.query() != parsed.query()
                            || percent_encoding::percent_decode_str(actual_url.path())
                                .collect::<Vec<_>>()
                                != percent_encoding::percent_decode_str(parsed.path())
                                    .collect::<Vec<_>>()
                        {
                            continue;
                        }
                        if let Some(digest) = asset
                            .get("digest")
                            .and_then(Value::as_str)
                            .and_then(|s| s.strip_prefix("sha256:"))
                        {
                            return Ok(Artifact {
                                url: actual_url.to_string(),
                                checksum: parse_checksum(Algorithm::Sha256, digest)?,
                            });
                        }
                    }
                }
            }
        }
    }
    Err(invalid(
        "SDK download has no SHA-256 or SHA-512 checksum; this release cannot be installed",
    ))
}

/// Parses an exact hexadecimal digest without accepting a truncated or nonhex value.
pub(super) fn parse_checksum(algorithm: Algorithm, text: &str) -> Result<Checksum> {
    if text.len() != algorithm.digest_length() * 2 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid SDK checksum"));
    }
    let mut bytes = vec![algorithm as u8];
    for pair in text.as_bytes().as_chunks::<2>().0 {
        bytes.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
    }
    Ok(Checksum::decode(&bytes)?)
}

/// Reads a required string from a provider response.
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("missing SDK metadata field: {key}")))
}

/// Reads bounded JSON metadata from a disposable 24-hour cache or HTTPS.
pub(super) fn metadata(root: &Path, url: &str, options: &CatalogOptions) -> Result<Value> {
    serde_json::from_slice(&cached_bytes(root, url, options)?)
        .map_err(|e| invalid(format!("invalid SDK catalog JSON: {e}")))
}

/// Caches bounded provider metadata independently of its JSON, XML, or digest representation.
pub(super) fn cached_bytes(root: &Path, url: &str, options: &CatalogOptions) -> Result<Vec<u8>> {
    let key = hex(Checksum::compute(Algorithm::Sha256, url.as_bytes())?.digest());
    let directory = root.join("cache/sdk");
    let path = directory.join(format!("{key}.metadata"));
    if options.offline && options.refresh {
        return Err(invalid("offline and refresh are mutually exclusive"));
    }
    if !options.refresh
        && let Ok(file) = fs::File::open(&path)
    {
        let fresh = file
            .metadata()?
            .modified()?
            .elapsed()
            .is_ok_and(|age| age < Duration::from_secs(86400));
        if fresh || options.offline {
            let mut bytes = Vec::new();
            file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() <= 16 * 1024 * 1024 {
                return Ok(bytes);
            }
        }
    }
    let bytes = fetch_bytes(url, 16 * 1024 * 1024, options)?;
    fs::create_dir_all(&directory)?;
    super::state::publish(&path, &bytes)?;
    Ok(bytes)
}

/// Downloads a small bounded HTTPS response into memory.
fn fetch_bytes(url: &str, limit: u64, options: &CatalogOptions) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    download(url, &mut bytes, limit, options)?;
    Ok(bytes)
}

/// Streams an identity-encoded HTTPS representation, with a shared deadline across redirects.
pub(super) fn download(
    url: &str,
    output: &mut impl Write,
    limit: u64,
    options: &CatalogOptions,
) -> Result<()> {
    if options.offline {
        return Err(invalid("SDK artifact or metadata is unavailable offline"));
    }
    if limit == u64::MAX || options.timeout.is_zero() {
        return Err(invalid("invalid SDK download limits"));
    }
    let agent = agent();
    let mut response = response(&agent, url, None, options.timeout)?;
    if response.status().as_u16() != 200 {
        return Err(invalid(format!(
            "SDK server returned HTTP {}",
            response.status()
        )));
    }
    if content_length(&response)?.is_some_and(|size| size > limit) {
        return Err(invalid("SDK download byte limit exceeded"));
    }
    let count = std::io::copy(&mut response.body_mut().as_reader().take(limit + 1), output)?;
    if count > limit {
        return Err(invalid("SDK download byte limit exceeded"));
    }
    Ok(())
}

/// Acquires bounded ranges so a stalled archive request can be retried without discarding completed bytes.
pub(super) fn download_archive(
    url: &str,
    output: &mut fs::File,
    options: &CatalogOptions,
) -> Result<()> {
    if options.offline {
        return Err(invalid("SDK archives cannot be downloaded offline"));
    }
    let limit = options.max_download_bytes;
    if limit == 0 || limit == u64::MAX || options.timeout.is_zero() {
        return Err(invalid("invalid SDK download limits"));
    }
    let agent = agent();
    let start = Instant::now();
    let mut offset = 0u64;
    let mut total = None;
    loop {
        let end = offset.saturating_add(8 * 1024 * 1024 - 1).min(limit - 1);
        let range = format!("bytes={offset}-{end}");
        let mut outcome = Err(invalid("SDK download retry limit exceeded"));
        for _ in 0..3 {
            let remaining = options
                .timeout
                .checked_sub(start.elapsed())
                .filter(|d| !d.is_zero())
                .ok_or_else(|| invalid("SDK download timed out"))?;
            output.set_len(offset)?;
            output.seek(SeekFrom::Start(offset))?;
            outcome = (|| -> Result<(u64, u64)> {
                let mut response = response(
                    &agent,
                    url,
                    Some(&range),
                    remaining.min(Duration::from_secs(90)),
                )?;
                let status = response.status().as_u16();
                let (length, complete) = if status == 206 {
                    let header = response
                        .headers()
                        .get("content-range")
                        .and_then(|s| s.to_str().ok())
                        .ok_or_else(|| invalid("SDK range response has no Content-Range"))?;
                    let (first, last, complete) = content_range(header)?;
                    if first != offset
                        || last > end
                        || complete > limit
                        || total.is_some_and(|known| known != complete)
                    {
                        return Err(invalid(
                            "SDK range response does not match the requested bytes",
                        ));
                    }
                    (last - first + 1, complete)
                } else if status == 200 && offset == 0 {
                    let length = content_length(&response)?.ok_or_else(|| {
                        invalid("SDK server without range support must supply Content-Length")
                    })?;
                    if length == 0 || length > limit {
                        return Err(invalid("SDK archive byte limit exceeded"));
                    }
                    (length, length)
                } else {
                    return Err(invalid(format!("SDK range server returned HTTP {status}")));
                };
                if offset == 0
                    && let Some(progress) = options.progress
                {
                    progress(0, Some(complete));
                }
                let count = std::io::copy(
                    &mut response.body_mut().as_reader().take(length + 1),
                    output,
                )?;
                if count != length {
                    return Err(invalid("SDK archive range length mismatch"));
                }
                Ok((count, complete))
            })();
            if outcome.is_ok() {
                break;
            }
        }
        let (count, complete) = outcome?;
        offset += count;
        total = Some(complete);
        if let Some(progress) = options.progress {
            progress(offset, total);
        }
        if offset == complete {
            return Ok(());
        }
    }
}

/// Parses a complete byte-range response, rejecting unknown lengths and inconsistent bounds.
fn content_range(value: &str) -> Result<(u64, u64, u64)> {
    let (range, total) = value
        .strip_prefix("bytes ")
        .and_then(|s| s.split_once('/'))
        .ok_or_else(|| invalid("invalid SDK Content-Range"))?;
    let (first, last) = range
        .split_once('-')
        .ok_or_else(|| invalid("invalid SDK Content-Range"))?;
    let number = |s: &str| {
        s.parse::<u64>()
            .map_err(|_| invalid("invalid SDK Content-Range number"))
    };
    let (first, last, total) = (number(first)?, number(last)?, number(total)?);
    if first > last || last >= total {
        return Err(invalid("invalid SDK Content-Range bounds"));
    }
    Ok((first, last, total))
}

/// Reads an optional representation length with strict integer validation.
fn content_length(response: &ureq::http::Response<ureq::Body>) -> Result<Option<u64>> {
    response
        .headers()
        .get("content-length")
        .map(|v| {
            v.to_str()
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| invalid("invalid SDK Content-Length"))
        })
        .transpose()
}

/// Creates an agent that delegates all redirect validation to the SDK transport.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_resolve(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// Opens one identity-encoded response while retaining HTTPS across a bounded redirect chain.
fn response(
    agent: &ureq::Agent,
    url: &str,
    range: Option<&str>,
    timeout: Duration,
) -> Result<ureq::http::Response<ureq::Body>> {
    let mut url = secure_url(url)?;
    let start = Instant::now();
    for redirects in 0..=5 {
        let remaining = timeout
            .checked_sub(start.elapsed())
            .filter(|v| !v.is_zero())
            .ok_or_else(|| invalid("SDK download timed out"))?;
        let mut request = agent
            .get(url.as_str())
            .header("User-Agent", "Janex/0.1")
            .header("Accept-Encoding", "identity");
        if let Some(range) = range {
            request = request.header("Range", range);
        }
        let response = request
            .config()
            .timeout_global(Some(remaining))
            .build()
            .call()
            .map_err(|e| invalid(format!("SDK request failed: {e}")))?;
        let status = response.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            if redirects == 5 {
                return Err(invalid("SDK redirect limit exceeded"));
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| invalid("SDK redirect has no valid Location"))?;
            url = secure_url(
                url.join(location)
                    .map_err(|_| invalid("invalid SDK redirect"))?
                    .as_str(),
            )?;
            continue;
        }
        if let Some(encoding) = response.headers().get("content-encoding")
            && encoding != "identity"
        {
            return Err(invalid("SDK server returned unsupported content encoding"));
        }
        return Ok(response);
    }
    unreachable!()
}

/// Rejects plaintext, credentials, fragments, and unsupported URL schemes.
fn secure_url(text: &str) -> Result<Url> {
    let url = Url::parse(text).map_err(|_| invalid("invalid SDK HTTPS URL"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "SDK metadata and downloads require credential-free HTTPS URLs",
        ));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_transport_and_hashes_reject_unsafe_inputs() {
        for url in [
            "http://example.org/a",
            "https://user:secret@example.org/a",
            "file:///tmp/a",
            "https://example.org/a#fragment",
        ] {
            assert!(secure_url(url).is_err());
        }
        assert!(parse_checksum(Algorithm::Sha256, "abcd").is_err());
        assert!(parse_checksum(Algorithm::Sha256, &"z".repeat(64)).is_err());
        assert_eq!(
            parse_checksum(Algorithm::Sha256, &"ab".repeat(32))
                .unwrap()
                .digest(),
            &[0xab; 32]
        );
        assert_eq!(content_range("bytes 8-15/24").unwrap(), (8, 15, 24));
        for range in [
            "bytes 8-7/24",
            "bytes 8-24/24",
            "bytes 0-1/*",
            "items 0-1/2",
            "bytes 0-18446744073709551615/1",
        ] {
            assert!(content_range(range).is_err(), "{range}");
        }
    }

    #[test]
    fn metadata_cannot_override_requested_platform_or_version() {
        let request = JavaRequest::parse("bellsoft@21").unwrap();
        let row = serde_json::json!({"id": "1234abcd", "distribution": "liberica", "java_version": "21.0.8+12",
            "operating_system": operating_system().unwrap(), "architecture": request.architecture,
            "package_type": "jdk", "archive_type": "zip", "release_status": "ga", "javafx_bundled": false,
            "directly_downloadable": true, "lib_c_type": request.libc, "filename": "jdk.zip"});
        let mut wrong = row.clone();
        wrong["architecture"] = serde_json::json!("unknown");
        let mut newer = row.clone();
        newer["java_version"] = serde_json::json!("22+1");
        let data = serde_json::json!({"result": [wrong, row, newer]});
        let packages = parse_packages(&data, &request, "zip").unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].version, "21.0.8+12");
    }
}

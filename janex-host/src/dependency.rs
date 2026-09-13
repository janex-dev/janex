// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Bounded HTTP(S) acquisition of explicitly named Java dependencies.

use crate::{Error, Result, error::invalid};
use janex_format::checksum::{Algorithm, Checksum};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
use url::Url;

/// Network and persistent-cache policy for external Java path entries.
#[derive(Clone, Debug)]
pub struct DependencyOptions {
    /// Cache directory; absent uses the current user's platform cache directory.
    pub cache_directory: Option<PathBuf>,
    /// Allows only verified cache hits, without starting network requests.
    pub offline: bool,
    /// Bypasses cache hits and atomically replaces an entry after successful verification.
    /// Cannot be combined with `offline`.
    pub refresh: bool,
    /// Maven repository used when a PURL has no `repository_url` qualifier.
    pub maven_repository: String,
    /// Maximum raw bytes returned for one dependency, including a cache hit.
    pub max_bytes: u64,
    /// Maximum elapsed time for a download, including redirects and body reads.
    pub timeout: Duration,
}

impl Default for DependencyOptions {
    fn default() -> Self {
        Self {
            cache_directory: None,
            offline: false,
            refresh: false,
            maven_repository: "https://repo.maven.apache.org/maven2/".into(),
            max_bytes: 512 * 1024 * 1024,
            timeout: Duration::from_secs(60),
        }
    }
}

/// An owned copy of resolved bytes, independent of later network or cache changes.
#[derive(Debug)]
pub struct Dependency {
    /// Original URL or Maven artifact filename used for automatic-module naming.
    pub jar_name: String,
    /// Complete JAR bytes before manifest rewriting or resource import.
    pub bytes: Vec<u8>,
}

/// Resolves one HTTP(S) JAR or exact Maven PURL, verifying its declared checksum when present.
///
/// HTTP requires a cryptographically secure checksum. `require_secure` also requires one for
/// HTTPS, allowing an authenticated descriptor to bind the downloaded bytes. TLS always validates
/// server certificates. Redirects cannot downgrade HTTPS, and at most five redirects are followed.
/// Cache hits are rechecked and copied into owned memory. Corrupt entries are refreshed online and
/// fail offline. Complete downloads are published atomically; failures never publish partial bytes.
/// A per-entry operating-system lock serializes cache updates across processes. Cache access may
/// block until another holder releases its lock; offline readers use a shared read-only lock.
/// Maven resolution does not read POMs, expand transitive dependencies, or select version ranges.
pub fn resolve(
    uri: &str,
    checksum: Option<&Checksum>,
    options: &DependencyOptions,
    require_secure: bool,
) -> Result<Dependency> {
    let (url, jar_name) = address(uri, options)?;
    let secure = checksum.is_some_and(|value| value.algorithm().is_secure());
    if (require_secure || url.scheme() == "http") && !secure {
        return Err(Error::Trust(
            "this external dependency requires a secure checksum".into(),
        ));
    }
    if options.max_bytes == u64::MAX
        || options.timeout.is_zero()
        || (options.offline && options.refresh)
    {
        return Err(invalid("invalid dependency size or timeout limit"));
    }
    let directory = match &options.cache_directory {
        Some(path) => path.clone(),
        None => cache_directory()?,
    };
    let mut identity = url.as_str().as_bytes().to_vec();
    identity.push(0);
    if let Some(checksum) = checksum {
        identity.extend(checksum.encode());
    }
    let key = Checksum::compute(Algorithm::Sha256, identity.as_slice())?;
    let key: String = key
        .digest()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let path = directory.join(format!("{key}.cache"));
    if !options.offline {
        fs::create_dir_all(&directory)?;
    }
    let lock = match fs::OpenOptions::new()
        .read(true)
        .write(!options.offline)
        .create(!options.offline)
        .truncate(false)
        .open(directory.join(format!("{key}.lock")))
    {
        Ok(file) => file,
        Err(error) if options.offline && error.kind() == std::io::ErrorKind::NotFound => {
            return Err(invalid("dependency is unavailable in the offline cache"));
        }
        Err(error) => return Err(error.into()),
    };
    if options.offline {
        fs4::FileExt::lock_shared(&lock)?;
    } else {
        fs4::FileExt::lock(&lock)?;
    }
    if !options.refresh {
        match fs::File::open(&path) {
            Ok(mut file) => {
                let cached = (|| -> Result<Vec<u8>> {
                    let mut header = [0u8; 40];
                    file.read_exact(&mut header)?;
                    if &header[..8] != b"JNXDEP01" {
                        return Err(invalid("invalid dependency cache header"));
                    }
                    let bytes = bounded(file, options.max_bytes)?;
                    if verify(&bytes, checksum)?.digest() != &header[8..] {
                        return Err(invalid("dependency cache checksum mismatch"));
                    }
                    Ok(bytes)
                })();
                match cached {
                    Ok(bytes) => return Ok(Dependency { jar_name, bytes }),
                    Err(error) if options.offline => return Err(error),
                    Err(_) => {}
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if options.offline {
        return Err(invalid("dependency is unavailable in the offline cache"));
    }
    let bytes = download(url, options)?;
    let digest = verify(&bytes, checksum)?;
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    temporary.write_all(b"JNXDEP01")?;
    temporary.write_all(digest.digest())?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path).map_err(|error| error.error)?;
    Ok(Dependency { jar_name, bytes })
}

/// Returns the cache digest after verifying the declared checksum, sharing the SHA-256 calculation.
fn verify(bytes: &[u8], checksum: Option<&Checksum>) -> Result<Checksum> {
    let digest = Checksum::compute(Algorithm::Sha256, bytes)?;
    if let Some(checksum) = checksum {
        if checksum.algorithm() == Algorithm::Sha256 {
            if checksum != &digest {
                return Err(janex_format::Error::new(
                    janex_format::ErrorKind::Verification,
                    "dependency checksum mismatch",
                )
                .into());
            }
        } else {
            checksum.verify(bytes)?;
        }
    }
    Ok(digest)
}

/// Reads at most the configured bytes plus one byte used to detect oversized input.
fn bounded(input: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("dependency byte limit exceeded"));
    }
    Ok(bytes)
}

/// Downloads an exact representation, checking every redirect before sending its request.
fn download(mut url: Url, options: &DependencyOptions) -> Result<Vec<u8>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .build()
        .into();
    let start = Instant::now();
    for redirects in 0..=5 {
        let remaining = options
            .timeout
            .checked_sub(start.elapsed())
            .filter(|value| !value.is_zero())
            .ok_or_else(|| invalid("dependency download timed out"))?;
        let mut response = agent
            .get(url.as_str())
            .header("Accept-Encoding", "identity")
            .config()
            .timeout_global(Some(remaining))
            .build()
            .call()
            .map_err(|error| invalid(format!("dependency request failed: {error}")))?;
        let status = response.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            if redirects == 5 {
                return Err(invalid("dependency redirect limit exceeded"));
            }
            let location = response
                .headers()
                .get("location")
                .ok_or_else(|| invalid("dependency redirect has no Location"))?
                .to_str()
                .map_err(|_| invalid("invalid dependency redirect"))?;
            let next = url
                .join(location)
                .map_err(|_| invalid("invalid dependency redirect URL"))?;
            validate_url(&next)?;
            if url.scheme() == "https" && next.scheme() != "https" {
                return Err(Error::Trust("dependency redirect downgrades HTTPS".into()));
            }
            url = next;
            continue;
        }
        if status != 200 {
            return Err(invalid(format!("dependency server returned HTTP {status}")));
        }
        if let Some(encoding) = response.headers().get("content-encoding")
            && encoding != "identity"
        {
            return Err(invalid(
                "dependency server returned an unsupported content encoding",
            ));
        }
        if let Some(length) = response.headers().get("content-length") {
            let length = length
                .to_str()
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or_else(|| invalid("invalid dependency Content-Length"))?;
            if length > options.max_bytes {
                return Err(invalid("dependency byte limit exceeded"));
            }
        }
        return bounded(response.body_mut().as_reader(), options.max_bytes);
    }
    unreachable!()
}

/// Maps a canonical external name to its exact transport URL and original JAR filename.
fn address(uri: &str, options: &DependencyOptions) -> Result<(Url, String)> {
    let (url, name) = if uri.starts_with("pkg:") {
        let package = janex_format::purl::parse(uri)
            .map_err(|error| invalid(format!("invalid dependency PURL: {error}")))?;
        if package.ty() != "maven" || package.subpath().is_some() {
            return Err(Error::Unsupported(
                "dependency resolver requires a canonical Maven PURL without a subpath".into(),
            ));
        }
        let group = package
            .namespace()
            .ok_or_else(|| invalid("Maven dependency requires a group ID"))?;
        let version = package
            .version()
            .ok_or_else(|| invalid("Maven dependency requires an exact version"))?;
        if version.ends_with("-SNAPSHOT")
            || matches!(version, "LATEST" | "RELEASE")
            || version.contains(['[', ']', '(', ')', ','])
        {
            return Err(Error::Unsupported(
                "Maven dependency requires an exact release or timestamped snapshot version".into(),
            ));
        }
        let qualifiers = package.qualifiers();
        for key in qualifiers.keys() {
            if !matches!(key.as_ref(), "classifier" | "type" | "repository_url") {
                return Err(Error::Unsupported(format!(
                    "unsupported Maven qualifier: {key}"
                )));
            }
        }
        let kind = qualifiers.get("type").map_or("jar", |value| value.as_ref());
        let default_classifier = match kind {
            "jar" | "bundle" | "maven-plugin" | "ejb" => None,
            "test-jar" => Some("tests"),
            "java-source" => Some("sources"),
            "javadoc" => Some("javadoc"),
            "ejb-client" => Some("client"),
            _ => {
                return Err(Error::Unsupported(
                    "Maven Java paths must resolve to JAR artifacts".into(),
                ));
            }
        };
        let classifier = qualifiers
            .get("classifier")
            .map(|value| value.as_ref())
            .or(default_classifier);
        let mut segments: Vec<&str> = group.split('.').collect();
        segments.extend([package.name(), version]);
        for part in segments.iter().copied().chain(classifier) {
            segment(part)?;
        }
        let base_version = snapshot_base(version);
        *segments.last_mut().unwrap() = &base_version;
        let name = format!(
            "{}-{version}{}.jar",
            package.name(),
            classifier.map_or(String::new(), |value| format!("-{value}"))
        );
        let repository = qualifiers
            .get("repository_url")
            .map_or(options.maven_repository.as_str(), |value| value.as_ref());
        let mut url =
            Url::parse(repository).map_err(|_| invalid("invalid Maven repository URL"))?;
        validate_url(&url)?;
        if url.query().is_some() {
            return Err(invalid("Maven repository URL must not have a query"));
        }
        url.path_segments_mut()
            .map_err(|_| invalid("invalid Maven repository path"))?
            .pop_if_empty()
            .extend(segments)
            .push(&name);
        (url, name)
    } else {
        let url = Url::parse(uri).map_err(|_| invalid("invalid dependency URL"))?;
        validate_url(&url)?;
        let encoded = url
            .path_segments()
            .and_then(|mut parts| parts.next_back())
            .unwrap_or("");
        let name = percent_encoding::percent_decode_str(encoded)
            .decode_utf8()
            .map_err(|_| invalid("dependency filename is not UTF-8"))?
            .into_owned();
        segment(&name)?;
        if !name.ends_with(".jar") {
            return Err(invalid("HTTP dependency URL must name a .jar file"));
        }
        (url, name)
    };
    validate_url(&url)?;
    Ok((url, name))
}

/// Rejects path components that could change repository layout or native file identity.
fn segment(value: &str) -> Result<()> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value
            .chars()
            .any(|ch| ch.is_control() || "/\\:<>\"|?*".contains(ch))
    {
        return Err(invalid("invalid dependency path component"));
    }
    Ok(())
}

/// Converts a timestamped Maven snapshot version to its repository directory version.
fn snapshot_base(version: &str) -> String {
    let mut parts = version.rsplitn(3, '-');
    if let (Some(build), Some(stamp), Some(base)) = (parts.next(), parts.next(), parts.next())
        && !build.is_empty()
        && build.bytes().all(|b| b.is_ascii_digit())
        && stamp.len() == 15
        && stamp.as_bytes()[8] == b'.'
        && stamp
            .bytes()
            .enumerate()
            .all(|(index, b)| index == 8 || b.is_ascii_digit())
    {
        return format!("{base}-SNAPSHOT");
    }
    version.to_owned()
}

/// Allows only credential-free HTTP(S) transport URLs without fragments.
fn validate_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "dependency URL must use HTTP(S) without credentials or a fragment",
        ));
    }
    Ok(())
}

/// Locates the user's dependency cache without creating it for local-only launches.
fn cache_directory() -> Result<PathBuf> {
    Ok(janex_platform::janex_home()?.join("cache/dependencies"))
}

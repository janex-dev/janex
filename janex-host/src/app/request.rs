// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Maven application PURLs and CLI shorthands, independent of SDK version rules.

use crate::maven::CENTRAL;
use crate::{Result, error::invalid};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use url::Url;

/// How a JAR application obtains its runtime dependencies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyMode {
    /// Resolve the published POM's runtime dependency graph before installation or execution.
    #[default]
    Maven,
    /// Keep a self-contained JAR without reading its POM.
    None,
}

/// Local JAR launch options, independent of package identity.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct JarOptions {
    /// Explicit binary class name; absent uses the manifest's Main-Class.
    pub main_class: Option<String>,
    /// Whether acquisition resolves POM dependencies.
    pub dependencies: DependencyMode,
}

/// Characters escaped inside a canonical ECMA-427 component.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'.')
    .remove(b'-')
    .remove(b'_')
    .remove(b'~')
    .remove(b':');

/// A canonical Maven package reference and its independent local command name.
/// An absent version follows repository release metadata; a present version is exact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppRequest {
    /// Authoritative package identity, excluding local installation options.
    purl: String,
    /// Decoded Maven group identifier.
    pub(super) group: String,
    /// Decoded Maven artifact identifier.
    pub(super) artifact: String,
    /// Exact release, or `None` to follow repository release metadata.
    pub(super) version: Option<String>,
    /// Optional Maven artifact classifier.
    pub(super) classifier: Option<String>,
    /// Supported Maven artifact type: `jar` or `janex`.
    pub(super) kind: String,
    /// Canonical HTTPS or absolute file repository URL ending with a slash.
    pub(super) repository: String,
    /// Lowercase command name exposed under the Janex bin directory.
    pub command: String,
    /// JAR launch options; Janex artifacts require the default value and use their own descriptor.
    pub jar: JarOptions,
}

impl AppRequest {
    /// Parses a Maven PURL or `maven:group:artifact[@version]` shorthand.
    /// Both forms accept local `[key=value]` options; shorthand options may also name qualifiers.
    /// Versions have the same exact meaning in both forms; omit the version to track releases.
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_with_repository(text, CENTRAL)
    }

    /// Parses a target, using `repository` only when it has no explicit repository qualifier.
    pub(crate) fn parse_with_repository(text: &str, repository: &str) -> Result<Self> {
        let (text, options) = match text.split_once('[') {
            Some((base, rest)) => (
                base,
                Some(
                    rest.strip_suffix(']')
                        .ok_or_else(|| invalid("unterminated application options"))?,
                ),
            ),
            None => (text, None),
        };
        let mut command = None;
        let mut jar = JarOptions::default();
        let mut option_qualifiers = BTreeMap::new();
        let mut local_keys = std::collections::BTreeSet::new();
        if let Some(options) = options {
            for item in options.split(',') {
                let (key, value) = item
                    .split_once('=')
                    .ok_or_else(|| invalid("expected application option key=value"))?;
                if value.is_empty() {
                    return Err(invalid("empty application option"));
                }
                if matches!(key, "command" | "main-class" | "dependencies") {
                    if !local_keys.insert(key) {
                        return Err(invalid("duplicate application option"));
                    }
                    match key {
                        "command" => command = Some(value.to_owned()),
                        "main-class" => jar.main_class = Some(value.to_owned()),
                        "dependencies" => {
                            jar.dependencies = match value {
                                "maven" => DependencyMode::Maven,
                                "none" => DependencyMode::None,
                                _ => return Err(invalid("dependencies must be maven or none")),
                            }
                        }
                        _ => unreachable!(),
                    }
                } else {
                    let key = if key == "repository" {
                        "repository_url"
                    } else {
                        key
                    };
                    insert_qualifier(&mut option_qualifiers, key, value.into())?;
                }
            }
        }
        let mut request = if text.starts_with("pkg:") {
            if !option_qualifiers.is_empty() {
                return Err(invalid("use PURL query qualifiers for package coordinates"));
            }
            Self::from_purl(text, command, repository)?
        } else {
            let text = text
                .strip_prefix("maven:")
                .ok_or_else(|| invalid("expected a Maven PURL or maven:group:artifact"))?;
            let (coordinates, query) = text.split_once('?').unwrap_or((text, ""));
            let (coordinates, version) = match coordinates.split_once('@') {
                Some((coordinates, version)) => (coordinates, Some(decode(version)?)),
                None => (coordinates, None),
            };
            let (group, artifact) = coordinates
                .split_once(':')
                .ok_or_else(|| invalid("Maven application requires group:artifact"))?;
            let mut qualifiers = query_qualifiers(query)?;
            for (key, value) in option_qualifiers {
                insert_qualifier(&mut qualifiers, &key, value)?;
            }
            Self::from_parts(
                decode(group)?,
                decode(artifact)?,
                version,
                qualifiers,
                command,
                repository,
            )?
        };
        request.jar = jar;
        request.validate()?;
        Ok(request)
    }

    /// Returns the canonical package reference without local command or update policy fields.
    pub fn purl(&self) -> &str {
        &self.purl
    }

    /// Recognizes full PURL inputs and Maven shorthands before ecosystem-specific parsing.
    pub fn recognizes(text: &str) -> bool {
        text.starts_with("pkg:") || text.starts_with("maven:")
    }

    /// Validates local launch options; package components are validated at construction.
    pub fn validate(&self) -> Result<()> {
        command_name(&self.command)?;
        if self.kind != "jar" && self.jar != JarOptions::default() {
            return Err(invalid("JAR launch options do not apply to Janex packages"));
        }
        if self.jar.main_class.as_ref().is_some_and(|name| {
            name.is_empty()
                || name.starts_with('-')
                || name.chars().any(|c| {
                    c.is_whitespace() || matches!(c, '/' | '\\' | '[' | ']' | ',' | ';' | '\0')
                })
        }) {
            return Err(invalid("invalid application main class"));
        }
        Ok(())
    }

    /// Returns a round-trippable PURL followed by nondefault local installation options.
    pub fn target(&self) -> String {
        let mut options = Vec::new();
        if self.command != self.artifact.to_ascii_lowercase() {
            options.push(format!("command={}", self.command));
        }
        if let Some(main) = &self.jar.main_class {
            options.push(format!("main-class={main}"));
        }
        if self.jar.dependencies == DependencyMode::None {
            options.push("dependencies=none".into());
        }
        if options.is_empty() {
            self.purl.clone()
        } else {
            format!("{}[{}]", self.purl, options.join(","))
        }
    }

    /// Parses PURL components without treating URL query values as form-encoded text.
    fn from_purl(text: &str, command: Option<String>, repository: &str) -> Result<Self> {
        let text = text
            .strip_prefix("pkg:")
            .ok_or_else(|| invalid("invalid Package URL scheme"))?
            .trim_start_matches('/');
        if text.contains('#') {
            return Err(invalid(
                "application installation does not support PURL subpaths",
            ));
        }
        let (path, query) = text.split_once('?').unwrap_or((text, ""));
        let (path, version) = match path.split_once('@') {
            Some((path, "")) => (path, None),
            Some((path, version)) => (path, Some(decode(version)?)),
            None => (path, None),
        };
        let (kind, path) = path
            .split_once('/')
            .ok_or_else(|| invalid("missing Package URL type"))?;
        if !kind.eq_ignore_ascii_case("maven") {
            return Err(crate::Error::Unsupported(format!(
                "application installation does not support PURL type {kind}"
            )));
        }
        let (group, artifact) = path
            .split_once('/')
            .ok_or_else(|| invalid("Maven PURL requires groupId/artifactId"))?;
        Self::from_parts(
            decode(group)?,
            decode(artifact)?,
            version,
            query_qualifiers(query)?,
            command,
            repository,
        )
    }

    /// Validates decoded coordinates and emits the one stored package representation.
    fn from_parts(
        group: String,
        artifact: String,
        version: Option<String>,
        mut qualifiers: BTreeMap<String, String>,
        command: Option<String>,
        default_repository: &str,
    ) -> Result<Self> {
        for part in group.split('.') {
            component(part)?;
        }
        component(&artifact)?;
        if let Some(version) = &version {
            release_version(version)?;
        }
        let classifier = qualifiers.remove("classifier");
        if let Some(classifier) = &classifier {
            component(classifier)?;
        }
        let kind = qualifiers.remove("type").unwrap_or_else(|| "jar".into());
        if !matches!(kind.as_str(), "jar" | "janex") {
            return Err(crate::Error::Unsupported(
                "application installation supports Maven types jar and janex".into(),
            ));
        }
        let repository = repository(
            qualifiers
                .remove("repository_url")
                .as_deref()
                .unwrap_or(default_repository),
        )?;
        if let Some(key) = qualifiers.keys().next() {
            return Err(crate::Error::Unsupported(format!(
                "unsupported Maven application qualifier: {key}"
            )));
        }
        let command = command.unwrap_or_else(|| artifact.to_ascii_lowercase());
        command_name(&command)?;
        let mut request = Self {
            purl: String::new(),
            group,
            artifact,
            version,
            classifier,
            kind,
            repository,
            command,
            jar: JarOptions::default(),
        };
        request.purl = request.encode();
        // Persisted identities use the same canonical PURL rules as package metadata.
        janex_format::purl::parse(&request.purl)?;
        Ok(request)
    }

    /// Encodes validated components, omitting Maven defaults and sorting qualifier keys.
    fn encode(&self) -> String {
        let mut result = format!(
            "pkg:maven/{}/{}",
            encode(&self.group),
            encode(&self.artifact)
        );
        if let Some(version) = &self.version {
            result.push('@');
            result.push_str(&encode(version));
        }
        let mut qualifiers = Vec::new();
        if let Some(classifier) = &self.classifier {
            qualifiers.push(format!("classifier={}", encode(classifier)));
        }
        if self.repository != CENTRAL {
            qualifiers.push(format!("repository_url={}", encode(&self.repository)));
        }
        if self.kind != "jar" {
            qualifiers.push(format!("type={}", encode(&self.kind)));
        }
        if !qualifiers.is_empty() {
            result.push('?');
            result.push_str(&qualifiers.join("&"));
        }
        result
    }

    /// Returns the same request with a concrete release and its corresponding PURL.
    pub(super) fn exact(&self, version: &str) -> Result<Self> {
        release_version(version)?;
        let mut result = self.clone();
        result.version = Some(version.into());
        result.purl = result.encode();
        Ok(result)
    }

    /// Tests whether two versions may share a command without replacing another product's entry.
    pub(super) fn same_product(&self, other: &Self) -> bool {
        self.group == other.group
            && self.artifact == other.artifact
            && self.repository == other.repository
    }

    /// Builds the Maven filename; only concrete requests may be used for artifact access.
    pub(super) fn filename(&self) -> String {
        format!(
            "{}-{}{}.{}",
            self.artifact,
            self.version
                .as_deref()
                .expect("concrete application release"),
            self.classifier
                .as_ref()
                .map_or(String::new(), |c| format!("-{c}")),
            self.kind
        )
    }

    /// Builds a metadata or artifact URL using Maven repository layout.
    pub(super) fn url(&self, metadata: bool) -> Result<Url> {
        let mut url =
            Url::parse(&self.repository).map_err(|_| invalid("invalid repository URL"))?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| invalid("invalid repository path"))?;
            path.pop_if_empty()
                .extend(self.group.split('.'))
                .push(&self.artifact);
            if metadata {
                path.push("maven-metadata.xml");
            } else {
                path.push(
                    self.version
                        .as_deref()
                        .ok_or_else(|| invalid("application artifact requires an exact version"))?,
                )
                .push(&self.filename());
            }
        }
        Ok(url)
    }
}

impl Serialize for AppRequest {
    /// Stores the canonical PURL and local command without duplicating package coordinates.
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct(
            "AppRequest",
            if self.jar == JarOptions::default() {
                2
            } else {
                3
            },
        )?;
        value.serialize_field("purl", &self.purl)?;
        value.serialize_field("command", &self.command)?;
        if self.jar != JarOptions::default() {
            value.serialize_field("jar", &self.jar)?;
        }
        value.end()
    }
}

impl<'de> Deserialize<'de> for AppRequest {
    /// Reconstructs decoded fields from the persisted PURL and validates the local command.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        /// Persisted identity and installation options.
        #[derive(Deserialize)]
        struct StoredRequest {
            /// Package identity, without local options.
            purl: String,
            /// Local executable name.
            command: String,
            /// Local JAR launch policy.
            #[serde(default)]
            jar: JarOptions,
        }
        let value = StoredRequest::deserialize(deserializer)?;
        let mut request = Self::from_purl(&value.purl, Some(value.command), CENTRAL)
            .map_err(serde::de::Error::custom)?;
        request.jar = value.jar;
        request.validate().map_err(serde::de::Error::custom)?;
        Ok(request)
    }
}

/// Percent-encodes one component with uppercase escapes and literal colons.
fn encode(value: &str) -> String {
    utf8_percent_encode(value, COMPONENT).to_string()
}

/// Decodes UTF-8 percent escapes, preserving literal plus characters.
fn decode(value: &str) -> Result<String> {
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%'
            && !(input.next().is_some_and(|b| b.is_ascii_hexdigit())
                && input.next().is_some_and(|b| b.is_ascii_hexdigit()))
        {
            return Err(invalid("invalid Package URL percent escape"));
        }
    }
    percent_decode_str(value)
        .decode_utf8()
        .map(|s| s.into_owned())
        .map_err(|_| invalid("Package URL component is not UTF-8"))
}

/// Reads PURL qualifiers, omitting empty values as required by ECMA-427.
fn query_qualifiers(query: &str) -> Result<BTreeMap<String, String>> {
    let mut qualifiers = BTreeMap::new();
    if !query.is_empty() {
        for pair in query.split('&') {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| invalid("expected Package URL qualifier key=value"))?;
            if !value.is_empty() {
                insert_qualifier(&mut qualifiers, key, decode(value)?)?;
            }
        }
    }
    Ok(qualifiers)
}

/// Normalizes qualifier keys and rejects duplicate or invalid names.
fn insert_qualifier(values: &mut BTreeMap<String, String>, key: &str, value: String) -> Result<()> {
    if !key.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(invalid("invalid Package URL qualifier key"));
    }
    if values.insert(key.to_ascii_lowercase(), value).is_some() {
        return Err(invalid("duplicate Package URL qualifier"));
    }
    Ok(())
}

/// Checks the supported exact-release subset without interpreting versions as ranges.
fn release_version(value: &str) -> Result<()> {
    component(value)?;
    if value.to_ascii_uppercase().ends_with("-SNAPSHOT") || matches!(value, "LATEST" | "RELEASE") {
        return Err(invalid(
            "application installation requires an exact release; snapshots are not supported",
        ));
    }
    Ok(())
}

/// Requires a bounded ASCII coordinate component without filesystem or selector delimiters.
fn component(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 160
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
        || value.ends_with('.')
    {
        return Err(invalid(
            "unsupported Maven application coordinate component",
        ));
    }
    Ok(())
}

/// Rejects ambiguous, reserved, or shell-dependent command filenames on every platform.
pub(super) fn command_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 100
        || !value.as_bytes()[0].is_ascii_lowercase()
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
        || matches!(value, "janex" | "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|p| {
            value
                .strip_prefix(p)
                .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        })
    {
        return Err(invalid(
            "invalid application command name; use a lowercase command option",
        ));
    }
    Ok(())
}

/// Normalizes credential-free HTTPS repositories and absolute local Maven repositories.
fn repository(value: &str) -> Result<String> {
    let mut url = Url::parse(value).map_err(|_| invalid("invalid application repository URL"))?;
    if !matches!(url.scheme(), "https" | "file")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (url.scheme() == "https" && url.host_str().is_none())
        || (url.scheme() == "file" && url.to_file_path().is_err())
    {
        return Err(invalid(
            "application repository must be an HTTPS or absolute file URL without credentials, query, or fragment",
        ));
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purls_and_shorthands_have_one_identity_and_default_to_jar() {
        let expected = AppRequest::parse("pkg:maven/org.benf/cfr@0.152").unwrap();
        for input in [
            "maven:org.benf:cfr@0.152",
            "maven:org.benf:cfr@0.152[type=jar]",
            "pkg:maven/org.benf/cfr@0.152?type=jar",
            "pkg://MAVEN/org.benf/%63fr@0.152?TYPE=jar&classifier=",
            "pkg:maven/org.benf/cfr@0.152?repository_url=https://repo.maven.apache.org/maven2",
        ] {
            assert_eq!(AppRequest::parse(input).unwrap(), expected, "{input}");
        }
        assert_eq!(expected.purl(), "pkg:maven/org.benf/cfr@0.152");
        assert_eq!(expected.filename(), "cfr-0.152.jar");
        assert_eq!(
            expected.url(false).unwrap().as_str(),
            "https://repo.maven.apache.org/maven2/org/benf/cfr/0.152/cfr-0.152.jar"
        );
    }

    #[test]
    fn janex_requires_an_explicit_type_and_qualifiers_use_purl_encoding() {
        let request = AppRequest::parse("pkg:maven/org.example/tool@1.0%2bbuild?type=janex&repository_url=https://example.org/maven/a%26b&classifier=all").unwrap();
        assert_eq!(
            request.purl(),
            "pkg:maven/org.example/tool@1.0%2Bbuild?classifier=all&repository_url=https:%2F%2Fexample.org%2Fmaven%2Fa%26b%2F&type=janex"
        );
        assert_eq!(request.filename(), "tool-1.0+build-all.janex");
        assert_eq!(
            request.url(false).unwrap().as_str(),
            "https://example.org/maven/a&b/org/example/tool/1.0+build/tool-1.0+build-all.janex"
        );
        assert_eq!(AppRequest::parse(&request.target()).unwrap(), request);
        assert_eq!(AppRequest::parse("maven:org.example:tool@1.0+build[classifier=all,type=janex,repository=https://example.org/maven/a&b]").unwrap(), request);
    }

    #[test]
    fn version_requests_do_not_change_opaque_purl_versions() {
        let latest = AppRequest::parse("pkg:maven/org.example/tool").unwrap();
        assert!(latest.version.is_none());
        assert_eq!(latest, AppRequest::parse("maven:org.example:tool").unwrap());
        let literal = AppRequest::parse("pkg:maven/org.example/tool@latest").unwrap();
        assert_eq!(
            literal,
            AppRequest::parse("maven:org.example:tool@latest").unwrap()
        );
        assert_eq!(literal.version.as_deref(), Some("latest"));
        assert_eq!(literal.filename(), "tool-latest.jar");
        assert_eq!(
            latest.exact("1").unwrap().purl(),
            "pkg:maven/org.example/tool@1"
        );
        assert!(latest.url(false).is_err());
        assert_eq!(
            latest.url(true).unwrap().as_str(),
            "https://repo.maven.apache.org/maven2/org/example/tool/maven-metadata.xml"
        );
    }

    #[test]
    fn jar_launch_options_round_trip_without_becoming_purl_qualifiers() {
        let request = AppRequest::parse("pkg:maven/org.example/tool@1[main-class=example.Main,dependencies=none,command=custom]").unwrap();
        assert_eq!(request.purl(), "pkg:maven/org.example/tool@1");
        assert_eq!(AppRequest::parse(&request.target()).unwrap(), request);
        assert_eq!(
            serde_json::from_value::<AppRequest>(serde_json::to_value(&request).unwrap()).unwrap(),
            request
        );
        for target in [
            "pkg:maven/org.example/tool@1?type=janex[dependencies=none]",
            "maven:org.example:tool@1[dependencies=other]",
            "maven:org.example:tool@1[main-class=A,main-class=B]",
            "pkg:maven/org.example/tool@1?main-class=example.Main",
        ] {
            assert!(AppRequest::parse(target).is_err(), "{target}");
        }
    }

    #[test]
    fn persisted_requests_keep_commands_outside_purls() {
        let request =
            AppRequest::parse("maven:org.example:tool@1.0[type=janex,command=my-tool]").unwrap();
        let stored = serde_json::to_value(&request).unwrap();
        assert_eq!(
            stored,
            serde_json::json!({"purl": "pkg:maven/org.example/tool@1.0?type=janex", "command": "my-tool"})
        );
        assert_eq!(
            serde_json::from_value::<AppRequest>(stored).unwrap(),
            request
        );
        assert_eq!(AppRequest::parse(&request.target()).unwrap(), request);
        let mut renamed = request.clone();
        renamed.command = "another-tool".into();
        assert_eq!(renamed.purl(), request.purl());
        let dotted = AppRequest::parse("maven:org.example:tool.v2@latest[command=tool]").unwrap();
        assert_eq!(AppRequest::parse(&dotted.target()).unwrap(), dotted);
        assert_eq!(
            serde_json::from_value::<AppRequest>(serde_json::to_value(&dotted).unwrap()).unwrap(),
            dotted
        );
    }

    #[test]
    fn ambiguous_unsupported_or_unsafe_targets_fail_before_installation() {
        for input in [
            "pkg:npm/tool@1",
            "pkg:maven/tool@1",
            "pkg:maven/org.example/tool@1#file",
            "pkg:maven/org.example/tool@1?type=jar&type=janex",
            "pkg:maven/org.example/tool@1?TYPE=jar&type=janex",
            "pkg:maven/org.example/tool@1?command=tool",
            "pkg:maven/org.example/tool@1?type=zip",
            "pkg:maven/org.example/tool@1?classifier=%FF",
            "pkg:maven/org.example/tool@1?classifier=%2",
            "pkg:maven/org.example/tool@1?classifier=%xx",
            "pkg:maven/org.example/tool@1?repository_url=https://user:password@example.org/",
            "pkg:maven/org.example/tool@1?repository_url=http://example.org/",
            "pkg:maven/org.example/tool@1?repository_url=https://example.org/%3Ftoken=x",
            "pkg:maven/org.example/tool%2Fother@1",
            "pkg:maven/org..example/tool@1",
            "pkg:maven/org.example/../tool@1",
            "pkg:maven/org.example/tool@1-SNAPSHOT",
            "maven:org.example:tool@1[type=jar,type=janex]",
            "maven:org.example:tool@1?type=jar[type=janex]",
            "maven:org.example:tool@1[command=janex]",
            "maven:org.example:tool@1[command=con]",
            "maven:org.example:tool@1[command=../other]",
            "maven:org.example:tool@1[arch=aarch64]",
        ] {
            assert!(AppRequest::parse(input).is_err(), "{input}");
        }
    }
}

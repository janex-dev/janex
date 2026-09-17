// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Maven application selectors, independent of SDK product and version rules.

use crate::{Result, error::invalid};
use serde::{Deserialize, Serialize};
use url::Url;

/// Default repository for Maven applications.
pub const CENTRAL: &str = "https://repo.maven.apache.org/maven2/";

/// An application source, version requirement, and command name.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppRequest {
    /// Maven group identifier.
    pub group: String,
    /// Maven artifact identifier.
    pub artifact: String,
    /// Exact release, or `latest` to follow repository release metadata.
    pub version: String,
    /// Optional Maven artifact classifier.
    pub classifier: Option<String>,
    /// Download extension: `jar` or `janex`.
    pub kind: String,
    /// Lowercase command name exposed under the Janex bin directory.
    pub command: String,
    /// Canonical HTTPS or absolute file repository URL ending with a slash.
    pub repository: String,
}

impl AppRequest {
    /// Parses `maven:group:artifact[@version][classifier=...,type=...,command=...,repository=...]`.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text
            .strip_prefix("maven:")
            .ok_or_else(|| invalid("expected maven:group:artifact@version"))?;
        let (base, qualifiers) = match text.split_once('[') {
            Some((base, rest)) => (
                base,
                Some(
                    rest.strip_suffix(']')
                        .ok_or_else(|| invalid("unterminated application qualifiers"))?,
                ),
            ),
            None => (text, None),
        };
        let (coordinates, version) = base.split_once('@').unwrap_or((base, "latest"));
        let (group, artifact) = coordinates
            .split_once(':')
            .ok_or_else(|| invalid("Maven application requires group:artifact"))?;
        let mut result = Self {
            group: group.into(),
            artifact: artifact.into(),
            version: version.into(),
            classifier: None,
            kind: "jar".into(),
            command: artifact.to_ascii_lowercase(),
            repository: CENTRAL.into(),
        };
        let mut keys = std::collections::BTreeSet::new();
        if let Some(qualifiers) = qualifiers {
            for item in qualifiers.split(',') {
                let (key, value) = item
                    .split_once('=')
                    .ok_or_else(|| invalid("expected application qualifier key=value"))?;
                if value.is_empty() || !keys.insert(key) {
                    return Err(invalid("empty or duplicate application qualifier"));
                }
                match key {
                    "classifier" => result.classifier = Some(value.into()),
                    "type" => result.kind = value.into(),
                    "command" => result.command = value.into(),
                    "repository" => result.repository = repository(value)?,
                    _ => return Err(invalid(format!("unknown application qualifier: {key}"))),
                }
            }
        }
        result.validate()?;
        Ok(result)
    }

    /// Validates repository paths, supported package kinds, versions, and command filenames.
    pub fn validate(&self) -> Result<()> {
        for part in self.group.split('.') {
            component(part)?;
        }
        component(&self.artifact)?;
        component(&self.version)?;
        if self.version.to_ascii_uppercase().ends_with("-SNAPSHOT")
            || matches!(self.version.as_str(), "LATEST" | "RELEASE")
        {
            return Err(invalid(
                "application versions must be exact releases or latest; snapshots are not supported",
            ));
        }
        if let Some(classifier) = &self.classifier {
            component(classifier)?;
        }
        if !matches!(self.kind.as_str(), "jar" | "janex") {
            return Err(invalid("application type must be jar or janex"));
        }
        command_name(&self.command)?;
        if repository(&self.repository)? != self.repository {
            return Err(invalid("noncanonical application repository URL"));
        }
        Ok(())
    }

    /// Returns a selector preserving the request, repository, and command name.
    pub fn target(&self) -> String {
        let mut values = Vec::new();
        if let Some(classifier) = &self.classifier {
            values.push(format!("classifier={classifier}"));
        }
        if self.kind != "jar" {
            values.push(format!("type={}", self.kind));
        }
        if self.command != self.artifact.to_ascii_lowercase() {
            values.push(format!("command={}", self.command));
        }
        if self.repository != CENTRAL {
            values.push(format!("repository={}", self.repository));
        }
        let mut result = format!("maven:{}:{}@{}", self.group, self.artifact, self.version);
        if !values.is_empty() {
            result.push_str(&format!("[{}]", values.join(",")));
        }
        result
    }

    /// Returns the same request with a concrete release.
    pub(super) fn exact(&self, version: &str) -> Result<Self> {
        let mut exact = self.clone();
        exact.version = version.into();
        exact.validate()?;
        if version == "latest" {
            return Err(invalid(
                "repository did not provide an exact application release",
            ));
        }
        Ok(exact)
    }

    /// Tests whether two versions may share a command without replacing another application's entry.
    pub(super) fn same_product(&self, other: &Self) -> bool {
        self.group == other.group
            && self.artifact == other.artifact
            && self.repository == other.repository
    }

    /// Builds the Maven artifact filename for a concrete release.
    pub(super) fn filename(&self) -> String {
        format!(
            "{}-{}{}.{}",
            self.artifact,
            self.version,
            self.classifier
                .as_ref()
                .map_or(String::new(), |c| format!("-{c}")),
            self.kind
        )
    }

    /// Builds a metadata or artifact URL using Maven repository layout.
    pub(super) fn url(&self, metadata: bool) -> Result<Url> {
        self.validate()?;
        let mut url =
            Url::parse(&self.repository).map_err(|_| invalid("invalid repository URL"))?;
        let filename = self.filename();
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
                path.push(&self.version).push(&filename);
            }
        }
        Ok(url)
    }
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
        return Err(invalid("invalid Maven application coordinate component"));
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
            "invalid application command name; use a lowercase command qualifier",
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
        || value.contains(',')
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
    fn selectors_preserve_maven_versions_classifiers_and_repository_paths() {
        let request = AppRequest::parse("maven:org.example:tool@1.0.Final[classifier=all,type=janex,command=my-tool,repository=https://example.org/maven]").unwrap();
        assert_eq!(request.version, "1.0.Final");
        assert_eq!(
            request.url(false).unwrap().as_str(),
            "https://example.org/maven/org/example/tool/1.0.Final/tool-1.0.Final-all.janex"
        );
        assert_eq!(AppRequest::parse(&request.target()).unwrap(), request);
        assert_eq!(
            AppRequest::parse("maven:org.example:tool").unwrap().version,
            "latest"
        );
        assert_eq!(
            AppRequest::parse("maven:org.example:tool@1")
                .unwrap()
                .version,
            "1"
        );
        assert_eq!(request.exact("2.0").unwrap().command, "my-tool");
    }

    #[test]
    fn selectors_reject_unknown_qualifiers_unsafe_paths_and_mutable_snapshots() {
        for target in [
            "maven:org..example:tool@1",
            "maven:org.example:../tool@1",
            "maven:org.example:tool@1-SNAPSHOT",
            "maven:org.example:tool@1[command=janex]",
            "maven:org.example:tool@1[command=con]",
            "maven:org.example:tool@1[command=../other]",
            "maven:org.example:tool@1[arch=aarch64]",
            "maven:org.example:tool@1[type=zip]",
            "maven:org.example:tool@1[type=jar,type=janex]",
            "maven:org.example:tool@1[repository=http://example.org/]",
            "maven:org.example:tool@1[repository=https://user:password@example.org/]",
            "maven:org.example:tool@1[repository=https://example.org/?token=value]",
        ] {
            assert!(AppRequest::parse(target).is_err(), "{target}");
        }
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Maven runtime dependency collection, independent of downloading and application installation.

mod model;
#[cfg(test)]
mod tests;

use crate::{Error, Result, error::invalid};
use model::{Dependency, Model, Models};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use url::Url;

/// Default Maven repository, omitted from canonical package identities.
pub const CENTRAL: &str = "https://repo.maven.apache.org/maven2/";

/// Escapes reserved characters in canonical PURL components.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'.')
    .remove(b'-')
    .remove(b'_')
    .remove(b'~')
    .remove(b':');

/// Exact Maven artifact coordinates, after type-to-extension and default-classifier mapping.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Artifact {
    /// Maven group identifier.
    pub group: String,
    /// Maven artifact identifier.
    pub name: String,
    /// Exact release version; ranges and snapshots are unsupported.
    pub version: String,
    /// Artifact extension, normally `jar` or `pom`.
    pub extension: String,
    /// Optional classifier, without the leading hyphen.
    pub classifier: Option<String>,
}

impl Artifact {
    /// Rejects non-release coordinates and path separators before constructing repository URLs.
    pub fn validate(&self) -> Result<()> {
        for part in self.group.split('.') {
            component(part)?;
        }
        component(&self.name)?;
        component(&self.version)?;
        component(&self.extension)?;
        if let Some(classifier) = &self.classifier {
            component(classifier)?;
        }
        if self.version.ends_with("-SNAPSHOT")
            || matches!(self.version.as_str(), "LATEST" | "RELEASE")
        {
            return Err(Error::Unsupported(
                "Maven dependency resolution requires exact release versions".into(),
            ));
        }
        Ok(())
    }

    /// Returns the original Maven filename.
    pub fn filename(&self) -> String {
        format!(
            "{}-{}{}.{}",
            self.name,
            self.version,
            self.classifier
                .as_ref()
                .map_or(String::new(), |v| format!("-{v}")),
            self.extension
        )
    }

    /// Returns a URL for validated coordinates and a hierarchical repository URL.
    pub fn url(&self, repository: &Url) -> Url {
        let mut url = repository.clone();
        url.path_segments_mut()
            .expect("Maven repository URL")
            .pop_if_empty()
            .extend(self.group.split('.'))
            .push(&self.name)
            .push(&self.version)
            .push(&self.filename());
        url
    }

    /// Returns a canonical PURL, including the repository when it differs from Maven Central.
    pub fn purl(&self, repository: &Url) -> String {
        let encode = |v: &str| utf8_percent_encode(v, COMPONENT).to_string();
        let mut qualifiers = Vec::new();
        if let Some(classifier) = &self.classifier {
            qualifiers.push(format!("classifier={}", encode(classifier)));
        }
        if repository.as_str() != CENTRAL {
            qualifiers.push(format!("repository_url={}", encode(repository.as_str())));
        }
        if self.extension != "jar" {
            qualifiers.push(format!("type={}", encode(&self.extension)));
        }
        let mut result = format!(
            "pkg:maven/{}/{}@{}",
            encode(&self.group),
            encode(&self.name),
            encode(&self.version),
        );
        if !qualifiers.is_empty() {
            result.push('?');
            result.push_str(&qualifiers.join("&"));
        }
        result
    }

    /// Returns the unclassified POM for these coordinates.
    fn pom(&self) -> Self {
        Self {
            extension: "pom".into(),
            classifier: None,
            ..self.clone()
        }
    }

    /// Returns the conflict identity, excluding version.
    fn key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.group,
            self.name,
            self.extension,
            self.classifier.as_deref().unwrap_or("")
        )
    }
}

/// Resolves the application's runtime classpath in nearest-first, declaration order.
///
/// `fetch` reads POM bytes only; artifact downloads remain the caller's responsibility.
/// The application is the root project: its direct optional dependencies are included, while
/// optional dependencies of libraries are not inherited. The result excludes the root artifact.
/// All POMs use `repository`; POM repository declarations do not add download locations.
/// Version ranges, snapshots, relocations and environment-activated dependency profiles fail.
pub fn runtime_dependencies(
    root: &Artifact,
    repository: &Url,
    fetch: impl FnMut(&Url) -> Result<Vec<u8>>,
) -> Result<Vec<Artifact>> {
    root.validate()?;
    if !matches!(repository.scheme(), "https" | "file") || repository.cannot_be_a_base() {
        return Err(invalid(
            "Maven resolution requires an HTTPS or file repository",
        ));
    }
    let mut models = Models::new(repository, fetch);
    let model = models.get(root)?;
    let mut queue = VecDeque::new();
    enqueue(&mut queue, &model, &BTreeSet::new(), None, &model)?;
    let mut winners = BTreeMap::new();
    let mut selected: Vec<Selected> = Vec::new();
    let mut edges = 0_u32;
    while let Some(edge) = queue.pop_front() {
        edges += 1;
        if edges > 100_000 {
            return Err(invalid("Maven dependency graph exceeds edge limit"));
        }
        let artifact = edge.dependency.artifact()?;
        let key = artifact.key();
        if key == root.key() {
            continue;
        }
        let scope = Scope::parse(edge.dependency.scope.as_deref())?;
        // Maven retains the winning path's subtree, even when another path requests the same version.
        if let Some(&index) = winners.get(&key) {
            selected[index as usize].paths.push((edge.parent, scope));
            continue;
        }
        let index = selected.len() as u32;
        winners.insert(key, index);
        selected.push(Selected {
            artifact: artifact.clone(),
            on_classpath: edge.dependency.on_classpath(),
            scope: derive(edge.parent.map(|i| selected[i as usize].scope), scope),
            paths: vec![(edge.parent, scope)],
        });
        let child = models.get(&artifact)?;
        enqueue(&mut queue, &child, &edge.exclusions, Some(index), &model)?;
    }
    // Scope mediation can promote a transitive winner reached through a provided/test branch
    // when another surviving branch needs it at runtime. Propagate those promotions to children.
    let mut dependents = vec![Vec::new(); selected.len()];
    let mut pending = VecDeque::new();
    for (index, node) in selected.iter().enumerate() {
        pending.push_back(index as u32);
        for (parent, _) in &node.paths {
            if let Some(parent) = parent {
                dependents[*parent as usize].push(index as u32);
            }
        }
    }
    while let Some(index) = pending.pop_front() {
        let node = &selected[index as usize];
        let scope = if node.paths[0].0.is_none() {
            node.paths[0].1
        } else {
            node.paths
                .iter()
                .map(|(parent, scope)| derive(parent.map(|i| selected[i as usize].scope), *scope))
                .max()
                .unwrap()
        };
        if node.scope != scope {
            selected[index as usize].scope = scope;
            pending.extend(&dependents[index as usize]);
        }
    }
    Ok(selected
        .into_iter()
        .filter(|n| n.on_classpath && matches!(n.scope, Scope::Compile | Scope::Runtime))
        .map(|n| n.artifact)
        .collect())
}

/// Scope precedence used by Maven's Java scope mediation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Scope {
    /// Available only during tests.
    Test,
    /// Supplied by the runtime environment.
    Provided,
    /// Required for execution.
    Runtime,
    /// Required for compilation and execution.
    Compile,
}

impl Scope {
    /// Parses scopes supported by published runtime models.
    fn parse(value: Option<&str>) -> Result<Self> {
        match value.unwrap_or("compile") {
            "compile" => Ok(Self::Compile),
            "runtime" => Ok(Self::Runtime),
            "provided" => Ok(Self::Provided),
            "test" => Ok(Self::Test),
            scope => Err(Error::Unsupported(format!(
                "unsupported Maven dependency scope: {scope}"
            ))),
        }
    }
}

/// Derives an edge's scope from its selected parent's effective scope.
fn derive(parent: Option<Scope>, scope: Scope) -> Scope {
    match parent {
        None | Some(Scope::Compile) => scope,
        Some(parent) => parent,
    }
}

/// One candidate edge and the exclusion policy inherited along its path.
struct Edge {
    /// Effective declared dependency.
    dependency: Dependency,
    /// Exclusions applying to this dependency's descendants.
    exclusions: BTreeSet<(String, String)>,
    /// Selected parent index, or none for a direct application dependency.
    parent: Option<u32>,
}

/// A nearest-path winner and the surviving paths used to mediate its scope.
struct Selected {
    /// Chosen version and artifact handler result.
    artifact: Artifact,
    /// Whether the artifact handler exposes the archive on a Java classpath.
    on_classpath: bool,
    /// Effective scope after mediation.
    scope: Scope,
    /// Parent indices and declared scopes of all surviving occurrences.
    paths: Vec<(Option<u32>, Scope)>,
}

/// Queues runtime edges with inherited exclusions and root dependency management.
fn enqueue(
    queue: &mut VecDeque<Edge>,
    model: &Model,
    exclusions: &BTreeSet<(String, String)>,
    parent: Option<u32>,
    root_model: &Model,
) -> Result<()> {
    for dependency in &model.dependencies {
        let mut dependency = dependency.clone();
        if parent.is_some() {
            if let Some(managed) = root_model
                .managed
                .iter()
                .find(|m| m.key() == dependency.key())
            {
                dependency.manage(managed, true);
            }
            if dependency.optional.as_deref() == Some("true") {
                continue;
            }
        }
        let scope = Scope::parse(dependency.scope.as_deref())?;
        if parent.is_some() && matches!(scope, Scope::Test | Scope::Provided) {
            continue;
        }
        if exclusions.iter().any(|(group, name)| {
            (group == "*" || group == &dependency.group)
                && (name == "*" || name == &dependency.name)
        }) {
            continue;
        }
        let mut child_exclusions = exclusions.clone();
        child_exclusions.extend(dependency.exclusions.iter().cloned());
        queue.push_back(Edge {
            dependency,
            exclusions: child_exclusions,
            parent,
        });
    }
    Ok(())
}

/// Checks coordinate components before they are used as path segments.
fn component(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 160
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"._-+".contains(&v))
        || value.ends_with('.')
    {
        return Err(Error::Unsupported(format!(
            "unsupported Maven coordinate component: {value}"
        )));
    }
    Ok(())
}

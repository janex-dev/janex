// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Effective dependency models for published Maven POMs.

use super::Artifact;
use crate::{Error, Result, error::invalid};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

/// A dependency before artifact-handler mapping.
#[derive(Clone, Debug, Default)]
pub(super) struct Dependency {
    /// Group identifier.
    pub group: String,
    /// Artifact identifier.
    pub name: String,
    /// Declared or managed version, which may contain property expressions before interpolation.
    version: Option<String>,
    /// Maven dependency type; absent means `jar`.
    kind: Option<String>,
    /// Explicit classifier, or the artifact handler's default.
    classifier: Option<String>,
    /// Maven dependency scope.
    pub scope: Option<String>,
    /// Whether downstream consumers must opt into this dependency.
    pub optional: Option<String>,
    /// Excluded group/artifact patterns for this edge and its descendants.
    pub exclusions: BTreeSet<(String, String)>,
}

impl Dependency {
    /// Whether Maven's built-in artifact handler adds this type to the Java classpath.
    pub fn on_classpath(&self) -> bool {
        !matches!(
            self.kind.as_deref(),
            Some("pom" | "java-source" | "javadoc")
        )
    }

    /// Returns the dependency-management identity with Maven's default type.
    pub fn key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.group,
            self.name,
            self.kind.as_deref().unwrap_or("jar"),
            self.classifier.as_deref().unwrap_or("")
        )
    }

    /// Applies managed fields, overriding transitive versions and scopes when requested.
    pub fn manage(&mut self, managed: &Self, override_existing: bool) {
        if (override_existing || self.version.is_none()) && managed.version.is_some() {
            self.version = managed.version.clone();
        }
        if (override_existing || self.scope.is_none()) && managed.scope.is_some() {
            self.scope = managed.scope.clone();
        }
        // Maven does not inject optionality from dependencyManagement.
        if override_existing || self.exclusions.is_empty() {
            self.exclusions.extend(managed.exclusions.iter().cloned());
        }
    }

    /// Maps built-in Maven types to classpath artifacts without loading build extensions.
    pub fn artifact(&self) -> Result<Artifact> {
        let (extension, classifier) = match self.kind.as_deref().unwrap_or("jar") {
            "jar" | "bundle" | "maven-plugin" | "ejb" => ("jar", None),
            "test-jar" => ("jar", Some("tests")),
            "ejb-client" => ("jar", Some("client")),
            "java-source" => ("jar", Some("sources")),
            "javadoc" => ("jar", Some("javadoc")),
            "pom" => ("pom", None),
            kind => {
                return Err(Error::Unsupported(format!(
                    "unsupported Maven dependency type: {kind}"
                )));
            }
        };
        let result = Artifact {
            group: self.group.clone(),
            name: self.name.clone(),
            version: self.version.clone().ok_or_else(|| {
                invalid(format!(
                    "Maven dependency has no version: {}:{}",
                    self.group, self.name
                ))
            })?,
            extension: extension.into(),
            classifier: self
                .classifier
                .clone()
                .or_else(|| classifier.map(String::from)),
        };
        result.validate()?;
        Ok(result)
    }

    /// Expands expressions in all dependency fields and exclusion patterns.
    fn interpolate(&mut self, properties: &BTreeMap<String, String>) -> Result<()> {
        self.group = expand(&self.group, properties)?;
        self.name = expand(&self.name, properties)?;
        for field in [
            &mut self.version,
            &mut self.kind,
            &mut self.classifier,
            &mut self.scope,
            &mut self.optional,
        ]
        .into_iter()
        .flatten()
        {
            *field = expand(field, properties)?;
        }
        self.exclusions = self
            .exclusions
            .iter()
            .map(|(g, a)| Ok((expand(g, properties)?, expand(a, properties)?)))
            .collect::<Result<_>>()?;
        if self
            .optional
            .as_deref()
            .is_some_and(|v| v != "true" && v != "false")
        {
            return Err(invalid("invalid Maven optional value"));
        }
        Ok(())
    }
}

/// Runtime-relevant fields used during inheritance and effective-model construction.
#[derive(Clone, Debug, Default)]
pub(super) struct Model {
    /// Inherited project properties.
    properties: BTreeMap<String, String>,
    /// Dependency-management entries in precedence order.
    pub managed: Vec<Dependency>,
    /// Declared dependencies in declaration order.
    pub dependencies: Vec<Dependency>,
}

/// Per-resolution POM cache and recursion guard.
pub(super) struct Models<'a, F> {
    /// Explicit repository for the whole graph.
    repository: &'a Url,
    /// POM reader supplied by the caller.
    fetch: F,
    /// Completed models, keyed by unclassified POM coordinates.
    cache: BTreeMap<Artifact, Model>,
    /// Models after inheritance but before interpolation and BOM expansion.
    inherited: BTreeMap<Artifact, Model>,
    /// Parent models currently being inherited.
    parents: BTreeSet<Artifact>,
    /// Parent/BOM imports currently being evaluated.
    active: BTreeSet<Artifact>,
}

impl<'a, F: FnMut(&Url) -> Result<Vec<u8>>> Models<'a, F> {
    /// Creates a model cache for one dependency collection.
    pub fn new(repository: &'a Url, fetch: F) -> Self {
        Self {
            repository,
            fetch,
            cache: BTreeMap::new(),
            inherited: BTreeMap::new(),
            parents: BTreeSet::new(),
            active: BTreeSet::new(),
        }
    }

    /// Loads a published POM and evaluates its parent and BOM imports once.
    pub fn get(&mut self, artifact: &Artifact) -> Result<Model> {
        let pom = artifact.pom();
        if let Some(model) = self.cache.get(&pom) {
            return Ok(model.clone());
        }
        if self.cache.len() >= 10_000 || self.active.len() >= 64 {
            return Err(invalid("Maven model count or nesting limit exceeded"));
        }
        if !self.active.insert(pom.clone()) {
            return Err(invalid(format!(
                "cyclic Maven parent or BOM import: {}",
                pom.filename()
            )));
        }
        let result = self.effective(&pom);
        self.active.remove(&pom);
        let model = result?;
        self.cache.insert(pom, model.clone());
        Ok(model)
    }

    /// Caches raw inheritance so child properties can override inherited dependency expressions.
    fn inherit(&mut self, pom: &Artifact) -> Result<Model> {
        if let Some(model) = self.inherited.get(pom) {
            return Ok(model.clone());
        }
        if self.parents.len() >= 64 || self.inherited.len() >= 10_000 {
            return Err(invalid(
                "Maven parent model count or nesting limit exceeded",
            ));
        }
        if !self.parents.insert(pom.clone()) {
            return Err(invalid("cyclic Maven parent"));
        }
        let result = self.load(pom);
        self.parents.remove(pom);
        let model = result?;
        self.inherited.insert(pom.clone(), model.clone());
        Ok(model)
    }

    /// Reads only model data; build plugins and extensions are never executed.
    fn load(&mut self, pom: &Artifact) -> Result<Model> {
        let bytes = (self.fetch)(&pom.url(self.repository))
            .map_err(|e| invalid(format!("cannot read Maven POM {}: {e}", pom.filename())))?;
        let text = std::str::from_utf8(&bytes).map_err(|_| invalid("Maven POM is not UTF-8"))?;
        let document = roxmltree::Document::parse_with_options(
            text,
            roxmltree::ParsingOptions {
                allow_dtd: false,
                nodes_limit: 100_000,
                ..Default::default()
            },
        )
        .map_err(|e| invalid(format!("invalid Maven POM {}: {e}", pom.filename())))?;
        let root = document.root_element();
        if !root.has_tag_name("project") || value(root, "modelVersion").as_deref() != Some("4.0.0")
        {
            return Err(Error::Unsupported(
                "Maven POM requires modelVersion 4.0.0".into(),
            ));
        }
        if child(root, "distributionManagement")
            .and_then(|n| child(n, "relocation"))
            .is_some()
        {
            return Err(Error::Unsupported(
                "Maven POM relocations are not yet supported".into(),
            ));
        }
        let parent = if let Some(node) = child(root, "parent") {
            let parent = Artifact {
                group: required(node, "groupId")?,
                name: required(node, "artifactId")?,
                version: required(node, "version")?,
                extension: "pom".into(),
                classifier: None,
            };
            parent.validate()?;
            Some(parent)
        } else {
            None
        };
        let mut model = if let Some(parent) = &parent {
            self.inherit(parent)?
        } else {
            Model::default()
        };
        let inherited_dependencies = std::mem::take(&mut model.dependencies);
        let inherited_managed = std::mem::take(&mut model.managed);
        properties(root, &mut model.properties);
        let mut sections = vec![root];
        if let Some(profiles) = child(root, "profiles") {
            let affects_dependencies = |profile: roxmltree::Node<'_, '_>| {
                ["dependencies", "dependencyManagement", "properties"]
                    .iter()
                    .any(|name| child(profile, name).is_some())
            };
            let has_default_dependencies = profiles.children().any(|profile| {
                profile.has_tag_name("profile")
                    && affects_dependencies(profile)
                    && child(profile, "activation")
                        .and_then(|a| value(a, "activeByDefault"))
                        .as_deref()
                        == Some("true")
            });
            for profile in profiles.children().filter(|n| n.has_tag_name("profile")) {
                let relevant = affects_dependencies(profile);
                if !relevant && !has_default_dependencies {
                    continue;
                }
                let activation = child(profile, "activation");
                // Even a build-only profile can disable a dependency-bearing default profile.
                if activation.is_some_and(|a| {
                    a.children()
                        .any(|n| n.is_element() && !n.has_tag_name("activeByDefault"))
                }) {
                    return Err(Error::Unsupported(format!(
                        "environment-activated Maven dependency profile is unsupported: {}",
                        value(profile, "id").unwrap_or_default()
                    )));
                }
                if relevant
                    && activation
                        .and_then(|a| value(a, "activeByDefault"))
                        .as_deref()
                        == Some("true")
                {
                    properties(profile, &mut model.properties);
                    sections.push(profile);
                }
            }
        }
        for (key, val) in [
            ("groupId", &pom.group),
            ("artifactId", &pom.name),
            ("version", &pom.version),
        ] {
            model
                .properties
                .insert(format!("project.{key}"), val.clone());
            model.properties.insert(format!("pom.{key}"), val.clone());
        }
        if let Some(parent) = &parent {
            for (key, val) in [
                ("groupId", &parent.group),
                ("artifactId", &parent.name),
                ("version", &parent.version),
            ] {
                model
                    .properties
                    .insert(format!("project.parent.{key}"), val.clone());
                model
                    .properties
                    .insert(format!("parent.{key}"), val.clone());
            }
        }
        for (key, expected) in [
            ("groupId", &pom.group),
            ("artifactId", &pom.name),
            ("version", &pom.version),
        ] {
            let actual = value(root, key)
                .or_else(|| match key {
                    "groupId" => parent.as_ref().map(|p| p.group.clone()),
                    "version" => parent.as_ref().map(|p| p.version.clone()),
                    _ => None,
                })
                .ok_or_else(|| invalid(format!("Maven POM has no {key}")))?;
            if expand(&actual, &model.properties)? != *expected {
                return Err(invalid(format!(
                    "Maven POM coordinates do not match {}",
                    pom.filename()
                )));
            }
        }
        let mut declarations = Vec::new();
        let mut dependencies = Vec::new();
        for section in sections {
            declarations = merge(
                read_dependencies(child(section, "dependencyManagement"))?,
                declarations,
            );
            dependencies = merge(read_dependencies(Some(section))?, dependencies);
        }
        model.managed = merge(declarations, inherited_managed);
        model.dependencies = merge(dependencies, inherited_dependencies);
        Ok(model)
    }

    /// Interpolates the inherited model, imports BOMs, and applies dependency management.
    fn effective(&mut self, pom: &Artifact) -> Result<Model> {
        let mut model = self.inherit(pom)?;
        for dependency in model.dependencies.iter_mut().chain(&mut model.managed) {
            dependency.interpolate(&model.properties)?;
        }
        // Explicit management (including inherited management) dominates imported BOMs.
        let mut imports = Vec::new();
        for dependency in &model.managed {
            if dependency.scope.as_deref() == Some("import") {
                if dependency.kind.as_deref() != Some("pom") {
                    return Err(invalid("Maven BOM imports require type pom"));
                }
                imports.push(dependency.artifact()?);
            }
        }
        model
            .managed
            .retain(|d| d.scope.as_deref() != Some("import"));
        for import in imports {
            let imported = self.get(&import)?;
            for dependency in imported.managed {
                if !model.managed.iter().any(|d| d.key() == dependency.key()) {
                    model.managed.push(dependency);
                }
            }
        }
        for dependency in &mut model.dependencies {
            if let Some(managed) = model.managed.iter().find(|m| m.key() == dependency.key()) {
                dependency.manage(managed, false);
            }
        }
        Ok(model)
    }
}

/// Returns a direct child element by its local name.
fn child<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

/// Reads a trimmed scalar field.
fn value(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    child(node, name)
        .and_then(|n| n.text())
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Requires a nonempty model field.
fn required(node: roxmltree::Node<'_, '_>, name: &str) -> Result<String> {
    value(node, name).ok_or_else(|| invalid(format!("Maven POM has no {name}")))
}

/// Adds one model or active profile's properties to the inherited property map.
fn properties(node: roxmltree::Node<'_, '_>, result: &mut BTreeMap<String, String>) {
    if let Some(properties) = child(node, "properties") {
        for property in properties.children().filter(|n| n.is_element()) {
            result.insert(
                property.tag_name().name().into(),
                property.text().unwrap_or("").trim().into(),
            );
        }
    }
}

/// Reads raw declarations, retaining property expressions until inheritance is complete.
fn read_dependencies(node: Option<roxmltree::Node<'_, '_>>) -> Result<Vec<Dependency>> {
    let Some(dependencies) = node.and_then(|n| child(n, "dependencies")) else {
        return Ok(Vec::new());
    };
    dependencies
        .children()
        .filter(|n| n.has_tag_name("dependency"))
        .map(|n| {
            let mut dependency = Dependency {
                group: required(n, "groupId")?,
                name: required(n, "artifactId")?,
                version: value(n, "version"),
                kind: value(n, "type"),
                classifier: value(n, "classifier"),
                scope: value(n, "scope"),
                optional: value(n, "optional"),
                exclusions: BTreeSet::new(),
            };
            if let Some(exclusions) = child(n, "exclusions") {
                for exclusion in exclusions
                    .children()
                    .filter(|n| n.has_tag_name("exclusion"))
                {
                    dependency.exclusions.insert((
                        required(exclusion, "groupId")?,
                        required(exclusion, "artifactId")?,
                    ));
                }
            }
            Ok(dependency)
        })
        .collect()
}

/// Merges dependency lists with dominant declarations first and recessive missing fields filled in.
fn merge(mut dominant: Vec<Dependency>, recessive: Vec<Dependency>) -> Vec<Dependency> {
    for dependency in recessive {
        if let Some(existing) = dominant.iter_mut().find(|d| d.key() == dependency.key()) {
            existing.manage(&dependency, false);
            if existing.optional.is_none() {
                existing.optional = dependency.optional;
            }
            existing.exclusions.extend(dependency.exclusions);
        } else {
            dominant.push(dependency);
        }
    }
    dominant
}

/// Expands nested project properties, rejecting unresolved or cyclic expressions.
fn expand(value: &str, properties: &BTreeMap<String, String>) -> Result<String> {
    let mut result = value.to_owned();
    for _ in 0..64 {
        let Some(start) = result.find("${") else {
            return Ok(result);
        };
        let end = result[start + 2..]
            .find('}')
            .map(|i| i + start + 2)
            .ok_or_else(|| invalid("unterminated Maven property"))?;
        let name = &result[start + 2..end];
        let replacement = properties
            .get(name)
            .ok_or_else(|| Error::Unsupported(format!("unresolved Maven property: {name}")))?;
        result.replace_range(start..=end, replacement);
        if result.len() > 64 * 1024 {
            return Err(invalid("Maven property expansion exceeds limit"));
        }
    }
    Err(invalid("cyclic or excessively nested Maven property"))
}

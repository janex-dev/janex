// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::collections::BTreeMap;

/// Returns one fixture coordinate.
fn artifact(name: &str, version: &str) -> Artifact {
    Artifact {
        group: "org.example".into(),
        name: name.into(),
        version: version.into(),
        extension: "jar".into(),
        classifier: None,
    }
}

/// Builds a Maven dependency declaration, allowing optional fields to be supplied separately.
fn dependency(name: &str, version: Option<&str>, fields: &str) -> String {
    format!(
        "<dependency><groupId>org.example</groupId><artifactId>{name}</artifactId>{}{fields}</dependency>",
        version.map_or(String::new(), |v| format!("<version>{v}</version>"))
    )
}

/// In-memory published POM repository; unexpected requests fail the test.
#[derive(Default)]
struct Repository(BTreeMap<String, Vec<u8>>);

impl Repository {
    /// Adds a namespace-qualified published POM.
    fn add(&mut self, name: &str, version: &str, fields: &str) {
        self.0.insert(artifact(name, version).pom().filename(), format!("<project xmlns=\"http://maven.apache.org/POM/4.0.0\"><modelVersion>4.0.0</modelVersion><groupId>org.example</groupId><artifactId>{name}</artifactId><version>{version}</version>{fields}</project>").into_bytes());
    }

    /// Resolves the fixture application without network access.
    fn resolve(&self) -> Result<Vec<Artifact>> {
        runtime_dependencies(
            &artifact("app", "1"),
            &Url::parse("https://example.org/repository/").unwrap(),
            |url| {
                let name = url.path_segments().unwrap().next_back().unwrap();
                self.0
                    .get(name)
                    .cloned()
                    .ok_or_else(|| invalid(format!("unexpected POM: {name}")))
            },
        )
    }
}

#[test]
fn inherited_expressions_use_child_properties_and_project_coordinates() {
    let mut repo = Repository::default();
    repo.add("parent", "2", &format!("<properties><lib.version>2</lib.version></properties><dependencyManagement><dependencies>{}</dependencies></dependencyManagement><dependencies>{}{}</dependencies>", dependency("managed", Some("${lib.version}"), ""), dependency("inherited", Some("${lib.version}"), ""), dependency("selfversion", Some("${project.version}"), "")));
    repo.add("app", "1", &format!("<parent><groupId>org.example</groupId><artifactId>parent</artifactId><version>2</version></parent><properties><lib.version>3</lib.version></properties><dependencies>{}{}</dependencies>", dependency("direct", Some("1"), ""), dependency("managed", None, "")));
    for (name, version) in [
        ("direct", "1"),
        ("managed", "3"),
        ("inherited", "3"),
        ("selfversion", "1"),
    ] {
        repo.add(name, version, "");
    }
    assert_eq!(
        repo.resolve().unwrap(),
        vec![
            artifact("direct", "1"),
            artifact("managed", "3"),
            artifact("inherited", "3"),
            artifact("selfversion", "1")
        ]
    );
}

#[test]
fn parent_boms_properties_and_root_management_determine_runtime_versions() {
    let mut repo = Repository::default();
    repo.add("parent", "1", &format!("<properties><bom.version>2</bom.version><managed.version>3</managed.version></properties><dependencyManagement><dependencies>{}</dependencies></dependencyManagement>", dependency("managed", Some("${managed.version}"), "")));
    repo.add(
        "bom",
        "2",
        &format!(
            "<dependencyManagement><dependencies>{}{}</dependencies></dependencyManagement>",
            dependency("library", Some("4"), ""),
            dependency("unused", Some("99"), "")
        ),
    );
    repo.add("app", "1", &format!("<parent><groupId>org.example</groupId><artifactId>parent</artifactId><version>1</version></parent><dependencyManagement><dependencies>{}</dependencies></dependencyManagement><dependencies>{}</dependencies>", dependency("bom", Some("${bom.version}"), "<type>pom</type><scope>import</scope>"), dependency("library", None, "")));
    repo.add(
        "library",
        "4",
        &format!(
            "<dependencies>{}</dependencies>",
            dependency("managed", Some("1"), "<scope>runtime</scope>")
        ),
    );
    repo.add("managed", "3", "");
    assert_eq!(
        repo.resolve().unwrap(),
        vec![artifact("library", "4"), artifact("managed", "3")]
    );
}

#[test]
fn nearest_and_first_declarations_win_without_loading_losing_subtrees() {
    let mut repo = Repository::default();
    repo.add(
        "app",
        "1",
        &format!(
            "<dependencies>{}{}{}</dependencies>",
            dependency("left", Some("1"), ""),
            dependency("right", Some("1"), ""),
            dependency("near", Some("9"), "")
        ),
    );
    repo.add(
        "left",
        "1",
        &format!(
            "<dependencies>{}{}</dependencies>",
            dependency("shared", Some("2"), ""),
            dependency("near", Some("1"), "")
        ),
    );
    repo.add(
        "right",
        "1",
        &format!(
            "<dependencies>{}</dependencies>",
            dependency("shared", Some("3"), "")
        ),
    );
    repo.add("near", "9", "");
    repo.add(
        "shared",
        "2",
        &format!(
            "<dependencies>{}</dependencies>",
            dependency("left", Some("1"), "")
        ),
    );
    assert_eq!(
        repo.resolve().unwrap(),
        vec![
            artifact("left", "1"),
            artifact("right", "1"),
            artifact("near", "9"),
            artifact("shared", "2")
        ]
    );
}

#[test]
fn exclusions_are_per_path_and_optional_edges_respect_the_application_root() {
    let mut repo = Repository::default();
    let exclude = "<exclusions><exclusion><groupId>org.example</groupId><artifactId>leaf</artifactId></exclusion></exclusions>";
    repo.add(
        "app",
        "1",
        &format!(
            "<dependencies>{}{}{}{}{}</dependencies>",
            dependency("left", Some("1"), exclude),
            dependency("right", Some("1"), ""),
            dependency("optional-root", Some("1"), "<optional>true</optional>"),
            dependency("test", Some("1"), "<scope>test</scope>"),
            dependency("provided", Some("1"), "<scope>provided</scope>")
        ),
    );
    repo.add(
        "left",
        "1",
        &format!(
            "<dependencies>{}</dependencies>",
            dependency("shared", Some("1"), "")
        ),
    );
    repo.add(
        "right",
        "1",
        &format!(
            "<dependencies>{}</dependencies>",
            dependency("shared", Some("1"), "")
        ),
    );
    repo.add("optional-root", "1", "");
    repo.add(
        "shared",
        "1",
        &format!(
            "<dependencies>{}{}</dependencies>",
            dependency("leaf", Some("1"), "<scope>runtime</scope>"),
            dependency("optional-library", Some("1"), "<optional>true</optional>")
        ),
    );
    repo.add("leaf", "1", "");
    repo.add("test", "1", "");
    repo.add("provided", "1", "");
    assert_eq!(
        repo.resolve().unwrap(),
        vec![
            artifact("left", "1"),
            artifact("right", "1"),
            artifact("optional-root", "1"),
            artifact("shared", "1")
        ]
    );
    repo.add(
        "right",
        "1",
        &format!(
            "<dependencies>{}{}</dependencies>",
            dependency("shared", Some("1"), ""),
            dependency("leaf", Some("1"), "")
        ),
    );
    assert_eq!(
        repo.resolve().unwrap(),
        vec![
            artifact("left", "1"),
            artifact("right", "1"),
            artifact("optional-root", "1"),
            artifact("shared", "1"),
            artifact("leaf", "1")
        ]
    );
}

#[test]
fn first_bom_wins_and_local_management_does_not_inherit_bom_scope_or_optional() {
    let mut repo = Repository::default();
    repo.add(
        "first",
        "1",
        &format!(
            "<dependencyManagement><dependencies>{}{}</dependencies></dependencyManagement>",
            dependency("library", Some("1"), ""),
            dependency("managed", Some("4"), "<scope>provided</scope>")
        ),
    );
    repo.add(
        "second",
        "1",
        &format!(
            "<dependencyManagement><dependencies>{}</dependencies></dependencyManagement>",
            dependency("library", Some("2"), "")
        ),
    );
    repo.add("app", "1", &format!("<dependencyManagement><dependencies>{}{}{}</dependencies></dependencyManagement><dependencies>{}{}</dependencies>", dependency("first", Some("1"), "<type>pom</type><scope>import</scope>"), dependency("second", Some("1"), "<type>pom</type><scope>import</scope>"), dependency("managed", Some("3"), "<optional>true</optional>"), dependency("library", None, ""), dependency("managed", None, "")));
    repo.add("library", "1", "");
    repo.add("managed", "3", "");
    assert_eq!(
        repo.resolve().unwrap(),
        vec![artifact("library", "1"), artifact("managed", "3")]
    );
    assert_eq!(
        artifact("library", "1").purl(&Url::parse(CENTRAL).unwrap()),
        "pkg:maven/org.example/library@1"
    );
}

#[test]
fn direct_provided_versions_and_scope_promotion_match_maven_mediation() {
    let mut repo = Repository::default();
    repo.add(
        "app",
        "1",
        &format!(
            "<dependencies>{}{}</dependencies>",
            dependency("left", Some("1"), ""),
            dependency("shared", Some("2"), "<scope>provided</scope>")
        ),
    );
    repo.add(
        "left",
        "1",
        &format!(
            "<dependencies>{}</dependencies>",
            dependency("shared", Some("1"), "")
        ),
    );
    repo.add("shared", "2", "");
    assert_eq!(repo.resolve().unwrap(), vec![artifact("left", "1")]);
    repo.add(
        "app",
        "1",
        &format!(
            "<dependencies>{}{}</dependencies>",
            dependency("shared", Some("2"), "<scope>provided</scope>"),
            dependency("left", Some("1"), "")
        ),
    );
    repo.add(
        "shared",
        "2",
        &format!(
            "<dependencies>{}</dependencies>",
            dependency("leaf", Some("1"), "")
        ),
    );
    repo.add(
        "left",
        "1",
        &format!(
            "<dependencies>{}</dependencies>",
            dependency("leaf", Some("2"), "")
        ),
    );
    repo.add(
        "leaf",
        "1",
        &format!(
            "<dependencies>{}</dependencies>",
            dependency("child", Some("1"), "")
        ),
    );
    repo.add("child", "1", "");
    assert_eq!(
        repo.resolve().unwrap(),
        vec![
            artifact("left", "1"),
            artifact("leaf", "1"),
            artifact("child", "1")
        ]
    );
}

#[test]
fn classifier_artifacts_coexist_and_default_profiles_supply_properties() {
    let mut repo = Repository::default();
    repo.add("app", "1", &format!("<profiles><profile><id>defaults</id><activation><activeByDefault>true</activeByDefault></activation><properties><library.version>2</library.version></properties></profile></profiles><dependencies>{}{}</dependencies>", dependency("library", Some("${library.version}"), ""), dependency("library", Some("2"), "<type>test-jar</type>")));
    repo.add("library", "2", "");
    let mut tests = artifact("library", "2");
    tests.classifier = Some("tests".into());
    assert_eq!(
        repo.resolve().unwrap(),
        vec![artifact("library", "2"), tests.clone()]
    );
    let url = Url::parse("https://example.org/maven/").unwrap();
    assert_eq!(
        tests.url(&url).as_str(),
        "https://example.org/maven/org/example/library/2/library-2-tests.jar"
    );
    janex_format::purl::parse(&tests.purl(&url)).unwrap();
}

#[test]
fn unsupported_dependency_semantics_and_recursive_models_fail_explicitly() {
    for (fields, diagnostic) in [
        (format!("<dependencies>{}</dependencies>", dependency("library", Some("[1,2)"), "")), "coordinate"),
        (format!("<dependencies>{}</dependencies>", dependency("library", Some("${unknown}"), "")), "property"),
        (format!("<dependencies>{}</dependencies>", dependency("library", Some("1-SNAPSHOT"), "")), "exact release"),
        ("<distributionManagement><relocation><groupId>other</groupId></relocation></distributionManagement>".into(), "relocation"),
        ("<profiles><profile><id>jdk</id><activation><jdk>21</jdk></activation><properties><version>2</version></properties></profile></profiles>".into(), "profile"),
        ("<profiles><profile><id>defaults</id><activation><activeByDefault>true</activeByDefault></activation><properties><version>2</version></properties></profile><profile><id>build</id><activation><jdk>21</jdk></activation><build/></profile></profiles>".into(), "profile"),
        ("<parent><groupId>org.example</groupId><artifactId>app</artifactId><version>1</version></parent>".into(), "cyclic"),
        (format!("<dependencyManagement><dependencies>{}</dependencies></dependencyManagement>", dependency("app", Some("1"), "<type>pom</type><scope>import</scope>")), "cyclic"),
    ] {
        let mut repo = Repository::default();
        repo.add("app", "1", &fields);
        let error = repo.resolve().unwrap_err().to_string();
        assert!(error.contains(diagnostic), "{error}");
    }
}

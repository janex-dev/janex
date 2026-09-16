// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Descriptor inventory and preparation without a Java subprocess.

use janex_java::{
    Limits,
    launch::{EntryPoint, LaunchMode, LaunchRequest},
    modules::{Module, automatic, system_roots},
    runtime::JavaRuntime,
};
use std::collections::BTreeMap;

/// Describes an intentionally non-executable installation to catch accidental preparation probes.
fn runtime() -> JavaRuntime {
    JavaRuntime {
        executable: "missing-java-executable".into(),
        home: "missing-java-home".into(),
        feature: 25,
        version_text: "25".into(),
        vendor: "test".into(),
        architecture: "aarch64".into(),
        vm_name: None,
        modules: BTreeMap::from([
            ("java.base".into(), Some("25".into())),
            ("jdk.jconsole".into(), Some("25".into())),
        ]),
    }
}

#[test]
fn inventory_checks_requirements_without_resolving_or_executing() {
    let runtime = runtime();
    let app = Module {
        name: "sample.app".into(),
        version: Some("2+build.7".into()),
        requires: vec!["java.base".into(), "jdk.jconsole".into()],
    };
    let roots = system_roots(
        &runtime,
        std::slice::from_ref(&app),
        &[
            ("sample.app".into(), Some("2+build.7".into())),
            ("java.base".into(), Some("25".into())),
        ],
        Some("sample.app"),
    )
    .unwrap();
    assert_eq!(
        roots.into_iter().collect::<Vec<_>>(),
        ["java.base", "jdk.jconsole"]
    );
    assert!(
        system_roots(
            &runtime,
            std::slice::from_ref(&app),
            &[("sample.app".into(), Some("2".into()))],
            None
        )
        .is_err()
    );
    let mut missing = runtime.clone();
    missing.modules.remove("jdk.jconsole");
    assert!(system_roots(&missing, std::slice::from_ref(&app), &[], None).is_err());
    assert!(system_roots(&runtime, &[app.clone(), app], &[], None).is_err());
    assert!(system_roots(&runtime, &[], &[], Some("missing.app")).is_err());
    let shadow = Module {
        name: "java.base".into(),
        version: None,
        requires: vec![],
    };
    assert!(system_roots(&runtime, &[shadow], &[], None).is_err());
}

#[test]
fn automatic_identities_follow_filename_and_manifest_rules() {
    for (file, name, version) in [
        ("auto-library-1.2.jar", "auto.library", Some("1.2")),
        ("auto--library-11.jar", "auto.library", Some("11")),
        ("auto-library-1beta.jar", "auto.library.1beta", None),
        ("auto-library-1.2-.jar", "auto.library", None),
        ("auto-library-1.2-rc-.jar", "auto.library", Some("1.2-rc-")),
        ("auto-library-1.2-rc+.jar", "auto.library", None),
    ] {
        let module = automatic(file, None);
        assert_eq!(module.name, name);
        assert_eq!(module.version.as_deref(), version);
    }
    let module = automatic("auto-library-1.2.jar", Some("custom.library"));
    assert_eq!(module.name, "custom.library");
    assert_eq!(module.version.as_deref(), Some("1.2"));
}

#[test]
fn indexed_launch_preparation_never_starts_java() {
    let directory = tempfile::tempdir().unwrap();
    let request = LaunchRequest {
        entry_point: EntryPoint {
            main_class: Some("app.Main".into()),
            main_module: Some("sample.app".into()),
        },
        mode: LaunchMode::Bootstrap,
        jvm_options: vec![
            "--add-modules=sample.app,jdk.jconsole".into(),
            "--add-opens=sample.app/app=ALL-UNNAMED".into(),
        ],
        class_path: vec![],
        module_path: vec![],
        agents: vec![],
        arguments: vec![],
    };
    // Launch data is opaque to this layer; only the final JVM consumes the resource index.
    let args = request
        .prepare_with_resources(
            &runtime(),
            directory.path(),
            Limits::default(),
            Some(b"opaque-index"),
        )
        .unwrap();
    assert!(args.iter().any(|arg| arg == "--add-modules=jdk.jconsole"));
    assert!(!args.iter().any(|arg| arg == "--add-modules=ALL-SYSTEM"));
    assert!(
        !args
            .iter()
            .any(|arg| arg == "--add-modules=sample.app,jdk.jconsole")
    );
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Automatic module metadata across direct, bootstrap, and standalone launches.

use janex_format::{
    application::{Application, EntryPoint, PathEntry},
    binary::Limits,
    blob::{BlobRef, PoolBuilder},
    cbor::Value,
    container::{APPLICATION, BLOB_POOL, Writer},
    resource::ResourceRoot,
};
use janex_host::{
    import::{ImportOptions, import_path},
    run::{LaunchMode, RunOptions, prepare},
};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

/// Writes a module root with independently selected metadata and original resources.
fn package(target: &Path, mut root: ResourceRoot, module: &str) {
    let mut pool = PoolBuilder::new();
    let encoded = root.encode(Limits::default()).unwrap();
    assert_eq!(pool.push(&root.data.encode().unwrap(), 3).unwrap(), 0);
    let index = pool.push(&encoded, 3).unwrap();
    let pool = pool.finish(8, 3).unwrap();
    let entry = PathEntry::Local(BlobRef { pool: 1, index });
    let application = Application::from_values(
        Value::map([
            (Value::uint(0), Value::text("main")),
            (Value::uint(1), Value::text("janex.java")),
        ])
        .unwrap(),
        Value::map([(
            Value::uint(0),
            Value::map([(
                Value::uint(0),
                Value::map([
                    (
                        Value::uint(1),
                        EntryPoint {
                            main_module: Some(module.into()),
                            main_class: Some("demo.Main".into()),
                        }
                        .to_value()
                        .unwrap(),
                    ),
                    (
                        Value::uint(2),
                        Value::array([entry.to_value(true).unwrap()]),
                    ),
                ])
                .unwrap(),
            )])
            .unwrap(),
        )])
        .unwrap(),
        Limits::default(),
    )
    .unwrap();
    let tail = include_bytes!("../../janex-bootstrap/build/libs/janex-bootstrap.jar");
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(1, BLOB_POOL, &pool.bytes, Some(pool.type_info))
        .unwrap();
    writer
        .write_section(
            0,
            APPLICATION,
            &application.encode().unwrap(),
            Some(application.type_info().clone()),
        )
        .unwrap();
    let mut bytes = writer
        .finish(
            Value::map([(
                Value::uint(2),
                Value::map([(Value::uint(0), Value::uint(tail.len() as u64))]).unwrap(),
            )])
            .unwrap(),
        )
        .unwrap();
    bytes.extend_from_slice(tail);
    fs::write(target, bytes).unwrap();
}

/// Checks the selected module identity and effective manifest in a launched JVM.
fn check(command: &mut Command, expected: &str) {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), expected);
}

#[test]
fn automatic_module_metadata_overrides_manifests_but_not_descriptors() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("demo")).unwrap();
    fs::write(source.join("demo/Main.java"), r#"
package demo;
/// Reports both module identity and the effective runtime manifest.
public class Main {
    /// Reads the manifest from this module's own resources.
    public static void main(String[] args) throws Exception {
        try (java.io.InputStream input = Main.class.getResourceAsStream("/META-INF/MANIFEST.MF")) {
            String name = input == null ? "missing"
                : new java.util.jar.Manifest(input).getMainAttributes().getValue("Automatic-Module-Name");
            System.out.println(Main.class.getModule().getName() + ":" + name);
        }
    }
}
"#).unwrap();
    let compiled = Command::new("javac")
        .args(["--release", "9", "-d"])
        .arg(&source)
        .arg(source.join("demo/Main.java"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    for (case, manifest, extension, descriptor, expected) in [
        (
            "missing",
            None,
            Some("override.name"),
            false,
            "override.name:override.name",
        ),
        (
            "override",
            Some("original.name"),
            Some("override.name"),
            false,
            "override.name:override.name",
        ),
        (
            "manifest",
            Some("original.name"),
            None,
            false,
            "original.name:original.name",
        ),
        ("filename", None, None, false, "filename.name:missing"),
        (
            "descriptor",
            Some("original.name"),
            Some("override.name"),
            true,
            "explicit.name:override.name",
        ),
    ] {
        let manifest_path = source.join("META-INF/MANIFEST.MF");
        if let Some(name) = manifest {
            fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
            fs::write(
                &manifest_path,
                format!("Manifest-Version: 1.0\r\nAutomatic-Module-Name: {name}\r\n\r\n"),
            )
            .unwrap();
        } else if manifest_path.exists() {
            fs::remove_file(&manifest_path).unwrap();
        }
        if descriptor {
            fs::write(
                source.join("module-info.java"),
                "module explicit.name { exports demo; }",
            )
            .unwrap();
            let compiled = Command::new("javac")
                .args(["--release", "9", "-d"])
                .arg(&source)
                .arg(source.join("module-info.java"))
                .arg(source.join("demo/Main.java"))
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
        }
        let mut root = import_path(&source, ImportOptions::default())
            .unwrap()
            .into_resource_root(BlobRef { pool: 1, index: 0 })
            .unwrap();
        assert_eq!(
            root.metadata
                .get_text("janex.java.automatic_module_name")
                .unwrap()
                .map(|value| value.as_text().unwrap().to_owned())
                .as_deref(),
            manifest
        );
        let mut metadata = vec![(Value::uint(0), Value::text("filename-name-1.0.jar"))];
        if let Some(name) = extension {
            metadata.push((
                Value::text("janex.java.automatic_module_name"),
                Value::text(name),
            ));
        }
        root.metadata = Value::map(metadata).unwrap();
        let target = temp.path().join(format!("{case}.janex"));
        package(&target, root, expected.split(':').next().unwrap());
        for mode in [LaunchMode::Direct, LaunchMode::Bootstrap] {
            let mut options = RunOptions::new(&target);
            options.allow_unsigned = true;
            options.java.java = Some("java".into());
            options.launch_mode = mode;
            let plan = prepare(&options).unwrap();
            check(&mut plan.command(), expected);
        }
        check(Command::new("java").arg("-jar").arg(&target), expected);
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Native/Java application overlay selection and ordered argument parity.

use janex_format::{
    application::Application,
    binary::Limits,
    cbor::{self, Value},
    condition::{Context, RuntimeContext},
    container::{APPLICATION, Writer},
    version::JavaVersion,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Writes a trusted length-prefixed byte sequence.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Records an independent container and the native evaluated launch, if selectable.
fn vector(output: &mut Vec<u8>, config: Value, limits: Limits) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap());
    output[..4].copy_from_slice(&(count + 1).to_be_bytes());
    for limit in [
        limits.max_bytes,
        limits.max_elements,
        limits.max_depth as u64,
    ] {
        output.extend((limit as u32).to_be_bytes());
    }
    let info = map([(0, Value::text("main")), (1, Value::text("janex.java"))]);
    let value = map([(0, map([(0, config)]))]);
    let mut application = b"JANEXAPP".to_vec();
    cbor::write_sized(&mut application, &value).unwrap();
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(0, APPLICATION, &application, Some(info.clone()))
        .unwrap();
    let package = writer.finish(Value::empty_map()).unwrap();
    bytes(output, &package);
    let context = Context {
        os: "windows".into(),
        arch: "x86-64".into(),
        invocation: Some("run".into()),
        runtime: Some(RuntimeContext {
            runtime_type: "janex.java".into(),
            java_version: Some(JavaVersion::parse("25").unwrap()),
            vendor: "Vendor".into(),
        }),
    };
    let launch = Application::decode(&application, info, limits)
        .ok()
        .and_then(|application| application.evaluate_java(&context).ok())
        .flatten();
    output.push(u8::from(launch.is_some()));
    if let Some(launch) = launch {
        bytes(
            output,
            launch
                .entry_point
                .main_class
                .as_deref()
                .unwrap_or("")
                .as_bytes(),
        );
        bytes(
            output,
            launch
                .entry_point
                .main_module
                .as_deref()
                .unwrap_or("")
                .as_bytes(),
        );
        for list in [launch.jvm_options, launch.arguments] {
            output.extend((list.len() as u32).to_be_bytes());
            for value in list {
                bytes(output, value.as_bytes());
            }
        }
    }
}

/// Builds deterministic integer-keyed test maps.
fn map(entries: impl IntoIterator<Item = (u64, Value)>) -> Value {
    Value::map(
        entries
            .into_iter()
            .map(|(key, value)| (Value::uint(key), value)),
    )
    .unwrap()
}

/// Creates a configuration node with optional children and distinguishable arguments.
fn node(active: bool, label: &str, children: Vec<Value>) -> Value {
    map([
        (
            0,
            map([(1, Value::text(if active { "windows" } else { "never" }))]),
        ),
        (1, map([(0, Value::text(label))])),
        (
            5,
            Value::array([Value::text(&format!("-Dselected={label}"))]),
        ),
        (6, Value::array(children)),
        (7, Value::array([Value::text(label)])),
    ])
}

#[test]
fn java_launch_selection_matches_native_overlay_order_clearing_and_limits() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/build/libs/janex-bootstrap.jar");
    let compiled =
        Command::new("javac")
            .args(["--release", "8", "-cp"])
            .arg(&bootstrap)
            .arg("-d")
            .arg(temp.path())
            .arg(project.join(
                "janex-reader/src/testFixtures/java/org/janex/reader/LaunchSelectionTest.java",
            ))
            .output()
            .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let limits = Limits::default();
    let mut vectors = vec![0; 4];
    for bits in 0..32 {
        vector(
            &mut vectors,
            node(
                bits & 1 != 0,
                "root",
                vec![
                    node(
                        bits & 2 != 0,
                        "first",
                        vec![node(bits & 4 != 0, "nested", vec![])],
                    ),
                    node(
                        bits & 8 != 0,
                        "second",
                        vec![node(bits & 16 != 0, "last", vec![])],
                    ),
                ],
            ),
            limits,
        );
    }
    for key in [1, 2, 3, 4, 5, 7] {
        for value in [
            Value::null(),
            Value::array([]),
            Value::array([
                Value::text(""),
                Value::text("a b"),
                Value::text("中🚀"),
                Value::text("a\0b"),
            ]),
            Value::text("bad"),
            map([(0, Value::text("Replacement"))]),
        ] {
            for active in [false, true] {
                let child = map([
                    (
                        0,
                        map([(1, Value::text(if active { "windows" } else { "never" }))]),
                    ),
                    (key, value.clone()),
                ]);
                vector(
                    &mut vectors,
                    node(
                        true,
                        "root",
                        vec![child, map([(7, Value::array([Value::text("after")]))])],
                    ),
                    limits,
                );
            }
        }
    }
    for clear in [false, true] {
        let mut children = vec![map([(7, Value::array([Value::text("first")]))])];
        if clear {
            children.push(map([(5, Value::null()), (7, Value::null())]));
        }
        children.push(map([
            (5, Value::array([Value::text("-Dlast=true")])),
            (7, Value::array([Value::text("last")])),
        ]));
        vector(&mut vectors, node(true, "root", children), limits);
    }
    // Each individual list fits; nested pending siblings can still exceed the native work bound.
    for active in [false, true] {
        for width in [4, 5, 7] {
            let nested = map([
                (
                    0,
                    map([(1, Value::text(if active { "windows" } else { "never" }))]),
                ),
                (6, Value::array((0..width).map(|_| Value::empty_map()))),
            ]);
            let mut children = vec![nested];
            children.extend((1..width).map(|_| Value::empty_map()));
            for max_elements in [8, 12, 16] {
                vector(
                    &mut vectors,
                    node(true, "root", children.clone()),
                    Limits {
                        max_elements,
                        ..limits
                    },
                );
            }
        }
    }
    let fixture = temp.path().join("selection.bin");
    fs::write(&fixture, vectors).unwrap();
    let classpath = std::env::join_paths([temp.path(), bootstrap.as_path()]).unwrap();
    let mut runtimes = vec![PathBuf::from("java")];
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        runtimes.push(PathBuf::from(home).join("bin").join(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
    }
    for java in runtimes {
        let output = Command::new(&java)
            .arg("-cp")
            .arg(&classpath)
            .arg("org.janex.reader.LaunchSelectionTest")
            .arg(&fixture)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            java.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

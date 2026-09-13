// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Cross-runtime version and condition schema/selection parity.

use janex_format::{
    cbor::Value,
    condition::{Condition, Context, RuntimeContext},
    version::{JavaRange, JavaVersion},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Writes trusted harness text or bytes.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Begins one vector with its native acceptance and selection result.
fn vector(output: &mut Vec<u8>, mode: u8, encoded: &[u8], candidate: &str, result: i32) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap());
    output[..4].copy_from_slice(&(count + 1).to_be_bytes());
    output.push(mode);
    bytes(output, encoded);
    bytes(output, candidate.as_bytes());
    output.extend(result.to_be_bytes());
}

/// Records one range with an independently parsed runtime version.
fn range(output: &mut Vec<u8>, text: &str, candidate: &str) {
    let result = JavaRange::parse(text)
        .and_then(|range| {
            JavaVersion::parse(candidate).map(|candidate| i32::from(range.contains(&candidate)))
        })
        .unwrap_or(-1);
    vector(output, 1, text.as_bytes(), candidate, result);
}

#[test]
fn java_versions_and_conditions_match_native_validation_and_selection() {
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
                "janex-reader/src/testFixtures/java/org/glavo/janex/reader/ConditionsTest.java",
            ))
            .output()
            .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut versions = BTreeSet::new();
    for number in [
        "",
        "7",
        "8",
        "08",
        "17",
        "17.0",
        "17.0.1",
        "17..1",
        "2147483647",
        "2147483648",
        "8u0",
        "8u452",
        "8u0452",
        "1.8.0",
        "1.8.0_452",
        "1.8.0_0452",
    ] {
        for pre in [
            "", "-ea", "-1", "-01", "-0", "-000", "-b09", "-foo.bar", "--", "-",
        ] {
            for build in [
                "",
                "+",
                "+0",
                "+01",
                "+1",
                "+2147483648",
                "+-LTS",
                "-LTS",
                "-x+1",
                "+1-LTS",
            ] {
                versions.insert(format!("{number}{pre}{build}"));
            }
        }
    }
    for value in [
        "17\n",
        "17\r\n",
        "17\0",
        "17-é",
        "17-١",
        "1.8.0_452-b9",
        "1.8.0_452-b09",
        "1.8.0_0",
    ] {
        versions.insert(value.to_owned());
    }
    let mut vectors = vec![0; 4];
    for value in &versions {
        let result = JavaVersion::parse(value).map_or(-1, |version| version.feature() as i32);
        vector(&mut vectors, 0, &[], value, result);
        for text in [
            "vers:jep322/*",
            "vers:jep322/>=8|!=17|<25",
            "vers:jep322/>=17-1|<17",
        ] {
            range(&mut vectors, text, value);
        }
        for operator in ["", "=", "!=", ">", ">=", "<", "<="] {
            range(
                &mut vectors,
                &format!("vers:jep322/{operator}{value}"),
                "17",
            );
        }
    }
    for left in ["", "=", "!=", ">", ">=", "<", "<="] {
        for right in ["", "=", "!=", ">", ">=", "<", "<="] {
            for candidate in ["8", "11", "11-ea", "17", "18", "21", "25"] {
                range(
                    &mut vectors,
                    &format!("vers:jep322/{left}11|{right}17"),
                    candidate,
                );
                range(
                    &mut vectors,
                    &format!("vers:jep322/{left}17|{right}11"),
                    candidate,
                );
                range(
                    &mut vectors,
                    &format!("vers:jep322/{left}11|!=13|{right}17|>=21"),
                    candidate,
                );
            }
        }
    }
    for value in [
        format!("17{}", ".1".repeat(4096)),
        format!("17-{}", "0".repeat(4096)),
    ] {
        let result = JavaVersion::parse(&value).map_or(-1, |version| version.feature() as i32);
        vector(&mut vectors, 0, &[], &value, result);
        range(&mut vectors, &format!("vers:jep322/>={value}"), &value);
    }
    let selectors = vec![
        Value::null(),
        Value::uint(1),
        Value::text(""),
        Value::text("windows"),
        Value::text("linux"),
        Value::text("macos"),
        Value::text("freebsd"),
        Value::text("x86-64"),
        Value::text("aarch64"),
        Value::text("run"),
        Value::text("command"),
        Value::array([]),
        Value::array([Value::text("windows"), Value::text("")]),
        Value::array([Value::text("windows"), Value::uint(1)]),
        Value::array([Value::text("windows"), Value::text("linux")]),
    ];
    let mut conditions = vec![Value::empty_map()];
    for key in 0..=6 {
        for value in &selectors {
            conditions.push(Value::map([(Value::uint(key), value.clone())]).unwrap());
        }
    }
    for ty in ["", "unknown", "janex.java"] {
        for field in 0..=2 {
            for value in selectors.iter().cloned().chain([
                Value::text("vers:jep322/>=17"),
                Value::text("vers:jep322/17|<25"),
                Value::text("Vendor"),
                Value::text("vers:jep322/17--LTS"),
            ]) {
                let runtime = Value::map([
                    (Value::uint(0), Value::text(ty)),
                    (
                        Value::uint(1),
                        Value::map([(Value::uint(field), value)]).unwrap(),
                    ),
                ])
                .unwrap();
                for os in ["windows", "never-selected"] {
                    conditions.push(
                        Value::map([
                            (Value::uint(1), Value::text(os)),
                            (Value::uint(5), runtime.clone()),
                        ])
                        .unwrap(),
                    );
                }
            }
        }
    }
    for (os_property, os, arch_property, arch, version) in [
        ("Windows 11", "windows", "amd64", "x86-64", "1.8.0_452-b9"),
        ("Linux", "linux", "x86_64", "x86-64", "17-ea+1"),
        ("Mac OS X", "macos", "arm64", "aarch64", "25"),
        ("FreeBSD", "freebsd", "i386", "x86", "17"),
    ] {
        let context = Context {
            os: os.into(),
            arch: arch.into(),
            invocation: Some("run".into()),
            runtime: Some(RuntimeContext {
                runtime_type: "janex.java".into(),
                java_version: Some(JavaVersion::parse(version).unwrap()),
                vendor: "Vendor".into(),
            }),
        };
        for condition in &conditions {
            let result = Condition::from_value(condition.clone())
                .and_then(|condition| condition.matches(&context))
                .map_or(-1, i32::from);
            vector(&mut vectors, 2, condition.as_bytes(), "", result);
            for value in [os_property, arch_property, version, "Vendor"] {
                bytes(&mut vectors, value.as_bytes());
            }
        }
    }
    let fixture = temp.path().join("conditions.bin");
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
            .arg("org.glavo.janex.reader.ConditionsTest")
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

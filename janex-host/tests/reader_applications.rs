// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Java application schema validation and language lookup against native parsing.

use janex_format::{
    application::Application,
    binary::Limits,
    cbor::{self, Value},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Encodes a trusted harness byte string.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Replaces one field in an integer-keyed object.
fn field(value: &Value, key: u64, replacement: Value) -> Value {
    let mut entries = value.as_map().unwrap();
    entries.retain(|(candidate, _)| candidate.as_u64().ok() != Some(key));
    entries.push((Value::uint(key), replacement));
    Value::map(entries).unwrap()
}

/// Records native schema acceptance and locale-dependent results for one application.
fn vector(output: &mut Vec<u8>, value: &Value, kind: &str, locales: &[String]) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap());
    output[..4].copy_from_slice(&(count + 1).to_be_bytes());
    let info = Value::map([
        (Value::uint(0), Value::text("application-id")),
        (Value::uint(1), Value::text(kind)),
    ])
    .unwrap();
    let mut encoded = b"JANEXAPP".to_vec();
    cbor::write_sized(&mut encoded, value).unwrap();
    bytes(output, &encoded);
    bytes(output, info.as_bytes());
    let parsed = Application::decode(&encoded, info, Limits::default());
    output.push(u8::from(parsed.is_ok()));
    if let Ok(application) = parsed {
        bytes(output, application.id().as_bytes());
        bytes(output, application.application_type().as_bytes());
        output.push(u8::from(application.windowed()));
        bytes_count(output, locales.len());
        for locale in locales {
            bytes(output, locale.as_bytes());
            bytes(output, application.title(locale).as_bytes());
            let comment = application.comment(locale);
            output.push(u8::from(comment.is_some()));
            if let Some(comment) = comment {
                bytes(output, comment.as_bytes());
            }
        }
    }
}

/// Writes a bounded collection count.
fn bytes_count(output: &mut Vec<u8>, count: usize) {
    output.extend((count as u32).to_be_bytes());
}

#[test]
fn java_applications_validate_metadata_inactive_descriptors_and_locale_lookup() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/bootstrap.jar");
    let compile = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&bootstrap)
        .arg("-d")
        .arg(temp.path())
        .arg(
            project
                .join("janex-reader/src/testFixtures/java/org/janex/format/ApplicationTest.java"),
        )
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let mut vectors = vec![0; 4];
    let base = Value::map([(Value::uint(0), Value::empty_map())]).unwrap();
    let locales: Vec<String> = [
        "en-US",
        "en-US-x-test",
        "EN-gb",
        "zh-Hant-TW",
        "fr",
        "",
        "bad_tag",
        "und",
        "x-private",
        "de-a-bbbb-x-test",
        "i-klingon",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let values = [
        Value::null(),
        Value::boolean(false),
        Value::boolean(true),
        Value::uint(0),
        Value::uint(1),
        Value::uint(u64::MAX),
        Value::text(""),
        Value::text("text"),
        Value::bytes(b"bytes"),
        Value::array([]),
        Value::array([Value::null()]),
        Value::empty_map(),
        Value::map([(Value::text("extension"), Value::null())]).unwrap(),
    ];
    for key in 0..=5 {
        for value in &values {
            vector(
                &mut vectors,
                &field(&base, key, value.clone()),
                "org.example.runtime",
                &locales,
            );
        }
    }
    for key in 0..=2 {
        for value in &values {
            let integration = Value::map([(Value::uint(key), value.clone())]).unwrap();
            vector(
                &mut vectors,
                &field(&base, 4, integration),
                "org.example.runtime",
                &locales,
            );
        }
    }
    for command in [
        "command", ".", "..", "../cmd", "a/b", "a\0b", "a\\b", "名字",
    ] {
        vector(
            &mut vectors,
            &field(
                &base,
                4,
                Value::map([(Value::uint(0), Value::text(command))]).unwrap(),
            ),
            "org.example.runtime",
            &locales,
        );
    }
    let icon = Value::map([
        (Value::uint(0), Value::text("image/png")),
        (
            Value::uint(1),
            Value::array([Value::uint(u64::MAX), Value::uint(0)]),
        ),
    ])
    .unwrap();
    for key in 0..=1 {
        for value in values.iter().chain(std::iter::once(&icon)) {
            let icon = field(&icon, key, value.clone());
            let integration = Value::map([(Value::uint(2), Value::array([icon]))]).unwrap();
            vector(
                &mut vectors,
                &field(&base, 4, integration),
                "org.example.runtime",
                &locales,
            );
        }
    }
    for key in 0..=7 {
        for value in &values {
            let invalid = Value::map([(Value::uint(key), value.clone())]).unwrap();
            let config = Value::map([
                (
                    Value::uint(0),
                    Value::map([(Value::uint(1), Value::text("unknown-os"))]).unwrap(),
                ),
                (Value::uint(6), Value::array([invalid])),
            ])
            .unwrap();
            let value = Value::map([(
                Value::uint(0),
                Value::map([(Value::uint(0), config)]).unwrap(),
            )])
            .unwrap();
            vector(&mut vectors, &value, "janex.java", &locales);
        }
    }
    let translations = Value::map([
        (Value::text("en-US"), Value::text("English")),
        (Value::text("zh-Hant"), Value::text("Traditional")),
        (Value::text("und"), Value::text("Neutral")),
        (Value::text("bad_tag"), Value::text("Ignored")),
    ])
    .unwrap();
    vector(
        &mut vectors,
        &field(&field(&base, 1, translations.clone()), 3, translations),
        "org.example.runtime",
        &locales,
    );
    let mut tags: Vec<String> = [
        "en",
        "EN",
        "en-US",
        "zh-Hant",
        "qaa-Qaaa-QQ",
        "x-private",
        "x-toooolong",
        "x-a-b",
        "en-US-x-test",
        "en-abc-def-ghi",
        "en-abc-def-ghi-jkl",
        "abcd-abc",
        "en-123",
        "en-1234",
        "en-variant-variant",
        "en-a-one-a-two",
        "i-klingon",
        "en-GB-oed",
        "sgn-BE-FR",
        "zh-min-nan",
        "",
        "bad_tag",
        "en-",
        "x-",
        "en-a",
        "en-x",
        "e",
        "Kk",
        "en-KK",
        "é",
        "🚀",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for part in [
        "a",
        "ab",
        "abc",
        "abcd",
        "abcde",
        "123",
        "1234",
        "12345",
        "x",
        "a0",
        "123456789",
    ] {
        for prefix in ["en", "abcd", "en-US", "en-variant", "en-a", "x", "en-x"] {
            tags.push(format!("{prefix}-{part}"));
        }
    }
    for tag in &tags {
        let requested = [locales.clone(), vec![tag.clone(), format!("{tag}-x-test")]].concat();
        for key in [1, 3] {
            for entries in [
                vec![(Value::text(tag), Value::text("Translation"))],
                vec![
                    (Value::text(tag), Value::text("Translation")),
                    (Value::text("fr"), Value::text("Fallback")),
                ],
            ] {
                vector(
                    &mut vectors,
                    &field(&base, key, Value::map(entries).unwrap()),
                    "org.example.runtime",
                    &requested,
                );
            }
        }
    }
    let duplicate = Value::map([
        (Value::text("en"), Value::text("one")),
        (Value::text("EN"), Value::text("two")),
    ])
    .unwrap();
    vector(
        &mut vectors,
        &field(&base, 1, duplicate),
        "org.example.runtime",
        &locales,
    );
    let fixture = temp.path().join("applications.bin");
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
        let output = Command::new(java)
            .arg("-cp")
            .arg(&classpath)
            .arg("org.janex.format.ApplicationTest")
            .arg(&fixture)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

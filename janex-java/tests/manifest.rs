// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! JAR manifest parsing and deterministic runtime rewriting.

use janex_java::manifest::Manifest;

#[test]
fn manifest_continuations_join_before_utf8_decoding_and_sections_merge() {
    let bytes = b"Manifest-Version: 1.0\r\nMain-Class: sample.\r\n Main\r\nX-Text: \xe4\r\n \xb8\xad\r\nMulti-Release: TRUE\r\nClass-Path: remote.jar\r\n\r\nName: sample/Main.class\rX-Value: first\r\rName: sample/Main.class\nX-Value: last\n\n";
    let manifest = Manifest::parse(bytes, janex_java::Limits::default()).unwrap();
    assert_eq!(manifest.get("MAIN-CLASS"), Some("sample.Main"));
    assert_eq!(manifest.get("x-text"), Some("\u{4e2d}"));
    assert_eq!(manifest.get("Class-Path"), Some("remote.jar"));
    assert!(manifest.multi_release());
    assert_eq!(
        manifest.entry_attribute("sample/Main.class", "X-VALUE"),
        Some("last")
    );
    let duplicates = Manifest::parse(
        b"Main-Class: First\nmain-class: Last\n\n",
        janex_java::Limits::default(),
    )
    .unwrap();
    assert_eq!(duplicates.get("Main-Class"), Some("Last"));
    for invalid in [
        &b" Orphan\n"[..],
        &b"Header:no space\n"[..],
        &b"Bad!Name: value\n"[..],
        &b"Name: forbidden\n\n"[..],
        &b"Key: value\0\n"[..],
        &b"Key: \xff\n"[..],
        &b"Key: unterminated"[..],
        &b"Key: value\n\nOther: value\n"[..],
    ] {
        assert!(
            Manifest::parse(invalid, janex_java::Limits::default()).is_err(),
            "{invalid:?}"
        );
    }
}

#[test]
fn runtime_manifest_retains_metadata_and_folds_long_utf8_values() {
    let long_name = "X".repeat(70);
    let long_value = "\u{4e2d}\u{1f600} ".repeat(100);
    let input = format!(
        "Manifest-Version: 1.0\r\nClass-Path: missing.jar\r\nMain-Class: sample.Main\r\n{long_name}: {long_value}\r\nSHA-256-Digest-Manifest: stale\r\n\r\nName: sample/\r\nSealed: true\r\nSHA-256-Digest: stale\r\n\r\nName: only-digest\r\nSHA-256-Digest: stale\r\n\r\n"
    );
    let manifest = Manifest::parse(input.as_bytes(), janex_java::Limits::default()).unwrap();
    let bytes = manifest.for_runtime();
    assert!(
        bytes.split(|byte| *byte == b'\n').all(|line| line
            .strip_suffix(b"\r")
            .unwrap_or(line)
            .len()
            <= 72)
    );
    let rewritten = Manifest::parse(&bytes, janex_java::Limits::default()).unwrap();
    assert_eq!(rewritten.get(&long_name), Some(long_value.as_str()));
    assert_eq!(rewritten.get("Main-Class"), Some("sample.Main"));
    assert_eq!(rewritten.entry_attribute("sample/", "Sealed"), Some("true"));
    assert_eq!(rewritten.get("Class-Path"), None);
    assert_eq!(rewritten.get("SHA-256-Digest-Manifest"), None);
    assert_eq!(rewritten.entry_attribute("sample/", "SHA-256-Digest"), None);
    assert!(!String::from_utf8_lossy(&bytes).contains("only-digest"));
    assert_eq!(manifest.get("Class-Path"), Some("missing.jar"));
    assert_eq!(bytes, rewritten.for_runtime());
}

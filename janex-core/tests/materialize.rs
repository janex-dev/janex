// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Runtime JAR contents, manifest rewriting, and bounded link expansion.

use janex_core::{manifest::Manifest, materialize::materialize};
use janex_format::{
    binary::Limits,
    blob::{BlobRef, BlobStore},
    cbor::Value,
    condition::{Condition, Context},
    container::{Reader, Writer},
    content::Content,
    resource::{Directory, DirectoryEntry, Layer, ResourceRoot},
    strings::StringPool,
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
};

/// Creates a resource-only context.
fn context() -> Context {
    Context {
        os: "linux".into(),
        arch: "x86-64".into(),
        invocation: None,
        runtime: None,
    }
}

/// Supplies an empty container for roots with inline content.
fn blobs(limits: Limits) -> BlobStore<Cursor<Vec<u8>>> {
    let bytes = Writer::new(Vec::new())
        .unwrap()
        .finish(Value::empty_map())
        .unwrap();
    BlobStore::new(Reader::open_auto(Cursor::new(bytes), limits).unwrap())
}

/// Creates a root with sorted directories and entries.
fn root(directories: Vec<(&str, Vec<DirectoryEntry>)>) -> ResourceRoot {
    let mut directories: Vec<_> = directories
        .into_iter()
        .map(|(path, mut entries)| {
            entries.sort_by(|a, b| a.name().cmp(b.name()));
            Directory {
                path: path.into(),
                entries,
                metadata: Value::empty_map(),
            }
        })
        .collect();
    directories.sort_by(|a, b| a.path.cmp(&b.path));
    ResourceRoot {
        string_pool: BlobRef { pool: 1, index: 0 },
        strings: StringPool::new(),
        metadata: Value::map([(
            Value::text("janex.java.jar_name"),
            Value::text("original-name.jar"),
        )])
        .unwrap(),
        layers: vec![Layer {
            condition: Condition::unconditional(),
            directories,
        }],
    }
}

/// Creates one inline file.
fn file(name: &str, bytes: &[u8]) -> DirectoryEntry {
    DirectoryEntry::File {
        name: name.into(),
        content: Content::inline(bytes.into()),
        metadata: Value::empty_map(),
    }
}

/// Creates one symbolic link without native filesystem privileges.
fn link(name: &str, target: &str) -> DirectoryEntry {
    DirectoryEntry::SymbolicLink {
        name: name.into(),
        target: target.into(),
        metadata: Value::empty_map(),
    }
}

#[test]
fn runtime_manifest_retains_metadata_and_folds_long_utf8_values() {
    let long_name = "X".repeat(70);
    let long_value = "\u{4e2d}\u{1f600} ".repeat(100);
    let input = format!(
        "Manifest-Version: 1.0\r\nClass-Path: missing.jar\r\nMain-Class: sample.Main\r\n{long_name}: {long_value}\r\nSHA-256-Digest-Manifest: stale\r\n\r\nName: sample/\r\nSealed: true\r\nSHA-256-Digest: stale\r\n\r\nName: only-digest\r\nSHA-256-Digest: stale\r\n\r\n"
    );
    let manifest = Manifest::parse(input.as_bytes(), Limits::default()).unwrap();
    let bytes = manifest.for_runtime();
    assert!(
        bytes.split(|byte| *byte == b'\n').all(|line| line
            .strip_suffix(b"\r")
            .unwrap_or(line)
            .len()
            <= 72)
    );
    let rewritten = Manifest::parse(&bytes, Limits::default()).unwrap();
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

#[test]
fn expands_links_and_removes_only_signature_files() {
    let root = root(vec![
        ("", vec![link("alias", "real"), link("linked", "real/data")]),
        ("real", vec![file("data", b"value")]),
        ("empty", vec![]),
        (
            "META-INF",
            vec![
                file(
                    "MANIFEST.MF",
                    b"Manifest-Version: 1.0\nClass-Path: remote.jar\nSealed: true\n\n",
                ),
                file("OLD.SF", b"signature"),
                file("OLD.RSA", b"signature"),
                file("sig-custom", b"signature"),
            ],
        ),
        ("META-INF/data.SF", vec![file("keep", b"ordinary resource")]),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let mut store = blobs(Limits::default());
    let result = materialize(&root, &context(), &mut store, temp.path(), 1024).unwrap();
    assert_eq!(result.path.file_name().unwrap(), "original-name.jar");
    let mut archive = zip::ZipArchive::new(fs::File::open(&result.path).unwrap()).unwrap();
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        entries.insert(entry.name().to_owned(), bytes);
    }
    assert_eq!(entries["alias/data"], b"value");
    assert_eq!(entries["linked"], b"value");
    assert_eq!(entries["META-INF/data.SF/keep"], b"ordinary resource");
    assert!(entries.contains_key("empty/"));
    for path in ["META-INF/OLD.SF", "META-INF/OLD.RSA", "META-INF/sig-custom"] {
        assert!(!entries.contains_key(path));
    }
    let manifest = Manifest::parse(&entries["META-INF/MANIFEST.MF"], Limits::default()).unwrap();
    assert_eq!(manifest.get("Class-Path"), None);
    assert_eq!(manifest.get("Sealed"), Some("true"));
    assert_eq!(
        result.logical_bytes,
        entries
            .values()
            .map(|bytes| bytes.len() as u64)
            .sum::<u64>()
    );
    assert!(materialize(&root, &context(), &mut store, temp.path(), 1024).is_err());
}

#[test]
fn rejects_invalid_links_and_counts_expanded_bytes() {
    for target in ["missing", "../escape", "link", "."] {
        let root = root(vec![("", vec![link("link", target)])]);
        let temp = tempfile::tempdir().unwrap();
        assert!(
            materialize(
                &root,
                &context(),
                &mut blobs(Limits::default()),
                temp.path(),
                1024
            )
            .is_err(),
            "{target}"
        );
    }
    let root = root(vec![(
        "",
        vec![
            file("data", b"1234"),
            link("one", "data"),
            link("two", "data"),
        ],
    )]);
    for (limit, succeeds) in [(11, false), (12, true)] {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            materialize(
                &root,
                &context(),
                &mut blobs(Limits::default()),
                temp.path(),
                limit
            )
            .is_ok(),
            succeeds
        );
    }
    let invalid_signature = root_with_dangling_signature();
    let temp = tempfile::tempdir().unwrap();
    assert!(
        materialize(
            &invalid_signature,
            &context(),
            &mut blobs(Limits::default()),
            temp.path(),
            1024
        )
        .is_err()
    );
}

/// Creates a dangling link whose name would otherwise be omitted as a signature file.
fn root_with_dangling_signature() -> ResourceRoot {
    root(vec![("META-INF", vec![link("OLD.SF", "missing")])])
}

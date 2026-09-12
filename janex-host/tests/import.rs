// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Manifest parsing and directory/JAR import through real resource-tree reads.

use janex_format::{
    binary::Limits,
    blob::{BlobRef, BlobStore},
    cbor::Value,
    condition::{Context, RuntimeContext},
    container::{Reader, Writer},
    resource::{Node, ResourceRoot},
    version::JavaVersion,
};
use janex_host::import::{ImportOptions, import_path};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
    process::Command,
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

/// Returns a context selecting a specific Java version.
fn context(version: &str) -> Context {
    Context {
        os: "linux".into(),
        arch: "x86-64".into(),
        invocation: Some("run".into()),
        runtime: Some(RuntimeContext {
            runtime_type: "janex.java".into(),
            java_version: Some(JavaVersion::parse(version).unwrap()),
            vendor: String::new(),
        }),
    }
}

/// Creates an empty blob store for roots whose imported file contents are inline.
fn blobs() -> BlobStore<Cursor<Vec<u8>>> {
    let bytes = Writer::new(Vec::new())
        .unwrap()
        .finish(Value::empty_map())
        .unwrap();
    BlobStore::new(Reader::open_auto(Cursor::new(bytes), Limits::default()).unwrap())
}

/// Reads one imported resource under a Java version's layer selection.
fn read(root: &ResourceRoot, version: &str, path: &str) -> Vec<u8> {
    root.merge(&context(version), Limits::default())
        .unwrap()
        .read_file(path, &mut blobs())
        .unwrap()
}

/// Generates an archive with controlled paths and contents.
fn jar(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o4750);
    for (path, bytes) in entries {
        if path.ends_with('/') {
            writer.add_directory(*path, options).unwrap();
        } else {
            writer.start_file(*path, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
    }
    let mut bytes = writer.finish().unwrap().into_inner();
    // ZipWriter masks special permission bits; set setuid directly in the independent fixture.
    for offset in 0..bytes.len().saturating_sub(45) {
        if bytes[offset..].starts_with(b"PK\x01\x02") {
            let attributes =
                u32::from_le_bytes(bytes[offset + 38..offset + 42].try_into().unwrap())
                    | (0o4000 << 16);
            bytes[offset + 38..offset + 42].copy_from_slice(&attributes.to_le_bytes());
        }
    }
    bytes
}

#[test]
fn directory_import_preserves_bytes_empty_directories_and_stable_order() {
    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir_all(temporary.path().join("nested/empty")).unwrap();
    fs::write(temporary.path().join("z.txt"), b"last").unwrap();
    fs::write(
        temporary.path().join("nested/\u{4e2d}.txt"),
        b"unicode name",
    )
    .unwrap();
    fs::write(temporary.path().join("a.txt"), b"").unwrap();
    let imported = import_path(temporary.path(), ImportOptions::default()).unwrap();
    assert_eq!(imported.jar_name, "resources.jar");
    assert!(imported.manifest.is_none());
    let mut root = imported
        .into_resource_root(BlobRef { pool: 1, index: 0 })
        .unwrap();
    assert_eq!(read(&root, "25", "nested/\u{4e2d}.txt"), b"unicode name");
    assert_eq!(read(&root, "25", "a.txt"), b"");
    assert!(matches!(
        root.merge(&context("25"), Limits::default())
            .unwrap()
            .get("nested/empty"),
        Some(Node::Directory(_))
    ));
    let first = root.encode(Limits::default()).unwrap();
    let mut again = import_path(temporary.path(), ImportOptions::default())
        .unwrap()
        .into_resource_root(BlobRef { pool: 1, index: 0 })
        .unwrap();
    assert_eq!(first, again.encode(Limits::default()).unwrap());
    let options = ImportOptions {
        max_total_bytes: 2,
        ..ImportOptions::default()
    };
    assert!(import_path(temporary.path(), options).is_err());
}

#[test]
fn multi_release_jar_maps_layers_preserves_filename_and_ignores_invalid_versions() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("original-name-1.0.jar");
    fs::write(&path, jar(&[
        ("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\nMulti-Release: true\nMain-Class: sample.Main\nClass-Path: missing.jar\n\n"),
        ("value.txt", b"base"), ("empty/", b""), ("META-INF/versions/21/value.txt", b"twenty-one"),
        ("META-INF/versions/9/value.txt", b"nine"), ("META-INF/versions/08/ignored.txt", b"ignored"),
    ])).unwrap();
    let imported = import_path(&path, ImportOptions::default()).unwrap();
    assert_eq!(imported.jar_name, "original-name-1.0.jar");
    assert_eq!(imported.layers.len(), 3);
    assert_eq!(
        imported.manifest.as_ref().unwrap().get("Class-Path"),
        Some("missing.jar")
    );
    let root = imported
        .into_resource_root(BlobRef { pool: 1, index: 0 })
        .unwrap();
    assert_eq!(read(&root, "8", "value.txt"), b"base");
    assert_eq!(read(&root, "17", "value.txt"), b"nine");
    assert_eq!(read(&root, "25", "value.txt"), b"twenty-one");
    assert_eq!(
        read(&root, "25", "META-INF/versions/08/ignored.txt"),
        b"ignored"
    );
    let tree = root.merge(&context("25"), Limits::default()).unwrap();
    let Node::File { metadata, .. } = tree.get("value.txt").unwrap() else {
        panic!("expected file")
    };
    assert_eq!(metadata.required(5).unwrap().as_u64().unwrap(), 0o4750);
    fs::write(
        &path,
        jar(&[("META-INF/versions/9/value.txt", b"ordinary resource")]),
    )
    .unwrap();
    let root = import_path(&path, ImportOptions::default())
        .unwrap()
        .into_resource_root(BlobRef { pool: 1, index: 0 })
        .unwrap();
    assert_eq!(root.layers.len(), 1);
    assert_eq!(
        read(&root, "25", "META-INF/versions/9/value.txt"),
        b"ordinary resource"
    );
}

#[test]
fn jar_import_rejects_duplicates_conflicts_bad_paths_and_crc_corruption() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("invalid.jar");
    let mut duplicate = jar(&[("a", b"first"), ("b", b"second")]);
    for offset in 0..duplicate.len().saturating_sub(46) {
        let name_offset = if duplicate[offset..].starts_with(b"PK\x03\x04") {
            Some(offset + 30)
        } else if duplicate[offset..].starts_with(b"PK\x01\x02") {
            Some(offset + 46)
        } else {
            None
        };
        if let Some(index) = name_offset
            && duplicate.get(index) == Some(&b'b')
        {
            duplicate[index] = b'a';
        }
    }
    fs::write(&path, duplicate).unwrap();
    assert!(
        import_path(&path, ImportOptions::default())
            .unwrap_err()
            .to_string()
            .contains("duplicate JAR entry")
    );
    for entries in [
        vec![("../escape", &b""[..])],
        vec![("/absolute", &b""[..])],
        vec![("a//b", &b""[..])],
        vec![("a", &b""[..]), ("a/b", &b""[..])],
    ] {
        fs::write(&path, jar(&entries)).unwrap();
        assert!(import_path(&path, ImportOptions::default()).is_err());
    }
    let mut corrupt = jar(&[("file", b"hello")]);
    let central = corrupt
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    corrupt[central + 16] ^= 1;
    fs::write(&path, corrupt).unwrap();
    assert!(import_path(&path, ImportOptions::default()).is_err());
    fs::write(
        &path,
        jar(&[
            ("META-INF/MANIFEST.MF", b"Multi-Release: true\n\n"),
            ("META-INF/versions/9/META-INF/services/test", b"provider"),
        ]),
    )
    .unwrap();
    assert!(import_path(&path, ImportOptions::default()).is_err());
}

#[test]
fn archive_links_stay_links_and_entry_limits_are_checked() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("links.jar");
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    writer.start_file("file", options).unwrap();
    writer.write_all(b"value").unwrap();
    writer.add_symlink("link", "./file", options).unwrap();
    writer
        .add_symlink("missing", "not-present", options)
        .unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    fs::write(&path, &bytes).unwrap();
    let root = import_path(&path, ImportOptions::default())
        .unwrap()
        .into_resource_root(BlobRef { pool: 1, index: 0 })
        .unwrap();
    assert_eq!(read(&root, "25", "link"), b"value");
    let tree = root.merge(&context("25"), Limits::default()).unwrap();
    assert!(matches!(
        tree.get("missing"),
        Some(Node::SymbolicLink {
            target: "not-present",
            ..
        })
    ));
    assert!(tree.resolve("missing").is_err());
    let options = ImportOptions {
        limits: Limits {
            max_elements: 2,
            ..Limits::default()
        },
        ..ImportOptions::default()
    };
    assert!(import_path(&path, options).is_err());
    for end in 0..bytes.len().min(100) {
        fs::write(&path, &bytes[..end]).unwrap();
        assert!(import_path(&path, ImportOptions::default()).is_err());
    }
}

#[test]
fn zip64_end_records_are_bounded_before_archive_index_allocation() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("zip64.jar");
    for extension_size in [0, 70000] {
        let mut bytes = b"PK\x06\x06".to_vec();
        bytes.extend_from_slice(&(44u64 + extension_size as u64).to_le_bytes());
        bytes.extend_from_slice(&[45, 0, 45, 0]);
        bytes.extend_from_slice(&[0; 40]);
        bytes.resize(bytes.len() + extension_size, 0);
        bytes.extend_from_slice(b"PK\x06\x07");
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(b"PK\x05\x06\0\0\0\0");
        bytes.extend_from_slice(&[0xff; 12]);
        bytes.extend_from_slice(&[0, 0]);
        fs::write(&path, &bytes).unwrap();
        assert!(
            import_path(&path, ImportOptions::default()).is_ok(),
            "extension {extension_size}"
        );
        bytes[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
        bytes[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        let error = import_path(&path, ImportOptions::default()).unwrap_err();
        assert!(matches!(
            error,
            janex_host::Error::Java(janex_java::Error::Limit(_))
        ));
    }
}

#[cfg(unix)]
#[test]
fn native_symbolic_links_are_not_followed_and_permissions_are_retained() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("file"), b"value").unwrap();
    fs::set_permissions(
        temporary.path().join("file"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    symlink("./file", temporary.path().join("link")).unwrap();
    symlink("absent", temporary.path().join("missing")).unwrap();
    let root = import_path(temporary.path(), ImportOptions::default())
        .unwrap()
        .into_resource_root(BlobRef { pool: 1, index: 0 })
        .unwrap();
    assert_eq!(read(&root, "25", "link"), b"value");
    let tree = root.merge(&context("25"), Limits::default()).unwrap();
    assert!(matches!(
        tree.get("link"),
        Some(Node::SymbolicLink {
            target: "./file",
            ..
        })
    ));
    let Node::File { metadata, .. } = tree.get("file").unwrap() else {
        panic!("expected file")
    };
    assert_eq!(metadata.required(5).unwrap().as_u64().unwrap(), 0o640);
    symlink("/absolute", temporary.path().join("bad")).unwrap();
    assert!(import_path(temporary.path(), ImportOptions::default()).is_err());
}

/// Invokes a JDK tool and exposes its diagnostics on failure.
fn tool(directory: &Path, program: &str, args: &[&str]) {
    let output = Command::new(program)
        .args(args)
        .current_dir(directory)
        .output()
        .expect("JDK tools must be on PATH");
    assert!(
        output.status.success(),
        "{program}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn imports_a_real_jdk_jar_with_manifest_continuations() {
    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir(temporary.path().join("files")).unwrap();
    fs::write(temporary.path().join("files/payload.txt"), b"payload").unwrap();
    fs::write(
        temporary.path().join("manifest.mf"),
        b"Manifest-Version: 1.0\r\nMain-Class: sample.\r\n Main\r\n\r\n",
    )
    .unwrap();
    tool(
        temporary.path(),
        "jar",
        &[
            "--create",
            "--file",
            "jdk-input.jar",
            "--manifest",
            "manifest.mf",
            "-C",
            "files",
            ".",
        ],
    );
    let imported = import_path(
        &temporary.path().join("jdk-input.jar"),
        ImportOptions::default(),
    )
    .unwrap();
    assert_eq!(
        imported.manifest.as_ref().unwrap().get("Main-Class"),
        Some("sample.Main")
    );
    let root = imported
        .into_resource_root(BlobRef { pool: 1, index: 0 })
        .unwrap();
    assert_eq!(read(&root, "25", "payload.txt"), b"payload");
}

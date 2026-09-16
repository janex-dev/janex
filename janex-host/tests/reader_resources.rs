// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Resource-layer and link-expansion vectors shared with the independent Java reader.

use janex_format::{
    Error, ErrorKind, Result,
    binary::{self, Limits},
    blob::{BlobRef, BlobStore, BuiltPool, Encoding},
    cbor::{self, Value},
    checksum::{Algorithm, Checksum},
    condition::Context,
    container::{APPLICATION, BLOB_POOL, Reader, Writer},
    content::{Content, Transform},
    data_pool::DataPoolBuilder,
    resource::{Node, ResourceRoot, ResourceTree},
};
use std::{
    collections::BTreeSet,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
};

/// A deliberately selected resource-name representation.
#[derive(Clone)]
enum Name {
    Index(String),
    Inline(String),
    Concat(Vec<String>),
    Raw(Vec<u8>),
}

impl Name {
    /// Encodes the chosen form without normalizing malformed fixture values.
    fn write(&self, output: &mut Vec<u8>, pool: &mut DataPoolBuilder) {
        match self {
            Self::Raw(value) => binary::write_vuint(output, pool.intern(value)).unwrap(),
            Self::Index(value) => binary::write_vuint(output, pool.intern(value)).unwrap(),
            Self::Inline(value) => {
                output.push(0);
                binary::write_sized(output, value.as_bytes()).unwrap();
            }
            Self::Concat(parts) => {
                output.extend([0, 0]);
                binary::write_vuint(&mut *output, parts.len() as u64).unwrap();
                for part in parts {
                    binary::write_vuint(&mut *output, pool.intern(part)).unwrap();
                }
            }
        }
    }
}

/// One file, link, or tombstone, including deliberately invalid references.
#[derive(Clone)]
enum EntryKind {
    File(Content),
    Link(Name),
    Tombstone,
}

/// One directory entry before encoding.
#[derive(Clone)]
struct Entry {
    name: Name,
    kind: EntryKind,
    metadata: Value,
}

/// One explicit directory, retaining caller-provided entry order.
#[derive(Clone)]
struct Directory {
    path: String,
    metadata: Value,
    entries: Vec<Entry>,
}

/// One active or inactive resource layer.
#[derive(Clone)]
struct Layer {
    active: bool,
    directories: Vec<Directory>,
}

/// Creates one regular file with inline content.
fn file(name: Name, content: &[u8]) -> Entry {
    Entry {
        name,
        kind: EntryKind::File(Content::inline(content.to_vec())),
        metadata: Value::empty_map(),
    }
}

/// Creates one symbolic link with explicit text.
fn link(name: &str, target: &str) -> Entry {
    Entry {
        name: Name::Index(name.into()),
        kind: EntryKind::Link(Name::Inline(target.into())),
        metadata: Value::empty_map(),
    }
}

/// Creates an explicit directory without metadata.
fn directory(path: &str, entries: Vec<Entry>) -> Directory {
    Directory {
        path: path.into(),
        metadata: Value::empty_map(),
        entries,
    }
}

/// Creates a matching layer.
fn layer(directories: Vec<Directory>) -> Layer {
    Layer {
        active: true,
        directories,
    }
}

/// Keeps resource-boundary vectors independent of compression-window policy.
fn stored_pool(blobs: &[Vec<u8>]) -> BuiltPool {
    let mut data = Vec::new();
    let mut page = Vec::new();
    for blob in blobs {
        let mut entry = Vec::new();
        binary::write_vuint(&mut entry, data.len() as u64).unwrap();
        Encoding {
            stored_size: blob.len() as u64,
            filters: Vec::new(),
        }
        .write(&mut entry)
        .unwrap();
        page.push(0);
        binary::write_sized(&mut page, &entry).unwrap();
        data.extend_from_slice(blob);
    }
    let type_info = Value::map([
        (Value::uint(0), Value::uint(blobs.len() as u64)),
        (Value::uint(1), Value::uint(8)),
        (
            Value::uint(2),
            Value::array([Value::array([
                Value::uint(data.len() as u64),
                Encoding {
                    stored_size: page.len() as u64,
                    filters: Vec::new(),
                }
                .to_value()
                .unwrap(),
            ])]),
        ),
    ])
    .unwrap();
    let mut bytes = b"BLOBPOOL".to_vec();
    bytes.extend(data);
    bytes.extend(page);
    BuiltPool { bytes, type_info }
}

/// Encodes a container while leaving resource validation to each implementation.
fn package(layers: &[Layer]) -> Vec<u8> {
    let mut strings = DataPoolBuilder::new();
    let mut root = vec![0, 0, 0]; // Data-pool reference and empty root metadata.
    binary::write_vuint(&mut root, layers.len() as u64).unwrap();
    for layer in layers {
        let condition = if layer.active {
            Value::empty_map()
        } else {
            Value::map([(Value::uint(1), Value::text("never-selected"))]).unwrap()
        };
        cbor::write_sized(&mut root, &condition).unwrap();
        binary::write_vuint(&mut root, layer.directories.len() as u64).unwrap();
        for directory in &layer.directories {
            binary::write_vuint(&mut root, strings.intern(&directory.path)).unwrap();
            cbor::write_sized(&mut root, &directory.metadata).unwrap();
            binary::write_vuint(&mut root, directory.entries.len() as u64).unwrap();
            let mut entries = Vec::new();
            for entry in &directory.entries {
                let tag: u32 = match entry.kind {
                    EntryKind::File(_) => 0x00534552,
                    EntryKind::Link(_) => 0x4c4d5953,
                    EntryKind::Tombstone => 0x424d4f54,
                };
                entries.extend(tag.to_le_bytes());
                entry.name.write(&mut entries, &mut strings);
                match &entry.kind {
                    EntryKind::File(content) => content.write(&mut entries).unwrap(),
                    EntryKind::Link(target) => target.write(&mut entries, &mut strings),
                    EntryKind::Tombstone => continue,
                }
                cbor::write_sized(&mut entries, &entry.metadata).unwrap();
            }
            Content::inline(entries).write(&mut root).unwrap();
        }
    }
    strings.intern(b"\xff");
    strings.intern(b"\xc0\x80");
    let pool = stored_pool(&[strings.encode().unwrap(), root]);
    let entry = Value::map([
        (Value::uint(0), Value::uint(0)),
        (Value::uint(1), BlobRef { pool: 0, index: 1 }.to_value()),
    ])
    .unwrap();
    let config = Value::map([
        (
            Value::uint(1),
            Value::map([(Value::uint(0), Value::text("Main"))]).unwrap(),
        ),
        (Value::uint(3), Value::array([entry])),
    ])
    .unwrap();
    let value = Value::map([(
        Value::uint(0),
        Value::map([(Value::uint(0), config)]).unwrap(),
    )])
    .unwrap();
    let info = Value::map([
        (Value::uint(0), Value::text("main")),
        (Value::uint(1), Value::text("janex.java")),
    ])
    .unwrap();
    let mut application = b"JANEXAPP".to_vec();
    cbor::write_sized(&mut application, &value).unwrap();
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(0, BLOB_POOL, &pool.bytes, Some(pool.type_info))
        .unwrap();
    writer
        .write_section(1, APPLICATION, &application, Some(info))
        .unwrap();
    writer.finish(Value::empty_map()).unwrap()
}

/// Restored index entries in native depth-first traversal order.
type Files = Vec<(String, Vec<u8>, Value)>;

/// Expands aliases using native resolution, preserving directory markers and checking cycles.
fn expand(
    tree: &ResourceTree<'_>,
    blobs: &mut BlobStore<Cursor<&[u8]>>,
    canonical: &str,
    alias: &str,
    active: &mut BTreeSet<String>,
    files: &mut Files,
    limits: Limits,
) -> Result<()> {
    if active.len() > limits.max_depth || !active.insert(canonical.into()) {
        return Err(Error::new(ErrorKind::Limit, "directory expansion limit"));
    }
    for (path, _) in tree.entries().filter(|(path, _)| {
        !path.is_empty() && path.rsplit_once('/').map_or("", |(parent, _)| parent) == canonical
    }) {
        let component = path.rsplit('/').next().unwrap();
        let name = if alias.is_empty() {
            component.into()
        } else {
            format!("{alias}/{component}")
        };
        limits.bytes(name.len() as u64)?;
        let (resolved, node) = tree.resolve(path)?;
        match node {
            Node::Directory(metadata) => {
                files.push((
                    format!("{name}/"),
                    Vec::new(),
                    metadata.cloned().unwrap_or_else(Value::empty_map),
                ));
                limits.elements(files.len() as u64)?;
                expand(tree, blobs, resolved, &name, active, files, limits)?;
            }
            Node::File { metadata, .. } => {
                if let Some((parent, leaf)) = name.rsplit_once('/') {
                    let leaf = leaf.to_ascii_uppercase();
                    if parent.eq_ignore_ascii_case("META-INF")
                        && (leaf.starts_with("SIG-")
                            || [".SF", ".RSA", ".DSA", ".EC"]
                                .iter()
                                .any(|suffix| leaf.ends_with(suffix)))
                    {
                        continue;
                    }
                }
                let mut bytes = tree.read_file(resolved, blobs)?;
                if name.eq_ignore_ascii_case("META-INF/MANIFEST.MF") {
                    bytes = janex_java::manifest::Manifest::parse(
                        &bytes,
                        janex_java::Limits::default(),
                    )
                    .map_err(|error| Error::new(ErrorKind::Invalid, error.to_string()))?
                    .for_runtime();
                }
                files.push((name, bytes, (*metadata).clone()));
                limits.elements(files.len() as u64)?;
            }
            Node::SymbolicLink { .. } => unreachable!(),
        }
    }
    active.remove(canonical);
    Ok(())
}

/// Writes an owned byte sequence for the Java harness.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Records native acceptance and every expanded file or directory.
fn vector(output: &mut Vec<u8>, layers: &[Layer], limits: Limits) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap());
    output[..4].copy_from_slice(&(count + 1).to_be_bytes());
    for number in [
        limits.max_bytes,
        limits.max_elements,
        limits.max_depth as u64,
    ] {
        output.extend((number as u32).to_be_bytes());
    }
    let encoded = package(layers);
    bytes(output, &encoded);
    let parsed: Result<Files> = (|| {
        let reader = Reader::open_auto(Cursor::new(encoded.as_slice()), limits)?;
        let mut blobs = BlobStore::new(reader);
        let raw = blobs.resolve(BlobRef { pool: 0, index: 1 })?;
        let root = ResourceRoot::decode(&raw, &mut blobs)?;
        let context = Context {
            os: "windows".into(),
            arch: "x86-64".into(),
            invocation: Some("run".into()),
            runtime: None,
        };
        let tree = root.merge(&context, limits)?;
        let mut files = Files::new();
        expand(
            &tree,
            &mut blobs,
            "",
            "",
            &mut BTreeSet::new(),
            &mut files,
            limits,
        )?;
        Ok(files)
    })();
    output.push(u8::from(parsed.is_ok()));
    bytes(
        output,
        parsed
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default()
            .as_bytes(),
    );
    if let Ok(files) = parsed {
        output.extend((files.len() as u32).to_be_bytes());
        for (name, content, metadata) in files {
            bytes(output, name.as_bytes());
            bytes(output, &content);
            for key in 2..=4 {
                let value = metadata.get(key).unwrap();
                output.push(u8::from(value.is_some()));
                if let Some(value) = value {
                    output.extend(value.as_i128().unwrap().to_be_bytes());
                }
            }
            let mode = metadata
                .get(5)
                .unwrap()
                .map_or(-1, |value| value.as_u64().unwrap() as i32);
            output.extend(mode.to_be_bytes());
        }
    }
}

#[test]
fn java_resource_layers_and_aliases_match_native_resolution() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/build/libs/janex-bootstrap.jar");
    let compiled = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&bootstrap)
        .arg("-d")
        .arg(temp.path())
        .arg(project.join(
            "janex-bootstrap/src/testFixtures/java/org/glavo/janex/bootstrap/loader/ResourcesTest.java",
        ))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut vectors = vec![0; 4];
    for raw in [vec![0xff], vec![0xc0, 0x80], vec![0xed, 0xa0, 0x80]] {
        vector(
            &mut vectors,
            &[layer(vec![directory(
                "",
                vec![file(Name::Raw(raw), b"invalid name")],
            )])],
            Limits::default(),
        );
    }
    let limits = Limits::default();
    vector(
        &mut vectors,
        &[layer(vec![
            directory(
                "META-INF",
                vec![file(Name::Index("test.\u{17f}f".into()), b"keep long s")],
            ),
            directory(
                "meta-\u{130}nf",
                vec![file(Name::Index("TEST.SF".into()), b"keep dotted i")],
            ),
        ])],
        limits,
    );
    let manifest = b"Manifest-Version: 1.0\r\nClass-Path: hidden.jar\r\n\r\n";
    vector(
        &mut vectors,
        &[layer(vec![
            directory("", vec![file(Name::Index("original".into()), manifest)]),
            directory(
                "META-INF",
                vec![
                    Entry {
                        name: Name::Index("BAD.SF".into()),
                        kind: EntryKind::File(Content::blob(BlobRef {
                            pool: 999,
                            index: 0,
                        })),
                        metadata: Value::empty_map(),
                    },
                    link("MANIFEST.MF", "../original"),
                ],
            ),
        ])],
        limits,
    );
    vector(
        &mut vectors,
        &[layer(vec![
            directory(
                "",
                vec![
                    file(Name::Index("a.txt".into()), b"sibling"),
                    file(Name::Index("\u{e000}".into()), b"bmp"),
                    file(Name::Index("\u{1f600}".into()), b"supplementary"),
                ],
            ),
            directory("a", vec![file(Name::Index("child".into()), b"nested")]),
        ])],
        limits,
    );
    for name in [
        "file",
        "\u{1f600}",
        "\u{e000}",
        "",
        ".",
        "..",
        "a/b",
        "a\\b",
        "nul\0name",
    ] {
        for form in [
            Name::Index(name.into()),
            Name::Inline(name.into()),
            Name::Concat(vec![name.into(), String::new()]),
        ] {
            vector(
                &mut vectors,
                &[layer(vec![directory("", vec![file(form, b"content")])])],
                limits,
            );
        }
    }
    for parts in [vec![], vec!["one"], vec!["", ""], vec!["Object", ".class"]] {
        vector(
            &mut vectors,
            &[layer(vec![directory(
                "",
                vec![file(
                    Name::Concat(parts.into_iter().map(str::to_owned).collect()),
                    b"plain",
                )],
            )])],
            limits,
        );
    }
    for path in ["", "a", "a/b", "/a", "a/", "a//b", ".", "a/../b", "a\\b"] {
        vector(
            &mut vectors,
            &[layer(vec![directory(
                path,
                vec![file(Name::Index("file".into()), b"bytes")],
            )])],
            limits,
        );
    }
    for target in [
        "dir",
        "dir/file",
        "./dir/file",
        "dir/../dir/file",
        "dir/file/..",
        "missing",
        "../dir",
        "/dir",
        "dir/",
        "dir//file",
        "alias",
        ".",
    ] {
        let dirs = vec![
            directory("", vec![link("alias", target)]),
            directory("dir", vec![file(Name::Inline("file".into()), b"resolved")]),
        ];
        vector(&mut vectors, &[layer(dirs)], limits);
    }
    let base = layer(vec![directory(
        "",
        vec![
            file(Name::Index("a".into()), b"old"),
            file(Name::Index("b".into()), b"keep"),
        ],
    )]);
    let removal = Entry {
        name: Name::Index("a".into()),
        kind: EntryKind::Tombstone,
        metadata: Value::empty_map(),
    };
    vector(
        &mut vectors,
        &[
            base.clone(),
            layer(vec![directory("", vec![removal.clone()])]),
        ],
        limits,
    );
    vector(
        &mut vectors,
        &[
            base.clone(),
            layer(vec![directory(
                "",
                vec![file(Name::Inline("a".into()), b"new")],
            )]),
        ],
        limits,
    );
    vector(
        &mut vectors,
        &[
            layer(vec![directory("a", vec![])]),
            layer(vec![directory("", vec![removal])]),
        ],
        limits,
    );
    for active in [false, true] {
        for content in [
            Content::blob(BlobRef {
                pool: 999,
                index: 0,
            }),
            Content {
                source: janex_format::content::Source::Inline(vec![0]),
                transforms: vec![Transform {
                    input_size: 8,
                    method: 1,
                    properties: Value::map([(
                        Value::uint(0),
                        BlobRef {
                            pool: 999,
                            index: 0,
                        }
                        .to_value(),
                    )])
                    .unwrap(),
                }],
            },
        ] {
            let entry = Entry {
                name: Name::Index("bad".into()),
                kind: EntryKind::File(content),
                metadata: Value::empty_map(),
            };
            vector(
                &mut vectors,
                &[
                    base.clone(),
                    Layer {
                        active,
                        directories: vec![directory("", vec![entry])],
                    },
                ],
                limits,
            );
        }
        vector(
            &mut vectors,
            &[Layer {
                active,
                directories: vec![
                    directory("", vec![file(Name::Index("a".into()), b"conflict")]),
                    directory("a/b", vec![]),
                ],
            }],
            limits,
        );
    }
    for max_elements in [5, 6, 7, 8, 16] {
        vector(
            &mut vectors,
            &[layer(vec![
                directory("a/b/c", vec![]),
                directory("d/e/f", vec![]),
            ])],
            Limits {
                max_elements,
                ..limits
            },
        );
    }
    let dirs = vec![
        directory(
            "",
            vec![
                link("a", "d"),
                link("b", &format!("{}d/file", "a/../".repeat(10))),
            ],
        ),
        directory("d", vec![file(Name::Index("file".into()), b"value")]),
    ];
    for max_depth in [6, 7, 8, 10, 12, 16] {
        vector(
            &mut vectors,
            &[layer(dirs.clone())],
            Limits {
                max_depth,
                ..limits
            },
        );
    }
    for text in [
        "abcdefgh",
        "\u{4e2d}\u{6587}\u{5185}\u{5bb9}",
        "\u{1f600}\u{1f60a}",
    ] {
        for repeats in [32, 64, 128] {
            let name = Name::Concat(vec![text.into(); repeats]);
            for max_bytes in [512, 1024, 2048, 4096] {
                vector(
                    &mut vectors,
                    &[Layer {
                        active: false,
                        directories: vec![directory("", vec![file(name.clone(), b"value")])],
                    }],
                    Limits {
                        max_bytes,
                        ..limits
                    },
                );
            }
        }
    }
    let mut metadata_values = vec![
        Value::null(),
        Value::boolean(false),
        Value::text(""),
        Value::text("Comment"),
        Value::uint(0),
        Value::uint(4095),
        Value::uint(4096),
        Value::uint(u64::MAX),
        Value::integer(-1),
        Value::integer(i128::MIN),
        Value::integer(i128::MAX),
        Value::integer(janex_format::resource::MIN_TIMESTAMP_NANOS),
        Value::integer(janex_format::resource::MAX_TIMESTAMP_NANOS),
        Value::integer(janex_format::resource::MIN_TIMESTAMP_NANOS - 1),
        Value::integer(janex_format::resource::MAX_TIMESTAMP_NANOS + 1),
        Value::integer(-1_000_000_001),
        Value::integer(u64::MAX as i128 + 1),
        Value::integer(-(u64::MAX as i128) - 2),
        Value::bytes(&[]),
        Value::bytes(&[0x11]),
        Value::bytes(&[0; 9]),
    ];
    for tag in [2, 3, 4] {
        for length in [0, 8, 9, 16, 17] {
            for first in [0, 1, 0x7f, 0x80, 0xff] {
                let mut value = vec![0xc0 | tag, 0x40 | length];
                value.extend(vec![first; length as usize]);
                metadata_values.push(Value::from_bytes(&value, limits).unwrap());
            }
        }
    }
    for algorithm in [
        Algorithm::Xxh3_64,
        Algorithm::Xxh3_128,
        Algorithm::Sha256,
        Algorithm::Sha512,
        Algorithm::Sm3,
    ] {
        metadata_values.push(Value::bytes(
            &Checksum::compute(algorithm, b"value".as_slice())
                .unwrap()
                .encode(),
        ));
    }
    for key in 0..=6 {
        for value in &metadata_values {
            let metadata = Value::map([(Value::uint(key), value.clone())]).unwrap();
            for active in [false, true] {
                for kind in 0..3 {
                    let mut dirs = vec![directory(
                        "",
                        vec![file(Name::Index("target".into()), b"value")],
                    )];
                    if kind == 0 {
                        let mut entry = file(Name::Index("subject".into()), b"value");
                        entry.metadata = metadata.clone();
                        dirs[0].entries.push(entry);
                    } else if kind == 1 {
                        let mut dir = directory("subject", vec![]);
                        dir.metadata = metadata.clone();
                        dirs.push(dir);
                    } else {
                        let mut entry = link("subject", "target");
                        entry.metadata = metadata.clone();
                        dirs[0].entries.push(entry);
                    }
                    vector(
                        &mut vectors,
                        &[Layer {
                            active,
                            directories: dirs,
                        }],
                        limits,
                    );
                }
            }
        }
    }
    // Manifest checksums cover the original content, including through an expanded alias.
    let manifest = b"Manifest-Version: 1.0\r\nClass-Path: ignored.jar\r\n\r\n";
    for algorithm in [
        Algorithm::Xxh3_64,
        Algorithm::Xxh3_128,
        Algorithm::Sha256,
        Algorithm::Sha512,
        Algorithm::Sm3,
    ] {
        for corrupt in [false, true] {
            let mut checksum = Checksum::compute(algorithm, manifest.as_slice())
                .unwrap()
                .encode();
            if corrupt {
                *checksum.last_mut().unwrap() ^= 1;
            }
            let mut entry = file(Name::Index("original".into()), manifest);
            entry.metadata = Value::map([(Value::uint(0), Value::bytes(&checksum))]).unwrap();
            vector(
                &mut vectors,
                &[layer(vec![
                    directory("", vec![entry]),
                    directory("META-INF", vec![link("MANIFEST.MF", "../original")]),
                ])],
                limits,
            );
        }
    }
    let fixture = temp.path().join("resources.bin");
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
            .arg("org.glavo.janex.bootstrap.loader.ResourcesTest")
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

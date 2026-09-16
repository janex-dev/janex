// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Resource wire fixtures, layer conflicts, metadata, and symbolic-link boundaries.

use janex_format::{
    ErrorKind,
    binary::Limits,
    blob::{BlobRef, BlobStore, PoolBuilder},
    cbor::Value,
    checksum::{Algorithm, Checksum},
    condition::{Condition, Context},
    container::{BLOB_POOL, Reader, Writer},
    content::Content,
    data_pool::{DataPool, DataPoolBuilder},
    resource::{Directory, DirectoryEntry, Layer, Node, ResourceRoot, ValidatedRoot},
};
use std::io::Cursor;

/// Builds a checked container holding the supplied blobs in pool 1.
fn store(values: &[Vec<u8>]) -> BlobStore<Cursor<Vec<u8>>> {
    let mut pool = PoolBuilder::new();
    for value in values {
        pool.push(value, 3).unwrap();
    }
    let pool = pool.finish(8, 3).unwrap();
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(1, BLOB_POOL, &pool.bytes, Some(pool.type_info))
        .unwrap();
    let mut reader = Reader::open_auto(
        Cursor::new(writer.finish(Value::empty_map()).unwrap()),
        Limits::default(),
    )
    .unwrap();
    reader.verify_checksums().unwrap();
    BlobStore::new(reader)
}

/// Supplies a resource-only context without an application invocation or runtime.
fn context() -> Context {
    Context {
        os: "linux".into(),
        arch: "x86-64".into(),
        invocation: None,
        runtime: None,
    }
}

/// Creates an inline file with empty metadata.
fn file(name: &str, bytes: &[u8]) -> DirectoryEntry {
    DirectoryEntry::File {
        name: name.into(),
        content: Content::inline(bytes.into()),
        metadata: Value::empty_map(),
    }
}

/// Creates a symbolic link with empty metadata.
fn link(name: &str, target: &str) -> DirectoryEntry {
    DirectoryEntry::SymbolicLink {
        name: name.into(),
        target: target.into(),
        metadata: Value::empty_map(),
    }
}

/// Creates a tombstone for one direct entry.
fn tombstone(name: &str) -> DirectoryEntry {
    DirectoryEntry::Tombstone { name: name.into() }
}

/// Creates a directory, sorting its direct entries as required by the wire format.
fn directory(path: &str, mut entries: Vec<DirectoryEntry>) -> Directory {
    entries.sort_by(|a, b| a.name().cmp(b.name()));
    Directory {
        path: path.into(),
        entries,
        metadata: Value::empty_map(),
    }
}

/// Creates an unconditional layer with sorted directory paths.
fn layer(mut directories: Vec<Directory>) -> Layer {
    directories.sort_by(|a, b| a.path.cmp(&b.path));
    Layer {
        condition: Condition::unconditional(),
        directories,
    }
}

/// Creates a root whose pool will occupy blob zero.
fn root(layers: Vec<Layer>) -> ResourceRoot {
    ResourceRoot {
        data_pool: BlobRef { pool: 1, index: 0 },
        data: DataPool::new(),
        metadata: Value::empty_map(),
        layers,
    }
}

#[test]
fn encoding_failure_retains_the_pool_and_can_be_retried() {
    let mut root = root(vec![layer(vec![directory("p", vec![file("f", &[7; 64])])])]);
    let mut data = DataPoolBuilder::new();
    assert_eq!(data.intern("existing"), 1);
    root.data = data.finish();
    let limits = Limits {
        max_bytes: 16,
        ..Limits::default()
    };
    assert!(root.encode(limits).is_err());
    assert_eq!(root.data.get(1).unwrap(), b"existing");
    assert_eq!(root.data.get(2).unwrap(), b"p");
    assert_eq!(root.data.get(3).unwrap(), b"f");
    let encoded = root.encode(Limits::default()).unwrap();
    let pool = root.data.encode().unwrap();
    assert_eq!(root.encode(Limits::default()).unwrap(), encoded);
    assert_eq!(root.data.encode().unwrap(), pool);
}

#[test]
fn independent_root_bytes_read_inline_and_blob_backed_directory_entries() {
    // Inline name "foo", inline content "hello", no transforms or metadata.
    let entries = b"RES\0\0\x03foo\0\x05hello\0\0".to_vec();
    let mut blobs = store(&[b"\x01\x00".to_vec(), entries.clone()]);
    let mut inline = vec![1, 0, 0, 1, 0, 1, 0, 0, 1, 0, entries.len() as u8];
    inline.extend_from_slice(&entries);
    inline.push(0);
    let root = ResourceRoot::decode(&inline, &mut blobs).unwrap();
    let tree = root.merge(&context(), Limits::default()).unwrap();
    assert_eq!(tree.read_file("foo", &mut blobs).unwrap(), b"hello");
    // Same entry array in blob 1, with no transform.
    let referenced = [1, 0, 0, 1, 0, 1, 0, 0, 1, 1, 1, 1, 0];
    let root = ResourceRoot::decode(&referenced, &mut blobs).unwrap();
    assert_eq!(
        root.merge(&context(), Limits::default())
            .unwrap()
            .read_file("foo", &mut blobs)
            .unwrap(),
        b"hello"
    );
    for end in 0..inline.len() {
        assert!(
            ResourceRoot::decode(&inline[..end], &mut blobs).is_err(),
            "prefix {end}"
        );
    }
    let mut wrong_count = inline.clone();
    wrong_count[8] = 0;
    assert!(ResourceRoot::decode(&wrong_count, &mut blobs).is_err());
    let mut transformed_entries = referenced.to_vec();
    transformed_entries.pop();
    transformed_entries.extend_from_slice(&[1, 18, 1, 0]);
    assert!(ResourceRoot::decode(&transformed_entries, &mut blobs).is_err());
    inline.push(0);
    assert!(ResourceRoot::decode(&inline, &mut blobs).is_err());
}

#[test]
fn root_round_trip_shares_names_and_retains_metadata() {
    let checksum = Checksum::compute(Algorithm::Sha256, b"class bytes".as_slice()).unwrap();
    let metadata = Value::map([
        (Value::uint(0), Value::bytes(&checksum.encode())),
        (Value::uint(1), Value::text("")),
        (
            Value::uint(2),
            Value::integer(janex_format::resource::MIN_TIMESTAMP_NANOS),
        ),
        (
            Value::uint(3),
            Value::integer(janex_format::resource::MAX_TIMESTAMP_NANOS),
        ),
        (Value::uint(4), Value::integer(-1)),
        (Value::uint(5), Value::uint(0o755)),
        (Value::uint(100), Value::text("extension")),
    ])
    .unwrap();
    let mut root = root(vec![layer(vec![
        directory(
            "java/lang",
            vec![DirectoryEntry::File {
                name: "Object.class".into(),
                content: Content::blob(BlobRef { pool: 1, index: 1 }),
                metadata: metadata.clone(),
            }],
        ),
        directory("empty", vec![]),
    ])]);
    let mut data = DataPoolBuilder::new();
    data.intern("Object");
    root.data = data.finish();
    root.metadata = Value::map([
        (
            Value::text("janex.java.jar_name"),
            Value::text("original-name.jar"),
        ),
        (Value::text("extension"), Value::null()),
    ])
    .unwrap();
    let encoded = root.encode(Limits::default()).unwrap();
    assert!((0..root.data.len()).all(|index| root.data.get(index).unwrap() != b"Object.class"));
    let mut blobs = store(&[root.data.encode().unwrap(), b"class bytes".to_vec()]);
    let mut decoded = ResourceRoot::decode(&encoded, &mut blobs).unwrap();
    assert_eq!(decoded.encode(Limits::default()).unwrap(), encoded);
    assert_eq!(decoded.metadata, root.metadata);
    assert_eq!(decoded.jar_name().unwrap(), "original-name.jar");
    let tree = decoded.merge(&context(), Limits::default()).unwrap();
    assert_eq!(
        tree.read_file("java/lang/Object.class", &mut blobs)
            .unwrap(),
        b"class bytes"
    );
    assert!(matches!(tree.get("java"), Some(Node::Directory(None))));
    assert!(matches!(tree.get("empty"), Some(Node::Directory(Some(_)))));
    assert!(
        matches!(tree.get("java/lang/Object.class"), Some(Node::File { metadata: value, .. }) if **value == metadata)
    );
    let mut corrupted = store(&[root.data.encode().unwrap(), b"other bytes".to_vec()]);
    assert_eq!(
        tree.read_file("java/lang/Object.class", &mut corrupted)
            .unwrap_err()
            .kind(),
        ErrorKind::Verification
    );
}

/// Frozen roots keep structural validation while respecting stricter callers and new contexts.
#[test]
fn validated_roots_preserve_limits_and_conditional_conflict_checks() {
    let limits = Limits::default();
    let mut editable = root(vec![layer(vec![directory(
        "a/b",
        vec![file("x", b"value")],
    )])]);
    let encoded = editable.encode(limits).unwrap();
    let mut blobs = store(&[editable.data.encode().unwrap()]);
    let frozen = ValidatedRoot::decode(&encoded, &mut blobs).unwrap();
    let tree = frozen.merge(&context(), limits).unwrap();
    assert_eq!(tree.read_file("a/b/x", &mut blobs).unwrap(), b"value");
    for limit in [
        Limits {
            max_bytes: 2,
            ..limits
        },
        Limits {
            max_elements: 1,
            ..limits
        },
    ] {
        assert!(frozen.merge(&context(), limit).is_err());
    }
    let mut overlay = layer(vec![directory("a/b", vec![])]);
    overlay.condition =
        Condition::from_value(Value::map([(Value::uint(1), Value::text("windows"))]).unwrap())
            .unwrap();
    let frozen = ValidatedRoot::new(
        root(vec![
            layer(vec![directory("", vec![file("a", b"")])]),
            overlay,
        ]),
        limits,
    )
    .unwrap();
    assert!(frozen.merge(&context(), limits).is_ok());
    let mut windows = context();
    windows.os = "windows".into();
    assert!(frozen.merge(&windows, limits).is_err());
    let mut invalid = layer(vec![directory("../escape", vec![])]);
    invalid.condition =
        Condition::from_value(Value::map([(Value::uint(1), Value::text("windows"))]).unwrap())
            .unwrap();
    assert!(ValidatedRoot::new(root(vec![invalid]), limits).is_err());
}

#[test]
fn tombstones_precede_additions_and_directory_metadata_replaces() {
    let mut base = directory(
        "",
        vec![
            file("old", b"old"),
            file("replace", b"old"),
            file("turn", b"file"),
        ],
    );
    base.metadata = Value::map([(Value::uint(1), Value::text("old"))]).unwrap();
    let mut replacement = directory(
        "",
        vec![
            tombstone("old"),
            file("replace", b"new"),
            tombstone("turn"),
            tombstone("unknown"),
        ],
    );
    replacement.metadata = Value::map([(Value::uint(5), Value::uint(0))]).unwrap();
    let root = root(vec![
        layer(vec![base]),
        layer(vec![
            replacement,
            directory("turn/child", vec![file("nested", b"ok")]),
        ]),
    ]);
    let tree = root.merge(&context(), Limits::default()).unwrap();
    assert!(tree.get("old").is_none());
    assert!(tree.get("unknown").is_none());
    let mut blobs = store(&[]);
    assert_eq!(tree.read_file("replace", &mut blobs).unwrap(), b"new");
    assert_eq!(
        tree.read_file("turn/child/nested", &mut blobs).unwrap(),
        b"ok"
    );
    assert!(
        matches!(tree.get(""), Some(Node::Directory(Some(value))) if value.get(1).unwrap().is_none() && value.required(5).unwrap().as_u64().unwrap() == 0)
    );
}

#[test]
fn directory_conflicts_and_malformed_unmatched_layers_are_rejected() {
    for layers in [
        vec![layer(vec![
            directory("", vec![file("a", b"")]),
            directory("a/b", vec![]),
        ])],
        vec![
            layer(vec![directory("", vec![file("a", b"")])]),
            layer(vec![directory("a/b", vec![])]),
        ],
        vec![
            layer(vec![directory("a", vec![])]),
            layer(vec![directory("", vec![file("a", b"")])]),
        ],
        vec![
            layer(vec![directory("a", vec![])]),
            layer(vec![directory("", vec![tombstone("a")])]),
            layer(vec![directory("", vec![file("a", b"")])]),
        ],
    ] {
        assert!(root(layers).merge(&context(), Limits::default()).is_err());
    }
    let mut unmatched = layer(vec![directory("bad/../path", vec![])]);
    unmatched.condition =
        Condition::from_value(Value::map([(Value::uint(1), Value::text("windows"))]).unwrap())
            .unwrap();
    assert!(
        root(vec![unmatched])
            .merge(&context(), Limits::default())
            .is_err()
    );
    let mut unmatched = layer(vec![directory("", vec![file("a", b"new")])]);
    unmatched.condition =
        Condition::from_value(Value::map([(Value::uint(1), Value::text("windows"))]).unwrap())
            .unwrap();
    let root = root(vec![
        layer(vec![directory("", vec![file("a", b"old")])]),
        unmatched,
    ]);
    assert_eq!(
        root.merge(&context(), Limits::default())
            .unwrap()
            .read_file("a", &mut store(&[]))
            .unwrap(),
        b"old"
    );
}

#[test]
fn links_follow_components_before_parent_navigation() {
    let root = root(vec![layer(vec![
        directory(
            "",
            vec![
                file("file", b"value"),
                link("alias", "nested/deep"),
                link("chain", "alias/../sibling"),
                link("cycle-a", "cycle-b"),
                link("cycle-b", "cycle-a"),
                link("dangling", "absent"),
                link("escape", "nested/up/.."),
                link("nondir", "file/.."),
                link("self", "self"),
                link("top", "."),
            ],
        ),
        directory(
            "nested",
            vec![file("sibling", b"sibling"), link("up", "..")],
        ),
        directory("nested/deep", vec![link("file", "../../file")]),
    ])]);
    let tree = root.merge(&context(), Limits::default()).unwrap();
    let mut blobs = store(&[]);
    assert_eq!(
        tree.resolve("alias/../sibling").unwrap().0,
        "nested/sibling"
    );
    assert_eq!(tree.read_file("chain", &mut blobs).unwrap(), b"sibling");
    assert_eq!(tree.read_file("alias/file", &mut blobs).unwrap(), b"value");
    assert_eq!(tree.resolve("top").unwrap().0, "");
    for path in [
        "dangling",
        "escape",
        "nondir",
        "file/..",
        "../file",
        "/file",
        "file/",
        "nested//sibling",
    ] {
        assert!(tree.resolve(path).is_err(), "{path}");
    }
    for path in ["self", "cycle-a"] {
        assert_eq!(tree.resolve(path).unwrap_err().kind(), ErrorKind::Limit);
    }
}

#[test]
fn resource_metadata_is_checked_for_each_node_kind() {
    for (key, value) in [
        (5, Value::uint(4096)),
        (5, Value::integer(-1)),
        (1, Value::uint(0)),
        (2, Value::text("0")),
    ] {
        let mut entry = file("file", b"");
        let DirectoryEntry::File { metadata, .. } = &mut entry else {
            unreachable!()
        };
        *metadata = Value::map([(Value::uint(key), value)]).unwrap();
        assert!(
            root(vec![layer(vec![directory("", vec![entry])])])
                .merge(&context(), Limits::default())
                .is_err()
        );
    }
    let checksum = Value::bytes(
        &Checksum::compute(Algorithm::Sha256, b"".as_slice())
            .unwrap()
            .encode(),
    );
    for (key, value) in [(5, Value::uint(0)), (0, checksum.clone())] {
        let mut entry = link("link", "target");
        let DirectoryEntry::SymbolicLink { metadata, .. } = &mut entry else {
            unreachable!()
        };
        *metadata = Value::map([(Value::uint(key), value)]).unwrap();
        assert!(
            root(vec![layer(vec![directory("", vec![entry])])])
                .merge(&context(), Limits::default())
                .is_err()
        );
    }
    let mut dir = directory("", vec![]);
    dir.metadata = Value::map([(Value::uint(0), checksum)]).unwrap();
    assert!(
        root(vec![layer(vec![dir])])
            .merge(&context(), Limits::default())
            .is_err()
    );
    for name in ["../app.jar", "dir\\app.jar", "app.jar\0", "app.zip"] {
        let mut root = root(vec![]);
        root.metadata =
            Value::map([(Value::text("janex.java.jar_name"), Value::text(name))]).unwrap();
        assert!(root.encode(Limits::default()).is_err(), "{name}");
    }
}

#[test]
fn resource_timestamps_are_bounded_by_instant_range() {
    use janex_format::resource::{MAX_TIMESTAMP_NANOS, MIN_TIMESTAMP_NANOS};
    for (time, valid) in [
        (MIN_TIMESTAMP_NANOS, true),
        (MAX_TIMESTAMP_NANOS, true),
        (-1, true),
        (-1_000_000_001, true),
        (0, true),
        (MIN_TIMESTAMP_NANOS - 1, false),
        (MAX_TIMESTAMP_NANOS + 1, false),
        (i128::MIN, false),
        (i128::MAX, false),
    ] {
        for key in 2..=4 {
            let mut dir = directory("", vec![]);
            dir.metadata = Value::map([(Value::uint(key), Value::integer(time))]).unwrap();
            assert_eq!(
                root(vec![layer(vec![dir])])
                    .encode(Limits::default())
                    .is_ok(),
                valid,
                "{key}: {time}"
            );
        }
    }
}

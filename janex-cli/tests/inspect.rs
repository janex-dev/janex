// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Structure inspection across wrappers, inactive declarations, corruption, and unsigned trust.

use janex_format::{
    application::{Application, PathEntry},
    binary::Limits,
    blob::{BlobRef, BlobStore, Entry, Extent, PoolBuilder},
    cbor::Value,
    condition::Condition,
    container::{APPLICATION, BLOB_POOL, Reader, Writer},
    content::Content,
    resource::{Directory, DirectoryEntry, Layer, ResourceRoot},
    strings::StringPool,
};
use serde_json::Value as Json;
use std::{
    fs,
    io::Cursor,
    path::Path,
    process::{Command, Output},
};

/// Uses IDs outside JavaScript's exact integer range and unrelated to section positions.
const POOL: u64 = u64::MAX - 2;

/// Builds an integer-keyed CBOR map.
fn map(fields: impl IntoIterator<Item = (u64, Value)>) -> Value {
    Value::map(
        fields
            .into_iter()
            .map(|(key, value)| (Value::uint(key), value)),
    )
    .unwrap()
}

/// Creates a package with conditional layers, extents, unknown metadata, and dormant roots.
fn fixture(signed: bool, missing_root: bool) -> Vec<u8> {
    let mut root = ResourceRoot {
        string_pool: BlobRef {
            pool: POOL,
            index: 0,
        },
        strings: StringPool::new(),
        metadata: Value::map([(
            Value::text("janex.java.jar_name"),
            Value::text("sample.jar"),
        )])
        .unwrap(),
        layers: vec![
            Layer {
                condition: Condition::unconditional(),
                directories: vec![Directory {
                    path: String::new(),
                    metadata: Value::empty_map(),
                    entries: vec![DirectoryEntry::File {
                        name: "sample.txt".into(),
                        content: Content::blob(BlobRef {
                            pool: POOL,
                            index: 3,
                        }),
                        metadata: map([(100, Value::integer(i128::MIN))]),
                    }],
                }],
            },
            Layer {
                condition: Condition::from_value(map([(1, Value::text("nonexistent-os"))]))
                    .unwrap(),
                directories: vec![Directory {
                    path: String::new(),
                    metadata: Value::empty_map(),
                    entries: vec![
                        DirectoryEntry::SymbolicLink {
                            name: "alias".into(),
                            target: "sample.txt".into(),
                            metadata: Value::empty_map(),
                        },
                        DirectoryEntry::Tombstone {
                            name: "sample.txt".into(),
                        },
                    ],
                }],
            },
        ],
    };
    let encoded = root.encode(Limits::default()).unwrap();
    let mut pool = PoolBuilder::new();
    pool.push(&root.strings.encode().unwrap(), 3).unwrap();
    pool.push(&encoded, 3).unwrap();
    pool.push(&vec![b'x'; 4096], 3).unwrap();
    pool.push_extents(vec![Extent {
        stored_blob_index: 2,
        decoded_offset: 17,
        decoded_length: 31,
    }])
    .unwrap();
    pool.push(b"unreferenced", 3).unwrap();
    let pool = pool.finish(8, 3).unwrap();
    let local = PathEntry::Local(BlobRef {
        pool: POOL,
        index: if missing_root { 999 } else { 1 },
    })
    .to_value(false)
    .unwrap();
    let remote = PathEntry::External {
        uri: "https://127.0.0.1:1/no-download.jar".into(),
        checksum: None,
    }
    .to_value(false)
    .unwrap();
    let config = map([
        (1, map([(0, Value::text("example.Main"))])),
        (3, Value::array([remote])),
        (
            6,
            Value::array([
                map([
                    (0, map([(1, Value::text("nonexistent-os"))])),
                    (2, Value::array([local.clone()])),
                    (3, Value::array([local.clone()])),
                    (4, Value::array([map([(0, local), (1, Value::text(""))])])),
                ]),
                map([(3, Value::null())]),
            ]),
        ),
    ]);
    let app = Application::from_values(
        map([(0, Value::text("main")), (1, Value::text("janex.java"))]),
        map([(0, map([(0, config)])), (100, Value::uint(u64::MAX))]),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(app.resource_references().len(), 1);
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(POOL, BLOB_POOL, &pool.bytes, Some(pool.type_info))
        .unwrap();
    writer
        .write_section(
            3,
            APPLICATION,
            &app.encode().unwrap(),
            Some(app.type_info().clone()),
        )
        .unwrap();
    writer
        .write_section(
            4,
            12345,
            b"unknown section",
            Some(map([(100, Value::bytes(b"extension"))])),
        )
        .unwrap();
    if signed {
        writer
            .finish_with::<janex_format::Error>(Value::empty_map(), 3, |_| {
                Ok(b"opaque unverified signature".to_vec())
            })
            .unwrap()
    } else {
        writer.finish(Value::empty_map()).unwrap()
    }
}

/// Runs the real executable without a shell or any launch policy.
fn inspect(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_janex"))
        .arg("inspect")
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}

/// Parses successful machine output, reporting stderr when the operation fails.
fn json(output: Output) -> Json {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn reports_unmerged_layers_exact_ids_ranges_and_all_blobs() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.janex");
    let mut bytes = b"prefix".to_vec();
    bytes.extend(fixture(false, false));
    // An independent empty ZIP EOCD exercises appended-JAR boundary discovery.
    bytes.extend_from_slice(b"PK\x05\x06");
    bytes.extend_from_slice(&[0; 18]);
    fs::write(&path, &bytes).unwrap();
    let report = json(inspect(
        &path,
        &["--sections", "--blobs", "--strings", "--json"],
    ));
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["external_prefix_size"], "6");
    assert_eq!(report["external_tail_size"], "22");
    assert_eq!(report["sections"][0]["id"], POOL.to_string());
    assert_eq!(report["sections"][0]["offset"], "14");
    assert_eq!(report["sections"][2]["kind"], "unknown");
    assert_eq!(
        report["verification"]["container_checksums"]["status"],
        "not_checked"
    );
    let pool = &report["blob_pools"][0];
    assert_eq!(pool["count"], "5");
    assert_eq!(pool["entries"][3]["kind"], "extents");
    assert_eq!(pool["entries"][3]["extents"][0]["decoded_offset"], "17");
    assert!(pool["entries"][3].get("offset").is_none());
    assert_eq!(pool["entries"][4]["kind"], "stored");
    assert_eq!(pool["pages"][0]["checksum_status"], "passed");
    let mut store =
        BlobStore::new(Reader::open_auto(Cursor::new(&bytes), Limits::default()).unwrap());
    let info = store.pool_info(POOL).unwrap();
    assert_eq!(info.payload_offset, 22);
    assert_eq!(
        pool["pages"][0]["offset"],
        (22 + info.pages[0].offset).to_string()
    );
    let Entry::Stored { offset, .. } = store
        .entry(BlobRef {
            pool: POOL,
            index: 2,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(pool["entries"][2]["offset"], (22 + offset).to_string());
    let roots = report["resource_roots"].as_array().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0]["layers"].as_array().unwrap().len(), 2);
    assert_eq!(
        roots[0]["layers"][1]["directories"][0]["entries"][0]["kind"],
        "symbolic_link"
    );
    assert_eq!(
        roots[0]["layers"][1]["directories"][0]["entries"][1]["kind"],
        "tombstone"
    );
    assert_eq!(report["string_pools"][0]["strings"][0], "");
    assert!(report.to_string().contains(&u64::MAX.to_string()));
    assert!(report.to_string().contains("cbor_hex"));
    let output = inspect(&path, &["--sections", "--blobs", "--strings"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Section ",
        "Pool ",
        "Resource root ",
        "Layer 1",
        "String pool ",
        "signature: not_present",
    ] {
        assert!(text.contains(expected), "{expected}");
    }
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn summary_stays_lazy_and_signatures_are_never_implied_by_checksum_success() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("signed.janex");
    fs::write(&path, fixture(true, true)).unwrap();
    let report = json(inspect(&path, &["--verify", "--json"]));
    assert_eq!(report["verification"]["type"], "cms");
    assert_eq!(report["verification"]["signature"], "not_checked");
    assert_eq!(
        report["verification"]["container_checksums"]["status"],
        "passed"
    );
    assert_eq!(
        report["verification"]["container_checksums"]["complete_secure_coverage"],
        false
    );
    assert!(report.get("sections").is_none());
    assert!(report.get("blob_pools").is_none());
    assert!(report.get("resource_roots").is_none());
    let output = inspect(&path, &["--resources", "--json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("blob index exceeds pool count"));
}

#[test]
fn payload_corruption_is_not_read_by_structure_inspection_but_verify_fails() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("corrupt.janex");
    let mut bytes = fixture(false, false);
    let mut store =
        BlobStore::new(Reader::open_auto(Cursor::new(&bytes), Limits::default()).unwrap());
    let info = store.pool_info(POOL).unwrap();
    let Entry::Stored { offset, .. } = store
        .entry(BlobRef {
            pool: POOL,
            index: 2,
        })
        .unwrap()
    else {
        panic!()
    };
    let position = info.payload_offset + offset;
    drop(store);
    bytes[position as usize] ^= 0xff;
    fs::write(&path, &bytes).unwrap();
    json(inspect(&path, &["--blobs", "--resources", "--json"]));
    let output = inspect(&path, &["--verify", "--json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum"));
}

#[test]
fn pool_metadata_is_lazy_but_requested_corrupt_table_pages_fail() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bad-page.janex");
    let mut bytes = fixture(false, false);
    let mut store =
        BlobStore::new(Reader::open_auto(Cursor::new(&bytes), Limits::default()).unwrap());
    let info = store.pool_info(POOL).unwrap();
    let position = info.payload_offset + info.pages[0].offset;
    drop(store);
    bytes[position as usize] ^= 0xff;
    let mut store =
        BlobStore::new(Reader::open_auto(Cursor::new(&bytes), Limits::default()).unwrap());
    assert_eq!(store.pool_info(POOL).unwrap().count, 5);
    assert!(
        store
            .entry(BlobRef {
                pool: POOL,
                index: 0
            })
            .is_err()
    );
    fs::write(&path, &bytes).unwrap();
    json(inspect(&path, &["--json"]));
    let output = inspect(&path, &["--blobs", "--json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn arbitrary_tails_require_explicit_length_and_truncated_inputs_fail() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("tail.janex");
    let mut bytes = fixture(false, false);
    bytes.extend_from_slice(b"tail");
    fs::write(&path, &bytes).unwrap();
    assert!(!inspect(&path, &[]).status.success());
    let report = json(inspect(&path, &["--external-tail-length", "4", "--json"]));
    assert_eq!(report["external_tail_size"], "4");
    fs::write(&path, &bytes[..20]).unwrap();
    let output = inspect(&path, &["--json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

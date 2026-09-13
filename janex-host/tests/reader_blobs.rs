// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Blob page validation, wide logical indices, and deferred decoding limits.

use janex_format::{
    binary::{self, Limits},
    blob::{BlobRef, BlobStore, Encoding, Filter},
    cbor::{self, Value},
    checksum::{Algorithm, Checksum},
    condition::Context,
    container::{APPLICATION, BLOB_POOL, Reader, Writer},
    content::Content,
    resource::ResourceRoot,
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
};

/// Writes a trusted harness byte string.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Encodes a sequence of ULEB128 fields without schema validation.
fn integers(values: &[u64]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in values {
        binary::write_vuint(&mut bytes, *value).unwrap();
    }
    bytes
}

/// Encodes one Stored entry payload.
fn stored(offset: u64, size: u64, decoded: Option<u64>) -> Vec<u8> {
    let mut bytes = integers(&[offset]);
    Encoding {
        stored_size: size,
        filters: decoded
            .map(|input_size| Filter {
                input_size,
                method: 1,
                properties: Value::empty_map(),
            })
            .into_iter()
            .collect(),
    }
    .write(&mut bytes)
    .unwrap();
    bytes
}

/// Creates a pool with referenced structural blobs and an optional unselected test entry.
fn package(
    count: u64,
    shift: u8,
    extra: &(u8, Vec<u8>),
    select: bool,
    page_checksum: Option<Value>,
    physical: &[u8],
) -> Vec<u8> {
    let base = if count > 3 { count - 2 } else { 0 };
    let mut root = integers(&[0, base]);
    root.extend([0, u8::from(select)]);
    if select {
        root.extend([0, 1, 0, 0, 1]); // One unconditional layer and its root directory.
        let mut entry = 0x00534552u32.to_le_bytes().to_vec();
        entry.push(0);
        binary::write_sized(&mut entry, b"file").unwrap();
        Content::blob(BlobRef { pool: 0, index: 2 })
            .write(&mut entry)
            .unwrap();
        entry.push(0);
        Content::inline(entry).write(&mut root).unwrap();
    }
    let mut data = vec![1, 0]; // One empty StringPool entry.
    data.extend(&root);
    data.extend(physical);
    let mut pages = Vec::new();
    for first in (0..count).step_by(1 << shift) {
        let mut page = Vec::new();
        // A wide-index probe leaves unrequested pages empty; only the last page is read.
        let end = if count > i32::MAX as u64 && first + (1 << shift) <= base {
            first
        } else {
            count.min(first + (1 << shift))
        };
        for index in first..end {
            let (tag, payload) = if index == base {
                (0, stored(0, 2, None))
            } else if index == base + 1 {
                (0, stored(2, root.len() as u64, None))
            } else if index == 2 && base == 0 {
                extra.clone()
            } else {
                (255, Vec::new())
            };
            page.push(tag);
            binary::write_sized(&mut page, &payload).unwrap();
        }
        let mut descriptor = vec![
            Value::uint(data.len() as u64),
            Encoding {
                stored_size: page.len() as u64,
                filters: Vec::new(),
            }
            .to_value()
            .unwrap(),
        ];
        if first == 0
            && let Some(checksum) = &page_checksum
        {
            descriptor.push(checksum.clone());
        }
        pages.push(Value::array(descriptor));
        data.extend(page);
    }
    let info = Value::map([
        (Value::uint(0), Value::uint(count)),
        (Value::uint(1), Value::uint(shift.into())),
        (Value::uint(2), Value::array(pages)),
    ])
    .unwrap();
    let mut pool = b"BLOBPOOL".to_vec();
    pool.extend(data);
    let path = Value::map([
        (Value::uint(0), Value::uint(0)),
        (
            Value::uint(1),
            BlobRef {
                pool: 0,
                index: base + 1,
            }
            .to_value(),
        ),
    ])
    .unwrap();
    let config = Value::map([
        (
            Value::uint(1),
            Value::map([(Value::uint(0), Value::text("Main"))]).unwrap(),
        ),
        (Value::uint(3), Value::array([path])),
    ])
    .unwrap();
    let value = Value::map([(
        Value::uint(0),
        Value::map([(Value::uint(0), config)]).unwrap(),
    )])
    .unwrap();
    let mut application = b"JANEXAPP".to_vec();
    cbor::write_sized(&mut application, &value).unwrap();
    let application_info = Value::map([
        (Value::uint(0), Value::text("main")),
        (Value::uint(1), Value::text("janex.java")),
    ])
    .unwrap();
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(0, BLOB_POOL, &pool, Some(info))
        .unwrap();
    writer
        .write_section(1, APPLICATION, &application, Some(application_info))
        .unwrap();
    writer.finish(Value::empty_map()).unwrap()
}

/// Records only the pages and content required for this root, matching native lazy resolution.
fn vector(output: &mut Vec<u8>, encoded: &[u8], root_index: u64, select: bool, limits: Limits) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap());
    output[..4].copy_from_slice(&(count + 1).to_be_bytes());
    for value in [
        limits.max_bytes,
        limits.max_elements,
        limits.max_depth as u64,
    ] {
        output.extend((value as u32).to_be_bytes());
    }
    bytes(output, encoded);
    let result = (|| {
        let reader = Reader::open_auto(Cursor::new(encoded), limits)?;
        let mut blobs = BlobStore::new(reader);
        let root_bytes = blobs.resolve(BlobRef {
            pool: 0,
            index: root_index,
        })?;
        let root = ResourceRoot::decode(&root_bytes, &mut blobs)?;
        let tree = root.merge(
            &Context {
                os: "windows".into(),
                arch: "x86-64".into(),
                invocation: Some("run".into()),
                runtime: None,
            },
            limits,
        )?;
        if select {
            tree.read_file("file", &mut blobs).map(Some)
        } else {
            Ok(None)
        }
    })();
    output.push(u8::from(result.is_ok()));
    bytes(
        output,
        result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default()
            .as_bytes(),
    );
    if let Ok(content) = result {
        output.extend(u32::from(content.is_some()).to_be_bytes());
        if let Some(content) = content {
            bytes(output, b"file");
            bytes(output, &content);
            output.extend([0; 3]);
            output.extend((-1i32).to_be_bytes());
        }
    }
}

#[test]
fn java_blob_pages_match_native_lazy_validation_and_logical_index_limits() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/bootstrap.jar");
    let compiled =
        Command::new("javac")
            .args(["--release", "8", "-cp"])
            .arg(&bootstrap)
            .arg("-d")
            .arg(temp.path())
            .arg(project.join(
                "janex-bootstrap/src/testFixtures/java/org/janex/bootstrap/ResourcesTest.java",
            ))
            .output()
            .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut vectors = vec![0; 4];
    let limits = Limits::default();
    let mut extras = vec![
        (255, vec![]),
        (255, vec![0xff, 0xff]),
        (1, vec![]),
        (1, integers(&[0])),
        (1, integers(&[1, 0, 0, 0])),
        (1, integers(&[1, 0, 0, 1])),
        (1, integers(&[1, 0, 2, 1])),
        (1, integers(&[1, 2, 0, 1])),
        (1, integers(&[1, u64::MAX, 0, 1])),
        (1, integers(&[1, 0, u64::MAX, 1])),
        (1, integers(&[1, 0, 0, u64::MAX])),
        (1, integers(&[1, 0, 0, 1, 0])),
        (0, stored(0, 0, None)),
        (0, stored(0, 1, None)),
        (0, stored(u64::MAX, 0, None)),
    ];
    for length in [0, 1, 4096, 4097, i32::MAX as u64 + 1, u64::MAX] {
        extras.push((0, stored(0, 0, Some(length))));
    }
    for extra in extras {
        for select in [false, true] {
            let encoded = package(3, 8, &extra, select, None, &[]);
            vector(
                &mut vectors,
                &encoded,
                1,
                select,
                Limits {
                    max_bytes: 4096,
                    ..limits
                },
            );
        }
    }
    for count in [258, 514, 4098, (1 << 20) + 2, (1u64 << 31) + 2] {
        let shift = if count >= 4098 { 12 } else { 8 };
        let encoded = package(count, shift, &(255, Vec::new()), false, None, &[]);
        vector(
            &mut vectors,
            &encoded,
            count - 1,
            false,
            Limits {
                max_elements: if count > i32::MAX as u64 {
                    1_000_000
                } else {
                    512
                },
                ..limits
            },
        );
    }
    let physical = vec![0; 8192];
    let encoded = package(
        3,
        8,
        &(0, stored(6, physical.len() as u64, None)),
        false,
        None,
        &physical,
    );
    vector(
        &mut vectors,
        &encoded,
        1,
        false,
        Limits {
            max_bytes: 4096,
            ..limits
        },
    );
    for checksum in [
        Value::null(),
        Value::text("bad"),
        Value::bytes(&[]),
        Value::bytes(&[0xff; 9]),
        Value::bytes(
            &Checksum::compute(Algorithm::Sha256, b"wrong".as_slice())
                .unwrap()
                .encode(),
        ),
    ] {
        let encoded = package(258, 8, &(255, Vec::new()), false, Some(checksum), &[]);
        vector(&mut vectors, &encoded, 257, false, limits);
    }
    // Unreferenced pool schemas are interpreted only when their pool is opened.
    let original = package(3, 8, &(255, Vec::new()), false, None, &[]);
    let mut reader = Reader::open_auto(Cursor::new(&original), limits).unwrap();
    let sections: Vec<_> = reader.sections().cloned().collect();
    let mut writer = Writer::new(Vec::new()).unwrap();
    for section in sections {
        writer
            .write_section(
                section.id(),
                section.kind(),
                &reader.read_section(section.id()).unwrap(),
                section.type_info().unwrap(),
            )
            .unwrap();
    }
    writer
        .write_section(2, BLOB_POOL, b"BLOBPOOL", Some(Value::empty_map()))
        .unwrap();
    let encoded = writer.finish(Value::empty_map()).unwrap();
    vector(&mut vectors, &encoded, 1, false, limits);
    let fixture = temp.path().join("blobs.bin");
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
            .arg("org.janex.bootstrap.ResourcesTest")
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

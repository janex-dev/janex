// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Content framing, transform size checks, and explicit pool selection.

use janex_format::{
    binary::{Decoder, Limits},
    blob::{BlobRef, BlobStore, PoolBuilder},
    cbor::Value,
    container::{BLOB_POOL, Reader, Writer},
    content::{Content, Source, Transform},
    data_pool::DataPool,
};
use std::io::Cursor;

/// Creates a container with a valid explicit pool, a malformed pool, and transformed bytes.
fn fixture() -> Fixture {
    let mut strings = DataPool::new();
    for text in ["sample", "Example", "java/lang", "Object"] {
        strings.intern(text);
    }
    let mut original = b"\xca\xfe\xba\xbe\0\0\0\x34\0\x05".to_vec();
    original.extend_from_slice(
        b"\x01\0\x0esample/Example\x07\0\x01\x01\0\x10java/lang/Object\x07\0\x03",
    );
    let body = [0, 0x21, 0, 2, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0];
    original.extend_from_slice(&body);
    let mut transformed =
        b"\xca\xfe\xca\x70\0\0\0\x34\0\x05\xfe\x01\x02\x07\0\x01\xfe\x03\x04\x07\0\x03".to_vec();
    transformed.extend_from_slice(&body);
    let mut pool = PoolBuilder::new();
    assert!(pool.is_empty());
    pool.push(&strings.encode().unwrap(), 3).unwrap();
    pool.push(b"\x02\0\0", 3).unwrap();
    assert_eq!(pool.len(), 2);
    pool.push(&transformed, 3).unwrap();
    let pool = pool.finish(8, 3).unwrap();
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(1, BLOB_POOL, &pool.bytes, Some(pool.type_info))
        .unwrap();
    let reader = Reader::open_auto(
        Cursor::new(writer.finish(Value::empty_map()).unwrap()),
        Limits::default(),
    )
    .unwrap();
    Fixture {
        blobs: BlobStore::new(reader),
        strings,
        original,
        transformed,
    }
}

/// Holds independent source bytes and the pools used by content tests.
struct Fixture {
    /// Container holding valid and invalid referenced blobs.
    blobs: BlobStore<Cursor<Vec<u8>>>,
    /// Default pool used by the fixture's transformed class.
    strings: DataPool,
    /// Expected ordinary class-file bytes.
    original: Vec<u8>,
    /// Independently encoded external-string class-file bytes.
    transformed: Vec<u8>,
}

#[test]
fn transform_uses_default_pool_or_validated_override_without_fallback() {
    let Fixture {
        mut blobs,
        strings,
        original,
        transformed,
    } = fixture();
    let mut content = Content {
        source: Source::Inline(transformed),
        transforms: vec![Transform {
            input_size: original.len() as u64,
            method: 1,
            properties: Value::empty_map(),
        }],
    };
    assert_eq!(
        content.resolve_file(&mut blobs, &strings).unwrap(),
        original
    );
    content.source = Source::Blob(BlobRef { pool: 1, index: 2 });
    content.transforms[0].properties = Value::map([
        (Value::uint(0), BlobRef { pool: 1, index: 0 }.to_value()),
        (Value::uint(100), Value::text("extension")),
    ])
    .unwrap();
    assert_eq!(
        content.resolve_file(&mut blobs, &DataPool::new()).unwrap(),
        original
    );
    let mut encoded = Vec::new();
    content.write(&mut encoded).unwrap();
    let mut decoder = Decoder::new(&encoded, Limits::default()).unwrap();
    let decoded = Content::read(&mut decoder).unwrap();
    decoder.finish().unwrap();
    assert_eq!(
        decoded.transforms[0].properties,
        content.transforms[0].properties
    );
    assert_eq!(
        decoded.resolve_file(&mut blobs, &DataPool::new()).unwrap(),
        original
    );
    assert!(decoded.resolve_entries(&mut blobs).is_err());
    content.transforms[0].input_size += 1;
    assert!(content.resolve_file(&mut blobs, &strings).is_err());
    content.transforms[0].input_size -= 1;
    for index in [1, 99] {
        content.transforms[0].properties =
            Value::map([(Value::uint(0), BlobRef { pool: 1, index }.to_value())]).unwrap();
        assert!(
            content.resolve_file(&mut blobs, &strings).is_err(),
            "invalid explicit pool {index}"
        );
    }
}

#[test]
fn content_wire_vectors_reject_unknown_variants_and_malformed_properties() {
    let Fixture {
        mut blobs, strings, ..
    } = fixture();
    let mut decoder = Decoder::new(&[0, 0, 0], Limits::default()).unwrap();
    let empty = Content::read(&mut decoder).unwrap();
    decoder.finish().unwrap();
    assert!(empty.resolve_entries(&mut blobs).unwrap().is_empty());
    assert!(empty.resolve_file(&mut blobs, &strings).unwrap().is_empty());
    for wire in [
        &b"\x02"[..],
        &b"\x00\x00\x01\x00\x02\x00"[..],
        &b"\x00\x00\x01\x00\x01\x01\xa0"[..],
        &b"\x00\x00\x01\x00\x01\x01\x80"[..],
    ] {
        assert!(Content::read(&mut Decoder::new(wire, Limits::default()).unwrap()).is_err());
    }
}

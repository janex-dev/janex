// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Blob-page, extent, and compression tests, including manually encoded table entries.

use janex_format::{
    ErrorKind,
    binary::{Limits, write_sized, write_vuint},
    blob::{BlobRef, BlobStore, Encoding, Extent, Filter, PoolBuilder, decode_zstd},
    cbor::Value,
    container::{self, Reader, Writer},
};
use std::{
    cell::RefCell,
    io::{Cursor, Read, Seek, SeekFrom},
    rc::Rc,
};

/// Wraps one pool section in a container with independent section checksums.
fn container_bytes(bytes: &[u8], type_info: Value) -> Vec<u8> {
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(7, container::BLOB_POOL, bytes, Some(type_info))
        .unwrap();
    writer.finish(Value::empty_map()).unwrap()
}

/// Wraps raw stored bytes and a manually constructed uncompressed table page.
fn manual_pool(data: &[u8], entries: &[Vec<u8>]) -> Vec<u8> {
    let page: Vec<_> = entries.iter().flatten().copied().collect();
    let type_info = Value::map([
        (Value::uint(0), Value::uint(entries.len() as u64)),
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
    bytes.extend_from_slice(data);
    bytes.extend(page);
    container_bytes(&bytes, type_info)
}

/// Builds a Stored entry independently of the pool builder.
fn stored_entry(offset: u64, encoding: Encoding) -> Vec<u8> {
    let mut payload = Vec::new();
    write_vuint(&mut payload, offset).unwrap();
    encoding.write(&mut payload).unwrap();
    let mut entry = vec![0];
    write_sized(&mut entry, &payload).unwrap();
    entry
}

/// Opens an owned memory container for blob decoding.
fn store(bytes: Vec<u8>) -> BlobStore<Cursor<Vec<u8>>> {
    BlobStore::new(Reader::open(Cursor::new(bytes), 0, Limits::default()).unwrap())
}

#[test]
fn stored_and_extent_bytes_round_trip_across_page_boundaries() {
    let mut builder = PoolBuilder::new();
    for index in 0..257 {
        assert_eq!(
            builder.push(format!("item {index}").as_bytes(), 3).unwrap(),
            index
        );
    }
    let large = vec![b'x'; 32768];
    let large_index = builder.push(&large, 3).unwrap();
    let assembled = builder
        .push_extents(vec![
            Extent {
                stored_blob_index: 0,
                decoded_offset: 0,
                decoded_length: 4,
            },
            Extent {
                stored_blob_index: 256,
                decoded_offset: 5,
                decoded_length: 3,
            },
        ])
        .unwrap();
    let empty = builder.push(b"", 3).unwrap();
    let built = builder.finish(8, 3).unwrap();
    let mut store = store(container_bytes(&built.bytes, built.type_info));
    assert_eq!(
        store
            .resolve(BlobRef {
                pool: 7,
                index: 256
            })
            .unwrap(),
        b"item 256"
    );
    assert_eq!(
        store
            .resolve(BlobRef {
                pool: 7,
                index: large_index
            })
            .unwrap(),
        large
    );
    assert_eq!(
        store
            .resolve(BlobRef {
                pool: 7,
                index: assembled
            })
            .unwrap(),
        b"item256"
    );
    assert!(
        store
            .resolve(BlobRef {
                pool: 7,
                index: empty
            })
            .unwrap()
            .is_empty()
    );
    store.validate_pool(7).unwrap();
    assert!(
        store
            .resolve(BlobRef {
                pool: 7,
                index: empty + 1
            })
            .is_err()
    );
}

#[test]
fn malformed_entries_and_overlapping_ranges_are_rejected() {
    let plain = |stored_size| Encoding {
        stored_size,
        filters: Vec::new(),
    };
    for entries in [
        vec![stored_entry(0, plain(3)), stored_entry(1, plain(3))],
        vec![stored_entry(4, plain(1))],
        vec![vec![0, 3, 0, 1, 0, 0]],
        vec![vec![1, 1, 0]],
        vec![vec![1, 4, 1, 0, 0, 0]],
        vec![vec![1, 4, 1, 0, 0, 1]],
    ] {
        assert!(
            store(manual_pool(b"data", &entries))
                .validate_pool(7)
                .is_err()
        );
    }
    let mut unknown = store(manual_pool(b"", &[vec![99, 3, 1, 2, 3]]));
    unknown.validate_pool(7).unwrap();
    assert_eq!(
        unknown
            .resolve(BlobRef { pool: 7, index: 0 })
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn zstd_requires_complete_frames_exact_sizes_and_at_least_one_data_frame() {
    let first = zstd::stream::encode_all(&b"hello "[..], 3).unwrap();
    let second = zstd::stream::encode_all(&b"world"[..], 3).unwrap();
    let skip = [0x50, 0x2a, 0x4d, 0x18, 2, 0, 0, 0, b'a', b'b'];
    let bytes = [first.as_slice(), &skip, second.as_slice()].concat();
    assert_eq!(
        decode_zstd(&bytes, &[], 11, Limits::default()).unwrap(),
        b"hello world"
    );
    assert!(decode_zstd(&bytes, &[], 10, Limits::default()).is_err());
    assert!(decode_zstd(&bytes, &[], 12, Limits::default()).is_err());
    assert!(decode_zstd(&skip, &[], 0, Limits::default()).is_err());
    assert!(
        decode_zstd(
            &[bytes.as_slice(), &[0]].concat(),
            &[],
            11,
            Limits::default()
        )
        .is_err()
    );
    for length in 0..first.len() {
        assert!(decode_zstd(&first[..length], &[], 6, Limits::default()).is_err());
    }
    assert!(
        decode_zstd(
            &bytes,
            &[],
            11,
            Limits {
                max_bytes: 10,
                ..Limits::default()
            }
        )
        .is_err()
    );
}

#[test]
fn external_raw_dictionary_and_dictionary_recursion_restriction() {
    let dictionary = b"The quick brown fox jumps over the lazy dog. A repeated dictionary phrase.";
    let data = b"The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog.";
    let encoded = zstd::bulk::Compressor::with_dictionary(3, dictionary)
        .unwrap()
        .compress(data)
        .unwrap();
    let properties =
        Value::map([(Value::uint(0), BlobRef { pool: 7, index: 0 }.to_value())]).unwrap();
    let encoding = Encoding {
        stored_size: encoded.len() as u64,
        filters: vec![Filter {
            input_size: data.len() as u64,
            method: 1,
            properties,
        }],
    };
    let entries = vec![
        stored_entry(
            0,
            Encoding {
                stored_size: dictionary.len() as u64,
                filters: Vec::new(),
            },
        ),
        stored_entry(dictionary.len() as u64, encoding.clone()),
    ];
    let stored = [dictionary.as_slice(), encoded.as_slice()].concat();
    let mut blobs = store(manual_pool(&stored, &entries));
    assert_eq!(blobs.resolve(BlobRef { pool: 7, index: 1 }).unwrap(), data);
    let self_reference = Encoding {
        filters: vec![Filter {
            properties: Value::map([(Value::uint(0), BlobRef { pool: 7, index: 1 }.to_value())])
                .unwrap(),
            ..encoding.filters[0].clone()
        }],
        ..encoding
    };
    let entries = vec![
        entries[0].clone(),
        stored_entry(dictionary.len() as u64, self_reference),
    ];
    assert!(
        store(manual_pool(&stored, &entries))
            .resolve(BlobRef { pool: 7, index: 1 })
            .is_err()
    );
}

#[test]
fn page_checksum_covers_decoded_bytes_and_empty_pool_is_valid() {
    let built = PoolBuilder::new().finish(8, 3).unwrap();
    let mut empty = store(container_bytes(&built.bytes, built.type_info));
    empty.validate_pool(7).unwrap();
    assert!(empty.resolve(BlobRef { pool: 7, index: 0 }).is_err());
    let mut builder = PoolBuilder::new();
    builder.push(b"data", 3).unwrap();
    let mut built = builder.finish(8, 3).unwrap();
    let descriptors = built.type_info.required(2).unwrap().as_array().unwrap();
    let offset = descriptors[0].as_array().unwrap()[0].as_u64().unwrap() as usize;
    built.bytes[8 + offset] ^= 1;
    let mut corrupt = store(container_bytes(&built.bytes, built.type_info));
    assert_eq!(
        corrupt
            .resolve(BlobRef { pool: 7, index: 0 })
            .unwrap_err()
            .kind(),
        ErrorKind::Verification
    );
}

/// A seekable stream recording which physical ranges are read.
struct TrackingReader {
    /// The underlying immutable bytes.
    cursor: Cursor<Vec<u8>>,
    /// Shared read log, cleared after container metadata is opened.
    reads: Rc<RefCell<Vec<std::ops::Range<u64>>>>,
}

impl Read for TrackingReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let start = self.cursor.position();
        let count = self.cursor.read(bytes)?;
        self.reads.borrow_mut().push(start..start + count as u64);
        Ok(count)
    }
}

impl Seek for TrackingReader {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(position)
    }
}

#[test]
fn one_blob_lookup_does_not_read_an_unrelated_page() {
    let mut builder = PoolBuilder::new();
    for _ in 0..257 {
        builder.push(b"value", 1).unwrap();
    }
    let built = builder.finish(8, 1).unwrap();
    let descriptors = built.type_info.required(2).unwrap().as_array().unwrap();
    let first = descriptors[0].as_array().unwrap();
    let start = 16 + first[0].as_u64().unwrap();
    let end = start
        + Encoding::from_value(&first[1], Limits::default())
            .unwrap()
            .stored_size;
    let reads = Rc::new(RefCell::new(Vec::new()));
    let tracking = TrackingReader {
        cursor: Cursor::new(container_bytes(&built.bytes, built.type_info)),
        reads: reads.clone(),
    };
    let mut store = BlobStore::new(Reader::open(tracking, 0, Limits::default()).unwrap());
    reads.borrow_mut().clear();
    assert_eq!(
        store
            .resolve(BlobRef {
                pool: 7,
                index: 256
            })
            .unwrap(),
        b"value"
    );
    assert!(
        reads
            .borrow()
            .iter()
            .all(|range| range.end <= start || range.start >= end)
    );
}

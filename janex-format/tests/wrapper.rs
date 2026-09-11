// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Independent ZIP wire fixtures for JAR-tail discovery and ambiguous boundaries.

use janex_format::{
    binary::Limits,
    cbor::Value,
    container::{Reader, Writer},
};
use std::io::Cursor;

/// Produces an unsigned Janex fixture without constraints on external regions.
fn janex() -> Vec<u8> {
    Writer::new(Vec::new())
        .unwrap()
        .finish_with(Value::empty_map(), 0, |_| Ok(Vec::new()))
        .unwrap()
}

/// Writes an ordinary ZIP end record with caller-selected directory fields and comment.
fn eocd(count: u16, directory_size: u32, directory_offset: u32, comment: &[u8]) -> Vec<u8> {
    let mut bytes = b"PK\x05\x06".to_vec();
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&directory_size.to_le_bytes());
    bytes.extend_from_slice(&directory_offset.to_le_bytes());
    bytes.extend_from_slice(&(comment.len() as u16).to_le_bytes());
    bytes.extend_from_slice(comment);
    bytes
}

/// Creates a stored ZIP entry and central-directory record directly from their wire layouts.
fn zip_file(comment: &[u8]) -> Vec<u8> {
    let mut local = vec![0; 30];
    local[..4].copy_from_slice(b"PK\x03\x04");
    local[4..6].copy_from_slice(&20u16.to_le_bytes());
    local[14..18].copy_from_slice(&0x3610a686u32.to_le_bytes());
    local[18..22].copy_from_slice(&5u32.to_le_bytes());
    local[22..26].copy_from_slice(&5u32.to_le_bytes());
    local[26..28].copy_from_slice(&5u16.to_le_bytes());
    local.extend_from_slice(b"a.txthello");
    let mut central = vec![0; 46];
    central[..4].copy_from_slice(b"PK\x01\x02");
    central[4..6].copy_from_slice(&20u16.to_le_bytes());
    central[6..8].copy_from_slice(&20u16.to_le_bytes());
    central[16..20].copy_from_slice(&0x3610a686u32.to_le_bytes());
    central[20..24].copy_from_slice(&5u32.to_le_bytes());
    central[24..28].copy_from_slice(&5u32.to_le_bytes());
    central[28..30].copy_from_slice(&5u16.to_le_bytes());
    central.extend_from_slice(b"a.txt");
    let footer = eocd(1, central.len() as u32, local.len() as u32, comment);
    [local, central, footer].concat()
}

/// Encodes an empty ZIP64 archive with an optional extensible-data sector.
fn zip64(extension: &[u8]) -> Vec<u8> {
    let mut bytes = b"PK\x06\x06".to_vec();
    bytes.extend_from_slice(&(44 + extension.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 40]);
    bytes.extend_from_slice(extension);
    bytes.extend_from_slice(b"PK\x06\x07");
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend(eocd(u16::MAX, u32::MAX, u32::MAX, b"zip64 comment"));
    bytes
}

#[test]
fn locates_standalone_and_ordinary_jar_tails_with_comments() {
    let janex = janex();
    let reader = Reader::open_auto(Cursor::new(&janex), Limits::default()).unwrap();
    assert_eq!(reader.range(), 0..janex.len() as u64);
    for tail in [
        eocd(0, 0, 0, b""),
        zip_file(b"comment with PK\x05\x06 inside"),
        zip_file(&vec![b'c'; 65535]),
    ] {
        let bytes = [b"external header".as_slice(), &janex, &tail].concat();
        let reader = Reader::open_auto(Cursor::new(bytes), Limits::default()).unwrap();
        assert_eq!(reader.range(), 15..15 + janex.len() as u64);
    }
}

#[test]
fn variable_zip64_record_lengths_and_locator_offsets_are_checked() {
    let janex = janex();
    for extension in [&[][..], &[0x34, 0x12, 0, 0, 0, 0][..], &vec![0; 70000][..]] {
        let tail = zip64(extension);
        let bytes = [&janex[..], &tail].concat();
        let reader = Reader::open_auto(Cursor::new(bytes), Limits::default()).unwrap();
        assert_eq!(reader.range(), 0..janex.len() as u64);
        let mut invalid = tail;
        invalid[56 + extension.len() + 8] = 1;
        assert!(
            Reader::open_auto(
                Cursor::new([&janex[..], &invalid].concat()),
                Limits::default()
            )
            .is_err()
        );
    }
}

#[test]
fn rejects_invalid_zip_references_and_nonterminal_comments() {
    let janex = janex();
    let original = zip_file(b"comment");
    for offset in [0, 30, 40, 82] {
        let mut tail = original.clone();
        tail[offset] ^= 1;
        assert!(
            Reader::open_auto(Cursor::new([&janex[..], &tail].concat()), Limits::default())
                .is_err(),
            "accepted corruption at {offset}"
        );
    }
    assert!(
        Reader::open_auto(
            Cursor::new([&janex[..], &original, &[0]].concat()),
            Limits::default()
        )
        .is_err()
    );
}

#[test]
fn nested_valid_end_records_do_not_silently_select_one_container() {
    let janex = janex();
    let comment = [&janex[..], &eocd(0, 0, 0, b"")].concat();
    let tail = eocd(0, 0, 0, &comment);
    let bytes = [&janex[..], &tail].concat();
    let error = match Reader::open_auto(Cursor::new(bytes), Limits::default()) {
        Ok(_) => panic!("accepted ambiguous boundaries"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("ambiguous"));
}

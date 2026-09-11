// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Container framing, checksum coverage, and sequential-output tests.

use janex_format::{
    ErrorKind,
    binary::Limits,
    cbor::Value,
    container::{self, Reader, Verification, Writer},
};
use std::io::{Cursor, Write};

/// Builds the smallest standalone container from fixed wire bytes, without the writer.
fn minimal_file() -> Vec<u8> {
    let mut bytes =
        b"JANEX\0\0\0METADATA\0\0\0\0\x01\0\0\0\x03\xa1\x00\x80\x00\x00JANEXEND".to_vec();
    bytes.extend_from_slice(&46u64.to_le_bytes());
    bytes.extend_from_slice(&54u64.to_le_bytes());
    bytes
}

/// Returns explicit empty-region constraints for a standalone file.
fn standalone_metadata() -> Value {
    Value::map([1, 2].map(|key| {
        (
            Value::uint(key),
            Value::map([(Value::uint(0), Value::uint(0))]).unwrap(),
        )
    }))
    .unwrap()
}

#[test]
fn reads_independent_minimal_container_and_preserves_signed_input() {
    let bytes = minimal_file();
    let mut reader = Reader::open(Cursor::new(&bytes), 0, Limits::default()).unwrap();
    assert_eq!(reader.range(), 0..54);
    assert_eq!(reader.sections().len(), 0);
    assert!(matches!(reader.verification(), Verification::None));
    assert_eq!(reader.verification_input(), &bytes[8..29]);
    assert!(!reader.verify_checksums().unwrap().complete_secure_coverage);
}

#[test]
fn every_truncation_of_the_minimal_file_is_rejected() {
    let bytes = minimal_file();
    for length in 0..bytes.len() {
        assert!(
            Reader::open(Cursor::new(&bytes[..length]), 0, Limits::default()).is_err(),
            "accepted prefix of {length} bytes"
        );
    }
    for (position, byte) in [
        (0, b'X'),
        (8, b'X'),
        (16, 1),
        (20, 2),
        (30, b'X'),
        (38, 255),
        (46, 255),
    ] {
        let mut corrupt = bytes.clone();
        corrupt[position] = byte;
        assert!(
            Reader::open(Cursor::new(corrupt), 0, Limits::default()).is_err(),
            "accepted corruption at {position}"
        );
    }
}

#[test]
fn writer_records_sections_extensions_and_secure_coverage() {
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(800, container::PADDING, b"padding bytes", None)
        .unwrap();
    writer
        .write_section(4, 123, b"unknown section", Some(Value::empty_map()))
        .unwrap();
    let mut fields = standalone_metadata().as_map().unwrap();
    fields.push((
        Value::text("org.example.extension"),
        Value::array([Value::uint(42), Value::boolean(true)]),
    ));
    let metadata = Value::map(fields).unwrap();
    let bytes = writer.finish(metadata.clone()).unwrap();
    let mut reader = Reader::open(Cursor::new(&bytes), 0, Limits::default()).unwrap();
    assert_eq!(
        reader
            .sections()
            .map(|section| section.id())
            .collect::<Vec<_>>(),
        [800, 4]
    );
    assert_eq!(reader.read_section(4).unwrap(), b"unknown section");
    assert_eq!(reader.read_section_range(800, 8, 5).unwrap(), b"bytes");
    assert!(reader.read_section_range(4, u64::MAX, 1).is_err());
    assert!(reader.section(0).is_err());
    assert!(reader.metadata().as_map().unwrap().contains(&(
        Value::text("org.example.extension"),
        metadata.as_map().unwrap().last().unwrap().1.clone()
    )));
    let report = reader.verify_checksums().unwrap();
    assert_eq!(report.checksums_verified, 2);
    assert!(report.complete_secure_coverage);
    assert!(matches!(reader.verification(), Verification::Checksum(_)));
    let mut corrupt = bytes.clone();
    corrupt[8] ^= 1;
    let mut reader = Reader::open(Cursor::new(corrupt), 0, Limits::default()).unwrap();
    assert_eq!(
        reader.verify_checksums().unwrap_err().kind(),
        ErrorKind::Verification
    );
}

#[test]
fn external_regions_are_located_and_constrained_separately() {
    let mut writer = Writer::new(b"header".to_vec()).unwrap();
    writer
        .write_section(1, container::PADDING, b"data", None)
        .unwrap();
    let metadata = Value::map([
        (
            Value::uint(1),
            Value::map([(Value::uint(0), Value::uint(6))]).unwrap(),
        ),
        (
            Value::uint(2),
            Value::map([(Value::uint(0), Value::uint(4))]).unwrap(),
        ),
    ])
    .unwrap();
    let mut bytes = writer.finish(metadata).unwrap();
    let janex_end = bytes.len() as u64;
    bytes.extend_from_slice(b"tail");
    let mut reader = Reader::open(Cursor::new(&bytes), 4, Limits::default()).unwrap();
    assert_eq!(reader.range(), 6..janex_end);
    assert_eq!(reader.read_section(1).unwrap(), b"data");
    assert!(!reader.verify_checksums().unwrap().complete_secure_coverage);
    assert!(Reader::open(Cursor::new(&bytes), 0, Limits::default()).is_err());
    assert!(Reader::open(Cursor::new(&bytes), u64::MAX, Limits::default()).is_err());
}

#[test]
fn writer_checks_identity_metadata_and_payloads() {
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(9, container::PADDING, b"", None)
        .unwrap();
    assert!(
        writer
            .write_section(9, container::PADDING, b"", None)
            .is_err()
    );
    assert!(
        writer
            .write_section(10, container::APPLICATION, b"bad", None)
            .is_err()
    );
    assert!(
        Writer::new(Vec::new())
            .unwrap()
            .finish(Value::map([(Value::uint(0), Value::array([]))]).unwrap())
            .is_err()
    );
    assert!(
        Writer::new(Vec::new())
            .unwrap()
            .finish_with(Value::empty_map(), 0, |_| Ok(vec![0]))
            .is_err()
    );
    assert!(
        Writer::new(Vec::new())
            .unwrap()
            .finish_with(Value::empty_map(), 2, |_| Ok(vec![]))
            .is_err()
    );
    assert!(
        Writer::new(Vec::new())
            .unwrap()
            .finish_with(Value::empty_map(), 99, |_| Ok(vec![1]))
            .is_err()
    );
    let bytes = Writer::new(Vec::new())
        .unwrap()
        .finish_with(Value::empty_map(), 0, |_| Ok(vec![]))
        .unwrap();
    assert_eq!(bytes, minimal_file());
}

#[test]
fn parsing_checksums_does_not_imply_verification() {
    let bytes = Writer::new(Vec::new())
        .unwrap()
        .finish(Value::empty_map())
        .unwrap();
    let mut corrupt = bytes;
    let payload_end = corrupt.len() - 24;
    corrupt[payload_end - 1] ^= 1;
    let mut reader = Reader::open(Cursor::new(corrupt), 0, Limits::default()).unwrap();
    assert_eq!(
        reader.verify_checksums().unwrap_err().kind(),
        ErrorKind::Verification
    );
}

/// An output that retains a prefix before returning a write error.
struct FailingWriter(Vec<u8>);

impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len() >= 10 {
            return Err(std::io::Error::other("injected failure"));
        }
        let count = bytes.len().min(10 - self.0.len());
        self.0.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn stream_failures_retain_the_io_cause() {
    let mut output = FailingWriter(Vec::new());
    let mut writer = Writer::new(&mut output).unwrap();
    let error = writer
        .write_section(0, container::PADDING, b"several bytes", None)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert!(std::error::Error::source(&error).is_some());
    assert_eq!(output.0.len(), 10);
}

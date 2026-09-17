// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Module inventory reads metadata without inflating unrelated archive payloads.

use janex_java::{Limits, jar};
use std::io::{Cursor, Write};
use zip::{ZipWriter, write::SimpleFileOptions};

/// Creates stored metadata and one unused payload with a recognizable byte sequence.
fn archive(link: bool) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in [
        ("META-INF/MANIFEST.MF", "Multi-Release: true\r\n\r\n"),
        ("module-info.class", "base"),
        ("META-INF/versions/17/module-info.class", "versioned"),
        ("unused.bin", "unused-payload"),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes.as_bytes()).unwrap();
    }
    if link {
        writer.add_symlink("alias", "unused.bin", options).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn metadata_reader_does_not_decode_unused_payloads() {
    let mut bytes = archive(false);
    let offset = bytes
        .windows(14)
        .position(|v| v == b"unused-payload")
        .unwrap();
    bytes[offset] ^= 1;
    let entries = jar::read_module_metadata(&bytes, Limits::default(), 4096)
        .unwrap()
        .unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[2].content, b"versioned");
    assert!(jar::read(&bytes, Limits::default(), 4096).is_err());
}

#[test]
fn symbolic_links_require_complete_resource_interpretation() {
    assert!(
        jar::read_module_metadata(&archive(true), Limits::default(), 4096)
            .unwrap()
            .is_none()
    );
}

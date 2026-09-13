// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Independent Java container framing, exact metadata access, and integrity-report parity.

use janex_format::{
    binary::{self, Limits},
    cbor::{self, Value},
    checksum::{Algorithm, Checksum},
    container::{APPLICATION, Reader, Verification},
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
};

/// Writes a counted byte sequence for the Java harness.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Records structural acceptance, exact metadata bytes, and the independent integrity result.
fn vector(output: &mut Vec<u8>, encoded: &[u8], tail: i64) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap());
    output[..4].copy_from_slice(&(count + 1).to_be_bytes());
    bytes(output, encoded);
    output.extend(tail.to_be_bytes());
    let parsed = if tail == -1 {
        Reader::open_auto(Cursor::new(encoded), Limits::default())
    } else {
        Reader::open(Cursor::new(encoded), tail as u64, Limits::default())
    };
    let Ok(mut reader) = parsed else {
        output.push(0);
        return;
    };
    output.push(1);
    output.extend(reader.range().start.to_be_bytes());
    output.extend(reader.range().end.to_be_bytes());
    bytes(output, reader.metadata().as_bytes());
    bytes(output, reader.verification_input());
    let (kind, payload) = match reader.verification() {
        Verification::None => (0, Vec::new()),
        Verification::Checksum(checksum) => (1, checksum.encode()),
        Verification::OpenPgp(bytes) => (2, bytes.clone()),
        Verification::Cms(bytes) => (3, bytes.clone()),
    };
    output.push(kind);
    bytes(output, &payload);
    output.extend((reader.sections().len() as u32).to_be_bytes());
    let mut offset = reader.range().start + 8;
    for section in reader.sections().cloned().collect::<Vec<_>>() {
        output.extend(section.id().to_be_bytes());
        output.extend(section.kind().to_be_bytes());
        output.extend(section.length().to_be_bytes());
        output.extend(offset.to_be_bytes());
        offset += section.length();
        bytes(output, section.value().as_bytes());
        let info = section.type_info().unwrap();
        output.push(u8::from(info.is_some()));
        if let Some(info) = info {
            bytes(output, info.as_bytes());
        }
        output.push(u8::from(section.checksum().is_some()));
        if let Some(checksum) = section.checksum() {
            bytes(output, &checksum.encode());
        }
        bytes(output, &reader.read_section(section.id()).unwrap());
    }
    let report = reader.verify_checksums();
    output.push(u8::from(report.is_ok()));
    if let Ok(report) = report {
        output.extend((report.checksums_verified as u32).to_be_bytes());
        output.push(u8::from(report.complete_secure_coverage));
    }
}

/// Computes an encoded digest for a fixture field.
fn checksum(algorithm: Algorithm, bytes: &[u8]) -> Value {
    Value::bytes(&Checksum::compute(algorithm, bytes).unwrap().encode())
}

/// Builds exact wire bytes without using the container writer, including nonminimal Sized prefixes.
fn sample(
    algorithm: Option<Algorithm>,
    verification: u8,
    region_mode: usize,
    header: &[u8],
    tail: &[u8],
    nonminimal: bool,
) -> Vec<u8> {
    let bodies = [
        vec![0x42; 65539],
        Vec::new(),
        // Known magic alone is valid framing, even though application-body parsing would fail.
        APPLICATION.to_le_bytes().to_vec(),
    ];
    let mut sections = Vec::new();
    for (index, body) in bodies.iter().enumerate() {
        let mut fields = vec![
            (
                Value::uint(0),
                Value::uint(if index == 2 { APPLICATION } else { u64::MAX }),
            ),
            (Value::uint(1), Value::uint(u64::MAX - index as u64)),
            (Value::uint(2), Value::uint(body.len() as u64)),
            (Value::uint(88), Value::bytes(b"unknown descriptor field")),
        ];
        if let Some(algorithm) = algorithm {
            fields.push((Value::uint(3), checksum(algorithm, body)));
        }
        if index != 0 {
            fields.push((Value::uint(4), Value::empty_map()));
        }
        sections.push(Value::map(fields).unwrap());
    }
    let mut fields = vec![
        (Value::uint(0), Value::array(sections)),
        (Value::uint(3), Value::text("org.example.fixture")),
        (
            Value::text("org.example.extension"),
            Value::from_bytes(&[0xd8, 42, 0xf9, 0x7e, 0], Limits::default()).unwrap(),
        ),
    ];
    for (key, region) in [(1, header), (2, tail)] {
        if region_mode == 0 || region_mode == 3 && key == 1 {
            continue;
        }
        let mut info = vec![(Value::uint(0), Value::uint(region.len() as u64))];
        if region_mode >= 2 {
            info.push((
                Value::uint(1),
                checksum(algorithm.unwrap_or(Algorithm::Sha256), region),
            ));
        }
        fields.push((Value::uint(key), Value::map(info).unwrap()));
    }
    let value = Value::map(fields).unwrap();
    let mut metadata = b"METADATA\0\0\0\0\x01\0\0\0".to_vec();
    if nonminimal {
        let mut size = Vec::new();
        binary::write_vuint(&mut size, value.as_bytes().len() as u64).unwrap();
        *size.last_mut().unwrap() |= 0x80;
        size.push(0);
        metadata.extend(size);
        metadata.extend(value.as_bytes());
    } else {
        cbor::write_sized(&mut metadata, &value).unwrap();
    }
    metadata.push(verification);
    let payload = match verification {
        0 => Vec::new(),
        1 => Checksum::compute(algorithm.unwrap_or(Algorithm::Sha256), metadata.as_slice())
            .unwrap()
            .encode(),
        // Container parsing retains signature bytes without claiming they are valid signatures.
        2 | 3 => vec![0x80, 0x01, 0x42],
        _ => unreachable!(),
    };
    binary::write_sized(&mut metadata, &payload).unwrap();
    let mut encoded = header.to_vec();
    encoded.extend(b"JANEX\0\0\0");
    for body in bodies {
        encoded.extend(body);
    }
    encoded.extend(&metadata);
    let length = encoded.len() - header.len() + 24;
    encoded.extend(b"JANEXEND");
    encoded.extend((metadata.len() as u64 + 24).to_le_bytes());
    encoded.extend((length as u64).to_le_bytes());
    encoded.extend(tail);
    encoded
}

#[test]
fn java_container_preserves_raw_metadata_and_separates_parsing_from_integrity() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/build/libs/janex-bootstrap.jar");
    let output = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&bootstrap)
        .arg("-d")
        .arg(temp.path())
        .arg(project.join("janex-reader/src/testFixtures/java/org/janex/reader/ContainerTest.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut vectors = vec![0; 4];
    for algorithm in [
        None,
        Some(Algorithm::Xxh3_64),
        Some(Algorithm::Xxh3_128),
        Some(Algorithm::Sha256),
        Some(Algorithm::Sha512),
        Some(Algorithm::Sm3),
    ] {
        for verification in 0..=3 {
            for region_mode in 0..4 {
                for (header, tail) in [
                    (b"".as_slice(), b"".as_slice()),
                    (b"header".as_slice(), b"opaque tail".as_slice()),
                ] {
                    let original = sample(algorithm, verification, region_mode, header, tail, true);
                    vector(&mut vectors, &original, tail.len() as i64);
                    if tail.is_empty() {
                        vector(&mut vectors, &original, -1);
                    }
                    let mut corrupt = original.clone();
                    corrupt[header.len() + 8] ^= 1;
                    vector(&mut vectors, &corrupt, tail.len() as i64);
                }
            }
        }
    }
    let original = sample(Some(Algorithm::Sha256), 1, 2, b"", b"", false);
    let metadata_start = Reader::open(Cursor::new(&original), 0, Limits::default())
        .unwrap()
        .range()
        .end as usize
        - u64::from_le_bytes(
            original[original.len() - 16..original.len() - 8]
                .try_into()
                .unwrap(),
        ) as usize;
    for position in (0..8).chain(metadata_start..original.len()) {
        let mut corrupt = original.clone();
        corrupt[position] ^= 1;
        vector(&mut vectors, &corrupt, 0);
    }
    for length in 0..64 {
        vector(&mut vectors, &original[..length], 0);
    }
    for tail in [-2, 1, i64::MAX] {
        vector(&mut vectors, &original, tail);
    }
    vector(&mut vectors, &original, 0);
    let fixture = temp.path().join("containers.bin");
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
            .arg("org.janex.reader.ContainerTest")
            .arg(&fixture)
            .arg(temp.path().join("snapshot.janex"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

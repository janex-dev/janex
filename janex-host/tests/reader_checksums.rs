// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Independent Rust digest vectors exercised by Java 8 and the current Java runtime.

use janex_format::{
    binary::{self, Decoder, Limits},
    blob::Encoding,
    cbor::{self, Value},
    checksum::{Algorithm, Checksum},
    container::{self, Reader},
};
use janex_host::{
    pack::{PackOptions, pack},
    run::{RunOptions, prepare},
};
use std::{fs, io::Cursor, path::Path, process::Command};

/// Replaces or removes one integer-keyed metadata field.
fn field(value: Value, key: u64, replacement: Option<Value>) -> Value {
    let mut fields = value.as_map().unwrap();
    fields.retain(|(candidate, _)| candidate.as_u64().ok() != Some(key));
    if let Some(value) = replacement {
        fields.push((Value::uint(key), value));
    }
    Value::map(fields).unwrap()
}

/// Writes independent container bytes with the selected digest at every recorded checksum site.
fn recode(source: &Path, algorithm: Option<Algorithm>, corrupt_page: bool) -> Vec<u8> {
    let mut reader =
        Reader::open_auto(Cursor::new(fs::read(source).unwrap()), Limits::default()).unwrap();
    let mut body = container::MAGIC.to_vec();
    let mut sections = Vec::new();
    for section in reader.sections().cloned().collect::<Vec<_>>() {
        let content = reader.read_section(section.id()).unwrap();
        let mut value = section.value().clone();
        if section.kind() == container::BLOB_POOL {
            let info = section.type_info().unwrap().unwrap();
            let mut pages = Vec::new();
            for page in info.required(2).unwrap().as_array().unwrap() {
                let mut fields = page.as_array().unwrap();
                let offset = fields[0].as_u64().unwrap() as usize + 8;
                let encoding = Encoding::read(
                    &mut Decoder::new(fields[1].as_byte_string().unwrap(), Limits::default())
                        .unwrap(),
                )
                .unwrap();
                let mut decoded = content[offset..offset + encoding.stored_size as usize].to_vec();
                for _ in encoding.filters.iter().rev() {
                    decoded = zstd::stream::decode_all(decoded.as_slice()).unwrap();
                }
                fields.truncate(2);
                if let Some(algorithm) = algorithm {
                    let mut checksum = Checksum::compute(algorithm, decoded.as_slice())
                        .unwrap()
                        .encode();
                    if corrupt_page {
                        checksum[1] ^= 1;
                    }
                    fields.push(Value::bytes(&checksum));
                }
                pages.push(Value::array(fields));
            }
            value = field(value, 4, Some(field(info, 2, Some(Value::array(pages)))));
        }
        value = field(
            value,
            3,
            algorithm.map(|algorithm| {
                Value::bytes(
                    &Checksum::compute(algorithm, content.as_slice())
                        .unwrap()
                        .encode(),
                )
            }),
        );
        sections.push(value);
        body.extend(content);
    }
    let tail = include_bytes!("../../janex-bootstrap/bootstrap.jar");
    let mut metadata = Value::map([
        (Value::uint(0), Value::array(sections)),
        (
            Value::text("test.attribute"),
            Value::text("retained extension"),
        ),
    ])
    .unwrap();
    if let Some(algorithm) = algorithm {
        metadata = field(
            metadata,
            1,
            Some(Value::map([(Value::uint(0), Value::uint(0))]).unwrap()),
        );
        metadata = field(
            metadata,
            2,
            Some(
                Value::map([
                    (Value::uint(0), Value::uint(tail.len() as u64)),
                    (
                        Value::uint(1),
                        Value::bytes(
                            &Checksum::compute(algorithm, tail.as_slice())
                                .unwrap()
                                .encode(),
                        ),
                    ),
                ])
                .unwrap(),
            ),
        );
    }
    let mut encoded = container::METADATA_MAGIC.to_vec();
    encoded.extend(0u32.to_le_bytes());
    encoded.extend(1u32.to_le_bytes());
    cbor::write_sized(&mut encoded, &metadata).unwrap();
    encoded.push(u8::from(algorithm.is_some()));
    let checksum = algorithm
        .map(|algorithm| {
            Checksum::compute(algorithm, encoded.as_slice())
                .unwrap()
                .encode()
        })
        .unwrap_or_default();
    binary::write_sized(&mut encoded, &checksum).unwrap();
    let metadata_length = encoded.len() as u64 + 24;
    let file_length = body.len() as u64 + metadata_length;
    body.extend(encoded);
    body.extend(container::END_MARK);
    body.extend(metadata_length.to_le_bytes());
    body.extend(file_length.to_le_bytes());
    body.extend(tail);
    body
}

#[test]
fn java_and_rust_accept_recorded_algorithms_and_reject_bad_page_checksums() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), "public class Main { public static void main(String[] args) { System.out.println(\"checksum-ok\"); } }").unwrap();
    let compiled = Command::new("javac")
        .current_dir(temp.path())
        .args(["--release", "8", "-d", "classes", "Main.java"])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let source = temp.path().join("source.janex");
    let mut options = PackOptions::new(temp.path().join("classes"), &source);
    options.main_class = Some("Main".into());
    options.with_launcher = true;
    pack(&options).unwrap();
    for algorithm in [
        None,
        Some(Algorithm::Xxh3_64),
        Some(Algorithm::Xxh3_128),
        Some(Algorithm::Sha256),
        Some(Algorithm::Sha512),
        Some(Algorithm::Sm3),
    ] {
        let target = temp.path().join(format!("{algorithm:?}.janex"));
        fs::write(&target, recode(&source, algorithm, false)).unwrap();
        Reader::open_auto(fs::File::open(&target).unwrap(), Limits::default())
            .unwrap()
            .verify_checksums()
            .unwrap();
        let mut run = RunOptions::new(&target);
        run.allow_unsigned = true;
        prepare(&run).unwrap();
        let result = Command::new("java")
            .arg("-jar")
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{algorithm:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "checksum-ok"
        );
        if algorithm.is_some() {
            fs::write(&target, recode(&source, algorithm, true)).unwrap();
            assert!(prepare(&run).is_err());
            let result = Command::new("java")
                .arg("-jar")
                .arg(&target)
                .output()
                .unwrap();
            assert!(!result.status.success());
            assert!(String::from_utf8_lossy(&result.stderr).contains("checksum mismatch"));
        }
    }
}

#[test]
fn java_checksums_match_rust_across_streaming_and_algorithm_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/bootstrap.jar");
    let harness =
        project.join("janex-reader/src/testFixtures/java/org/janex/format/ChecksumTest.java");
    let result = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&bootstrap)
        .arg("-d")
        .arg(temp.path())
        .arg(harness)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let lengths: Vec<usize> = (0..=260)
        .chain([
            511, 512, 513, 1023, 1024, 1025, 2047, 2048, 2049, 4095, 4096, 4097, 65535, 65536,
            65537, 1_000_003,
        ])
        .collect();
    let mut vectors = Vec::new();
    vectors.extend_from_slice(&((lengths.len() * 3) as u32).to_be_bytes());
    let mut random = 0x123456789abcdef0u64;
    for length in lengths {
        for pattern in 0..3 {
            let input: Vec<u8> = (0..length)
                .map(|index| {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    match pattern {
                        0 => 0,
                        1 => (index % 251) as u8,
                        _ => random as u8,
                    }
                })
                .collect();
            vectors.extend_from_slice(&(length as u32).to_be_bytes());
            vectors.extend(&input);
            for algorithm in [
                Algorithm::Xxh3_64,
                Algorithm::Xxh3_128,
                Algorithm::Sha256,
                Algorithm::Sha512,
                Algorithm::Sm3,
            ] {
                let encoded = Checksum::compute(algorithm, input.as_slice())
                    .unwrap()
                    .encode();
                vectors.push(encoded.len() as u8);
                vectors.extend(encoded);
            }
        }
    }
    let path = temp.path().join("checksums.bin");
    fs::write(&path, vectors).unwrap();
    let classpath = std::env::join_paths([temp.path(), bootstrap.as_path()]).unwrap();
    let mut runtimes = vec!["java".into()];
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        runtimes.push(Path::new(&home).join("bin").join(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
    }
    for java in runtimes {
        let result = Command::new(java)
            .arg("-cp")
            .arg(&classpath)
            .arg("org.janex.format.ChecksumTest")
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

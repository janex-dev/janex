// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Independent dictionary vectors and Java/Rust container acceptance tests.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use janex_format::{
    binary::{self, Limits},
    blob::{BlobRef, BlobStore, Encoding, Filter},
    cbor::Value,
    checksum::{Algorithm, Checksum},
    container::{BLOB_POOL, Reader, Writer},
};
use janex_host::{
    pack::{PackOptions, pack},
    run::{RunOptions, prepare},
};

/// Executes a JDK tool and preserves its diagnostics on failure.
fn tool(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Returns the default runtime and an explicitly configured Java 8 runtime.
fn runtimes() -> Vec<PathBuf> {
    let mut result = vec![PathBuf::from("java")];
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        result.push(Path::new(&home).join("bin").join(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
    }
    result
}

/// Produces deterministic training samples with varied repeated text and byte values.
fn samples() -> Vec<Vec<u8>> {
    (0..256)
        .map(|i| {
            let mut bytes = format!(
                "package org.glavo.janex.example; public final class Example{i} {{ public String name() {{ return \"resource-{i}\"; }} }}\n"
            ).repeat(8).into_bytes();
            bytes.extend((0..1024).map(|j| ((j * 17 + i * 7 + j / 11) % 251) as u8));
            bytes
        })
        .collect()
}

/// Encodes with libzstd, optionally omitting the dictionary ID from frame headers.
fn frame(bytes: &[u8], dictionary: &[u8], level: i32, id: bool) -> Vec<u8> {
    let mut encoder =
        zstd::stream::Encoder::with_dictionary(Vec::new(), level, dictionary).unwrap();
    encoder.include_dictid(id).unwrap();
    encoder.include_checksum(true).unwrap();
    encoder
        .set_pledged_src_size(Some(bytes.len() as u64))
        .unwrap();
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// Appends one byte string to the cross-language fixture stream.
fn bytes(output: &mut Vec<u8>, input: &[u8]) {
    output.extend_from_slice(&(input.len() as u32).to_be_bytes());
    output.extend(input);
}

/// Records native decoding as the oracle, bounded to two MiB of decoded data.
fn vector(output: &mut Vec<u8>, dictionary: &[u8], encoded: &[u8]) {
    let mut decoded = Vec::new();
    let valid = zstd::stream::Decoder::with_dictionary(encoded, dictionary)
        .and_then(|decoder| decoder.take(2_097_153).read_to_end(&mut decoded))
        .is_ok()
        && decoded.len() <= 2_097_152;
    output.push(u8::from(valid));
    bytes(output, dictionary);
    bytes(output, encoded);
    bytes(output, if valid { &decoded } else { &[0; 8192] });
    let count = u32::from_be_bytes(output[..4].try_into().unwrap()) + 1;
    output[..4].copy_from_slice(&count.to_be_bytes());
}

#[test]
fn java_dictionary_decoder_matches_native_raw_trained_and_malformed_vectors() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let jar = project.join("janex-bootstrap/build/libs/janex-bootstrap.jar");
    tool(
        Command::new("javac")
            .args(["--release", "8", "-cp"])
            .arg(&jar)
            .arg("-d")
            .arg(temp.path())
            .arg(project.join(
                "janex-reader/src/testFixtures/java/org/glavo/janex/reader/internal/codec/DictionaryTest.java",
            )),
    );
    let samples = samples();
    let trained = zstd::dict::from_samples(&samples, 4096).unwrap();
    let raw = samples.concat();
    let mut vectors = vec![0; 4];
    for dictionary in [&[][..], &raw[..], &trained[..]] {
        for size in [0, 1, 7, 8, 32, 128, 256, 1024, 8192, 131_072, 524_288] {
            let plain: Vec<_> = raw.iter().cycle().skip(73).take(size).copied().collect();
            for level in [1, 3, 9, 19] {
                for id in [false, true] {
                    let encoded = frame(&plain, dictionary, level, id);
                    vector(&mut vectors, dictionary, &encoded);
                }
            }
        }
        let mut concatenated = frame(&samples[3], dictionary, 3, true);
        concatenated.extend([0x50, 0x2a, 0x4d, 0x18, 1, 0, 0, 0, 42]);
        concatenated.extend(frame(&[], dictionary, 9, false));
        concatenated.extend(frame(&samples[5], dictionary, 19, false));
        vector(&mut vectors, dictionary, &concatenated);
    }
    let encoded = frame(&samples[17], &trained, 9, true);
    vector(&mut vectors, &[], &encoded);
    vector(&mut vectors, &raw, &encoded);
    for length in 0..trained.len().min(400) {
        vector(&mut vectors, &trained[..length], &encoded);
    }
    for i in 0..512 {
        let mut damaged = trained.clone();
        damaged[i % trained.len()] ^= 1 << (i % 8);
        vector(&mut vectors, &damaged, &encoded);
    }
    for i in 0..256 {
        let mut damaged = encoded.clone();
        damaged[i * 73 % encoded.len()] ^= 1 << (i % 8);
        vector(&mut vectors, &trained, &damaged);
    }
    for length in 0..8 {
        vector(
            &mut vectors,
            &raw[..length],
            &frame(b"short", &[], 3, false),
        );
    }
    let path = temp.path().join("dictionaries.bin");
    fs::write(&path, vectors).unwrap();
    let classpath = std::env::join_paths([temp.path(), jar.as_path()]).unwrap();
    for java in runtimes() {
        tool(
            Command::new(java)
                .arg("-cp")
                .arg(&classpath)
                .arg("org.glavo.janex.reader.internal.codec.DictionaryTest")
                .arg(&path),
        );
    }
}

/// Appends a Stored entry with an exact encoded range.
fn stored(data: &mut Vec<u8>, page: &mut Vec<u8>, encoded: &[u8], filters: Vec<Filter>) {
    let mut entry = Vec::new();
    binary::write_vuint(&mut entry, data.len() as u64).unwrap();
    Encoding {
        stored_size: encoded.len() as u64,
        filters,
    }
    .write(&mut entry)
    .unwrap();
    page.push(0);
    binary::write_sized(page, &entry).unwrap();
    data.extend(encoded);
}

/// Creates a Zstandard filter with an optional explicitly selected dictionary.
fn filter(length: usize, dictionary: Option<BlobRef>) -> Filter {
    Filter {
        input_size: length as u64,
        method: 1,
        properties: dictionary.map_or_else(Value::empty_map, |reference| {
            Value::map([(Value::uint(0), reference.to_value())]).unwrap()
        }),
    }
}

/// Writes a single-page pool with independent page compression and checksum.
fn pool(
    writer: &mut Writer<Vec<u8>>,
    id: u64,
    count: u64,
    mut data: Vec<u8>,
    page: Vec<u8>,
    forbidden_dictionary: Option<BlobRef>,
    dictionary: &[u8],
) {
    let encoded = frame(
        &page,
        if forbidden_dictionary.is_some() {
            dictionary
        } else {
            &[]
        },
        3,
        true,
    );
    let info = Value::map([
        (Value::uint(0), Value::uint(count)),
        (Value::uint(1), Value::uint(8)),
        (
            Value::uint(2),
            Value::array([Value::array([
                Value::uint(data.len() as u64),
                Encoding {
                    stored_size: encoded.len() as u64,
                    filters: vec![filter(page.len(), forbidden_dictionary)],
                }
                .to_value()
                .unwrap(),
                Value::bytes(
                    &Checksum::compute(Algorithm::Xxh3_128, page.as_slice())
                        .unwrap()
                        .encode(),
                ),
            ])]),
        ),
    ])
    .unwrap();
    data.extend(encoded);
    let mut section = b"BLOBPOOL".to_vec();
    section.extend(data);
    writer
        .write_section(id, BLOB_POOL, &section, Some(info))
        .unwrap();
}

/// Re-encodes every original blob with forward dictionary references in a separate pool.
/// Modes exercise valid Extents dictionaries and forbidden nested, self, and page references.
fn package(source: &Path, dictionary: &[u8], mode: u8) -> Vec<u8> {
    let reader = Reader::open_auto(fs::File::open(source).unwrap(), Limits::default()).unwrap();
    let sections: Vec<_> = reader.sections().cloned().collect();
    let dictionary_pool = sections.iter().map(|section| section.id()).max().unwrap() + 1;
    let reference = BlobRef {
        pool: dictionary_pool,
        index: if mode == 1 { 3 } else { 2 },
    };
    let mut metadata = reader.metadata().as_map().unwrap();
    metadata.retain(|(key, _)| key.as_u64().ok() != Some(0));
    let mut blobs = BlobStore::new(reader);
    let mut writer = Writer::new(Vec::new()).unwrap();
    for section in sections {
        let info = section.type_info().unwrap();
        if section.kind() == BLOB_POOL {
            let count = info.unwrap().required(0).unwrap().as_u64().unwrap();
            assert!(count <= 256);
            let mut data = Vec::new();
            let mut page = Vec::new();
            for index in 0..count {
                let current = BlobRef {
                    pool: section.id(),
                    index,
                };
                let plain = blobs.resolve(current).unwrap();
                let encoded = frame(&plain, dictionary, 9, true);
                // A second filter verifies that each stage resolves its own dictionary.
                let outer = frame(&encoded, dictionary, 3, false);
                let selected = if mode == 2 { current } else { reference };
                stored(
                    &mut data,
                    &mut page,
                    &outer,
                    vec![
                        filter(plain.len(), Some(selected)),
                        filter(encoded.len(), Some(selected)),
                    ],
                );
            }
            pool(
                &mut writer,
                section.id(),
                count,
                data,
                page,
                (mode == 3).then_some(reference),
                dictionary,
            );
        } else {
            let mut reader =
                Reader::open_auto(fs::File::open(source).unwrap(), Limits::default()).unwrap();
            writer
                .write_section(
                    section.id(),
                    section.kind(),
                    &reader.read_section(section.id()).unwrap(),
                    info,
                )
                .unwrap();
        }
    }
    let middle = dictionary.len() / 2;
    let mut data = Vec::new();
    let mut page = Vec::new();
    for part in [&dictionary[..middle], &dictionary[middle..]] {
        let encoded = frame(part, &[], 3, false);
        stored(
            &mut data,
            &mut page,
            &encoded,
            vec![filter(part.len(), None)],
        );
    }
    let mut extents = Vec::new();
    for value in [
        2,
        if mode == 4 { 3 } else { 0 },
        0,
        middle as u64,
        1,
        0,
        (dictionary.len() - middle) as u64,
    ] {
        binary::write_vuint(&mut extents, value).unwrap();
    }
    page.push(1);
    binary::write_sized(&mut page, &extents).unwrap();
    let encoded = frame(dictionary, dictionary, 3, true);
    stored(
        &mut data,
        &mut page,
        &encoded,
        vec![filter(
            dictionary.len(),
            Some(BlobRef {
                pool: dictionary_pool,
                index: 2,
            }),
        )],
    );
    pool(
        &mut writer,
        dictionary_pool,
        4,
        data,
        page,
        None,
        dictionary,
    );
    let mut output = writer.finish(Value::map(metadata).unwrap()).unwrap();
    output.extend(
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("janex-bootstrap/build/libs/janex-bootstrap.jar"),
        )
        .unwrap(),
    );
    output
}

#[test]
fn java_and_rust_launch_dictionary_packages_and_reject_forbidden_references() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), r#"
import java.nio.file.*;
public class Main {
    public static void main(String[] args) throws Exception {
        byte[] expected = Files.readAllBytes(Paths.get(args[0]));
        Path resource = Paths.get(Main.class.getResource("/content.bin").toURI());
        if (!java.util.Arrays.equals(Files.readAllBytes(resource), expected)) throw new AssertionError();
        System.out.println("dictionary-ok");
    }
}
"#).unwrap();
    tool(Command::new("javac").current_dir(temp.path()).args([
        "--release",
        "8",
        "-d",
        "classes",
        "Main.java",
    ]));
    let samples = samples();
    let raw = samples.concat();
    let trained = zstd::dict::from_samples(&samples, 4096).unwrap();
    let content = temp.path().join("classes/content.bin");
    fs::write(&content, &raw).unwrap();
    let source = temp.path().join("source.janex");
    let mut options = PackOptions::new(temp.path().join("classes"), &source);
    options.main_class = Some("Main".into());
    options.arguments.push(content.to_str().unwrap().into());
    options.with_launcher = true;
    pack(&options).unwrap();
    for (name, dictionary) in [("raw", &raw), ("trained", &trained)] {
        for mode in 0..5 {
            let target = temp.path().join(format!("{name}-{mode}.janex"));
            fs::write(&target, package(&source, dictionary, mode)).unwrap();
            let mut options = RunOptions::new(&target);
            options.allow_unsigned = true;
            let prepared = prepare(&options);
            let success = match prepared {
                Ok(prepared) => prepared.command().output().unwrap().status.success(),
                Err(_) => false,
            };
            assert_eq!(success, mode == 0, "{name}/{mode}");
            for java in runtimes() {
                let result = Command::new(java)
                    .arg("-jar")
                    .arg(&target)
                    .output()
                    .unwrap();
                assert_eq!(
                    result.status.success(),
                    mode == 0,
                    "{name}/{mode}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                if mode == 0 {
                    assert_eq!(
                        String::from_utf8_lossy(&result.stdout).trim(),
                        "dictionary-ok"
                    );
                } else {
                    assert!(
                        String::from_utf8_lossy(&result.stderr)
                            .to_lowercase()
                            .contains("dictionar")
                    );
                }
            }
        }
    }
}

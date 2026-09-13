// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Native/Java policy boundaries for CBOR, applications, containers, and launch preparation.

use janex_format::{
    ErrorKind, Result,
    application::Application,
    binary::Limits,
    cbor::{self, Value},
    checksum::{Algorithm, Checksum},
    condition::Context,
    container::Reader,
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
};

/// Writes a trusted harness byte sequence.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Records one operation and the native success/invalid/limit distinction.
fn vector(
    output: &mut Vec<u8>,
    mode: u8,
    limits: Limits,
    encoded: &[u8],
    info: &[u8],
    result: Result<()>,
) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap());
    output[..4].copy_from_slice(&(count + 1).to_be_bytes());
    output.push(mode);
    for value in [
        limits.max_bytes,
        limits.max_elements,
        limits.max_depth as u64,
    ] {
        output.extend((value as u32).to_be_bytes());
    }
    bytes(output, encoded);
    bytes(output, info);
    output.push(match result {
        Ok(()) => 1,
        Err(error) if error.kind() == ErrorKind::Limit => 2,
        Err(_) => 0,
    });
}

/// Builds a deterministic unsigned container with checksummed sections.
fn container(sections: &[(u64, Vec<u8>, Value)]) -> Vec<u8> {
    let descriptors = sections.iter().enumerate().map(|(id, (kind, body, info))| {
        Value::map([
            (Value::uint(0), Value::uint(*kind)),
            (Value::uint(1), Value::uint(id as u64)),
            (Value::uint(2), Value::uint(body.len() as u64)),
            (
                Value::uint(3),
                Value::bytes(
                    &Checksum::compute(Algorithm::Sha256, body.as_slice())
                        .unwrap()
                        .encode(),
                ),
            ),
            (Value::uint(4), info.clone()),
        ])
        .unwrap()
    });
    let value = Value::map([(Value::uint(0), Value::array(descriptors))]).unwrap();
    let mut metadata = b"METADATA\0\0\0\0\x01\0\0\0".to_vec();
    cbor::write_sized(&mut metadata, &value).unwrap();
    metadata.extend([0, 0]);
    let mut output = b"JANEX\0\0\0".to_vec();
    for (_, body, _) in sections {
        output.extend(body);
    }
    output.extend(&metadata);
    let length = output.len() as u64 + 24;
    output.extend(b"JANEXEND");
    output.extend((metadata.len() as u64 + 24).to_le_bytes());
    output.extend(length.to_le_bytes());
    output
}

/// Encodes a complete application section without imposing reader limits.
fn application(value: &Value) -> Vec<u8> {
    let mut encoded = b"JANEXAPP".to_vec();
    cbor::write_sized(&mut encoded, value).unwrap();
    encoded
}

#[test]
fn java_limits_match_native_decoding_and_are_inherited_by_nested_readers() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/bootstrap.jar");
    let compiled = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&bootstrap)
        .arg("-d")
        .arg(temp.path())
        .arg(project.join("janex-bootstrap/src/testFixtures/java/org/janex/format/LimitsTest.java"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut vectors = vec![0; 4];
    let mut values = vec![
        Value::uint(0),
        Value::uint(u64::MAX),
        Value::text(""),
        Value::text("\u{1f600}"),
        Value::bytes(&[7; 32]),
        Value::empty_map(),
        Value::array([]),
        Value::array((0..8).map(Value::uint)),
        Value::map((0..8).map(|key| (Value::uint(key), Value::uint(key)))).unwrap(),
    ];
    let mut nested = Value::uint(0);
    for _ in 0..12 {
        nested = Value::array([nested]);
        values.push(nested.clone());
    }
    for value in &values {
        for max_bytes in [0, 1, 2, 8, 32, 64] {
            for max_elements in [0, 1, 4, 8] {
                for max_depth in [0, 1, 4, 8, 16] {
                    let limits = Limits {
                        max_bytes,
                        max_elements,
                        max_depth,
                    };
                    vector(
                        &mut vectors,
                        0,
                        limits,
                        value.as_bytes(),
                        &[],
                        Value::from_bytes(value.as_bytes(), limits).map(|_| ()),
                    );
                }
            }
        }
    }
    for encoded in [
        vec![0x5a, 0x80, 0, 0, 0],
        vec![0x9b, 255, 255, 255, 255, 255, 255, 255, 255],
        vec![0xa1, 0],
    ] {
        vector(
            &mut vectors,
            0,
            Limits::default(),
            &encoded,
            &[],
            Value::from_bytes(&encoded, Limits::default()).map(|_| ()),
        );
    }
    let info = Value::map([
        (Value::uint(0), Value::text("main")),
        (Value::uint(1), Value::text("janex.java")),
    ])
    .unwrap();
    let config = Value::map([(
        Value::uint(1),
        Value::map([(Value::uint(0), Value::text("Main"))]).unwrap(),
    )])
    .unwrap();
    let value = Value::map([
        (
            Value::uint(0),
            Value::map([(Value::uint(0), config)]).unwrap(),
        ),
        (
            Value::uint(1),
            Value::map([(Value::text("en"), Value::text("Title"))]).unwrap(),
        ),
    ])
    .unwrap();
    let encoded = application(&value);
    for max_bytes in [0, encoded.len() as u64 - 1, encoded.len() as u64, 4096] {
        for max_elements in [0, 1, 2, 16] {
            for max_depth in 0..10 {
                let limits = Limits {
                    max_bytes,
                    max_elements,
                    max_depth,
                };
                vector(
                    &mut vectors,
                    1,
                    limits,
                    &encoded,
                    info.as_bytes(),
                    Application::decode(&encoded, info.clone(), limits).map(|_| ()),
                );
            }
        }
    }
    let encoded = container(&[(u64::MAX, vec![42; 2048], Value::empty_map())]);
    for mode in [2, 3] {
        for max_bytes in [0, 23, 24, 100, 256, 2047, 2048, 4096] {
            for max_elements in [0, 1, 4, 5, 8] {
                for max_depth in 0..7 {
                    let limits = Limits {
                        max_bytes,
                        max_elements,
                        max_depth,
                    };
                    for buffered in [false, true] {
                        let result = (|| {
                            let mut reader = if mode == 3 {
                                Reader::open_auto(Cursor::new(&encoded), limits)?
                            } else {
                                Reader::open(Cursor::new(&encoded), 0, limits)?
                            };
                            reader.verify_checksums()?;
                            if buffered {
                                reader.read_section(0)?;
                            }
                            Ok(())
                        })();
                        vector(
                            &mut vectors,
                            mode,
                            limits,
                            &encoded,
                            if buffered { &[1] } else { &[] },
                            result,
                        );
                    }
                }
            }
        }
    }
    // Application validation must include unselected sections before launch selection.
    for valid in [true, false] {
        let second = Value::map([
            (Value::uint(0), Value::empty_map()),
            (
                Value::uint(1),
                if valid {
                    Value::text("Other")
                } else {
                    Value::uint(0)
                },
            ),
        ])
        .unwrap();
        let second_info = Value::map([
            (Value::uint(0), Value::text("other")),
            (Value::uint(1), Value::text("org.example.unknown")),
        ])
        .unwrap();
        let second_bytes = application(&second);
        let encoded = container(&[
            (
                janex_format::container::APPLICATION,
                application(&value),
                info.clone(),
            ),
            (
                janex_format::container::APPLICATION,
                second_bytes.clone(),
                second_info.clone(),
            ),
        ]);
        vector(
            &mut vectors,
            4,
            Limits::default(),
            &encoded,
            &[],
            Application::decode(&second_bytes, second_info, Limits::default()).map(|_| ()),
        );
    }
    // Individually valid lists can exceed the limit only after matching overlays append them.
    let context = Context {
        os: String::new(),
        arch: String::new(),
        invocation: Some("run".into()),
        runtime: None,
    };
    for clears in [false, true] {
        let arguments = Value::array((0..4).map(|_| Value::text("argument")));
        let overlay = Value::map([(
            Value::uint(7),
            if clears {
                Value::null()
            } else {
                arguments.clone()
            },
        )])
        .unwrap();
        let config = Value::map([
            (
                Value::uint(1),
                Value::map([(Value::uint(0), Value::text("Main"))]).unwrap(),
            ),
            (Value::uint(7), arguments),
            (Value::uint(6), Value::array([overlay])),
        ])
        .unwrap();
        let value = Value::map([(
            Value::uint(0),
            Value::map([(Value::uint(0), config)]).unwrap(),
        )])
        .unwrap();
        let body = application(&value);
        let encoded = container(&[(
            janex_format::container::APPLICATION,
            body.clone(),
            info.clone(),
        )]);
        for max_elements in [5, 8] {
            let limits = Limits {
                max_bytes: 16384,
                max_elements,
                max_depth: 32,
            };
            let result = Application::decode(&body, info.clone(), limits)
                .and_then(|application| application.evaluate_java(&context).map(|_| ()));
            vector(&mut vectors, 5, limits, &encoded, &[], result);
        }
    }
    // A small compressed package must reject an oversized decoded file before invoking its decoder.
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("resource.txt"), vec![b'x'; 100_000]).unwrap();
    let package = temp.path().join("compressed.janex");
    let mut options = janex_host::pack::PackOptions::new(&source, &package);
    options.main_class = Some("Main".into());
    janex_host::pack::pack(&options).unwrap();
    let encoded = fs::read(&package).unwrap();
    assert!(encoded.len() < 16_384);
    for max_bytes in [16_384, 200_000, 4 * 1024 * 1024] {
        let limits = Limits {
            max_bytes,
            ..Limits::default()
        };
        let mut options = janex_host::run::RunOptions::new(&package);
        options.allow_unsigned = true;
        options.limits = limits;
        let result = janex_host::run::prepare(&options)
            .map(|_| ())
            .map_err(|error| {
                let message = error.to_string();
                assert!(
                    message.contains("limit") || message.contains("too much memory"),
                    "{message}"
                );
                janex_format::Error::new(ErrorKind::Limit, message)
            });
        vector(&mut vectors, 5, limits, &encoded, &[], result);
    }
    let fixture = temp.path().join("limits.bin");
    // Native codec errors and format-limit errors both reject oversized advertised windows.
    for window in (0..=48).chain([80, 255]) {
        let encoded = [0x28, 0xb5, 0x2f, 0xfd, 0, window, 9, 0, 0, b'x'];
        for max_bytes in [1024, 2047, 2048, 4096, 65_536] {
            let limits = Limits {
                max_bytes,
                ..Limits::default()
            };
            vector(
                &mut vectors,
                6,
                limits,
                &encoded,
                &[1],
                janex_format::blob::decode_zstd(&encoded, &[], 1, limits).map(|_| ()),
            );
        }
    }
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
            .arg("org.janex.format.LimitsTest")
            .arg(&fixture)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

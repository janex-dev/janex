// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Shared deterministic CBOR and binary framing vectors across Java runtimes.

use janex_format::{
    Result,
    binary::{self, Decoder, Limits},
    cbor::{self, Value},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Records acceptance and the decoded map length or integer without equating error messages.
fn vector(output: &mut Vec<u8>, mode: u8, bytes: &[u8]) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap());
    output[..4].copy_from_slice(&(count + 1).to_be_bytes());
    output.push(mode);
    output.extend((bytes.len() as u32).to_be_bytes());
    output.extend(bytes);
    let result: Result<u64> = (|| {
        let mut cursor = Decoder::new(bytes, Limits::default())?;
        if mode == 2 {
            let value = cursor.vuint()?;
            cursor.finish()?;
            return Ok(value);
        }
        let value = if mode == 1 {
            let value = cbor::read_sized(&mut cursor)?;
            value.as_map()?;
            cursor.finish()?;
            value
        } else {
            Value::from_bytes(bytes, Limits::default())?
        };
        Ok(if value.as_bytes()[0] >> 5 == 5 {
            value.as_map()?.len() as u64
        } else {
            u64::MAX
        })
    })();
    output.push(u8::from(result.is_ok()));
    output.extend(result.unwrap_or(0).to_be_bytes());
}

#[test]
fn java_matches_native_cbor_float_widths_unknown_values_and_binary_framing() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/bootstrap.jar");
    let compiled = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&bootstrap)
        .arg("-d")
        .arg(temp.path())
        .arg(project.join("janex-reader/src/testFixtures/java/org/janex/format/CborTest.java"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut encodings = BTreeSet::new();
    for first in 0..=255 {
        encodings.insert(vec![first]);
        for second in 0..=255 {
            encodings.insert(vec![first, second]);
        }
    }
    for bits in 0..=u16::MAX {
        let mut bytes = vec![0xf9];
        bytes.extend(bits.to_be_bytes());
        encodings.insert(bytes);
    }
    for sign in [0, 1u64 << 63] {
        for exponent in 0..=2047 {
            for fraction in [
                0,
                1,
                (1 << 29) - 1,
                1 << 29,
                (1 << 42) - 1,
                1 << 42,
                (1u64 << 52) - 1,
            ] {
                let bits = sign | exponent << 52 | fraction;
                let mut bytes = vec![0xfb];
                bytes.extend(bits.to_be_bytes());
                encodings.insert(bytes);
                let mut bytes = vec![0xfa];
                bytes.extend((f64::from_bits(bits) as f32).to_be_bytes());
                encodings.insert(bytes);
            }
        }
    }
    for major in 0..=6 {
        for (info, length) in [(24, 1), (25, 2), (26, 4), (27, 8)] {
            for number in [
                0u64,
                23,
                24,
                255,
                256,
                65535,
                65536,
                u32::MAX as u64,
                u32::MAX as u64 + 1,
                u64::MAX,
            ] {
                let mut bytes = vec![major << 5 | info];
                bytes.extend(&number.to_be_bytes()[8 - length..]);
                encodings.insert(bytes.clone());
                bytes.push(0);
                encodings.insert(bytes);
            }
        }
    }
    let keys: Vec<_> = [
        "00", "20", "40", "4100", "60", "6161", "80", "8100", "a0", "a10000", "c000", "c24100",
        "f0", "f4", "f6", "f7", "f820", "f90000", "f98000",
    ]
    .into_iter()
    .map(hex)
    .collect();
    for left in &keys {
        for right in &keys {
            let mut map = vec![0xa2];
            map.extend(left);
            map.push(0);
            map.extend(right);
            map.push(0);
            encodings.insert(map);
        }
    }
    for bytes in [
        "61ff",
        "62c080",
        "63eda080",
        "64f4908080",
        "64f09f9880",
        "63efbfbd",
        "9f00ff",
        "bf0000ff",
        "5f4000ff",
        "7f6000ff",
        "c240",
        "c34100",
        "a11863c240",
    ] {
        encodings.insert(hex(bytes));
    }
    for prefix in [0x81, 0xa1, 0xc0] {
        for count in [1, 63, 64, 65, 66] {
            let mut bytes = vec![prefix; count];
            bytes.extend(vec![0; if prefix == 0xa1 { count + 1 } else { 1 }]);
            encodings.insert(bytes);
        }
    }
    let mut vectors = vec![0; 4];
    for bytes in encodings {
        vector(&mut vectors, 0, &bytes);
        if bytes.first().is_some_and(|first| first >> 5 == 5) {
            let mut sized = Vec::new();
            binary::write_sized(&mut sized, &bytes).unwrap();
            vector(&mut vectors, 1, &sized);
        }
    }
    for bytes in [
        "00",
        "8000",
        "8100a0",
        "01a0",
        "0100",
        "03a10000",
        "04a1000000",
        "03a100",
        "80808080808080808002",
    ] {
        vector(&mut vectors, 1, &hex(bytes));
    }
    for length in 0..=11 {
        for repeated in [0, 0x7f, 0x80, 0xff] {
            for last in [0, 1, 2, 0x7f, 0x80, 0xff] {
                let mut bytes = vec![repeated; length];
                bytes.push(last);
                vector(&mut vectors, 2, &bytes);
            }
        }
    }
    let fixture = temp.path().join("cbor.bin");
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
            .arg("org.janex.format.CborTest")
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

/// Decodes compact, trusted test data.
fn hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Structural class-file rejection and exact transform parity on Java 8 and current Java.

use janex_format::{
    binary::{self, Limits},
    classfile,
    data_pool::{DataPool, DataPoolBuilder},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Writes a trusted length-prefixed harness value.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Records exact native acceptance and successful transform bytes under one limit policy.
fn vector(
    output: &mut Vec<u8>,
    transform: bool,
    encoded: &[u8],
    length: usize,
    pool: &DataPool,
    limits: Limits,
) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap());
    output[..4].copy_from_slice(&(count + 1).to_be_bytes());
    output.push(u8::from(transform));
    for limit in [
        limits.max_bytes,
        limits.max_elements,
        limits.max_depth as u64,
    ] {
        output.extend((limit as u32).to_be_bytes());
    }
    bytes(output, encoded);
    output.extend((length as u32).to_be_bytes());
    output.extend((pool.len() as u32).to_be_bytes());
    for index in 0..pool.len() {
        bytes(output, pool.get(index as u64).unwrap());
    }
    let restored = if transform {
        classfile::restore(encoded, pool, limits)
            .ok()
            .filter(|value| value.len() == length)
    } else {
        classfile::inspect(encoded, limits).ok().map(|_| Vec::new())
    };
    output.push(u8::from(restored.is_some()));
    bytes(output, &restored.unwrap_or_default());
}

/// Checks an ordinary candidate and the corresponding transform without external strings.
fn ordinary(output: &mut Vec<u8>, encoded: &[u8], limits: Limits) {
    let pool = DataPool::new();
    vector(output, false, encoded, 0, &pool, limits);
    let mut transformed = encoded.to_vec();
    if transformed.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) {
        transformed[..4].copy_from_slice(&[0xca, 0xfe, 0xca, 0x70]);
    }
    vector(output, true, &transformed, encoded.len(), &pool, limits);
}

/// Compiles an independent Java fixture and reports compiler diagnostics.
fn javac(directory: &Path, arguments: &[&str]) {
    let output = Command::new("javac")
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Creates a minimal ordinary class with one extra, unused constant record.
fn minimal(extra: &[u8], wide: bool) -> Vec<u8> {
    let mut value = vec![
        0xca,
        0xfe,
        0xba,
        0xbe,
        0,
        0,
        0,
        55,
        0,
        5 + u8::from(!extra.is_empty()) + u8::from(wide),
    ];
    value.extend([1, 0, 4]);
    value.extend(b"Test");
    value.extend([7, 0, 1, 1, 0, 16]);
    value.extend(b"java/lang/Object");
    value.extend([7, 0, 3]);
    value.extend(extra);
    value.extend([0, 0x21, 0, 2, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0]);
    value
}

#[test]
fn java_class_validation_and_restoration_match_native_structural_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/build/libs/janex-bootstrap.jar");
    let harness = project.join(
        "janex-reader/src/testFixtures/java/org/glavo/janex/reader/internal/codec/ClassFileTest.java",
    );
    javac(
        temp.path(),
        &[
            "--release",
            "8",
            "-cp",
            bootstrap.to_str().unwrap(),
            "-d",
            "harness",
            harness.to_str().unwrap(),
        ],
    );
    fs::write(temp.path().join("Fixture.java"), r#"
package sample;
public class Fixture implements Runnable {
    public java.util.Map<String, java.util.List<? extends Number[]>> generic;
    public <LONG extends Object & java.io.Serializable> LONG identity(LONG value, String[][] array) { return value; }
    static final String TEXT = "A long string with \u0000 and \ud83d\ude80 and \ud800";
    static final long LONG = 1234567890123L;
    static final double DOUBLE = 1.25;
    public void run() { Runnable action = () -> System.out.println(TEXT); action.run(); }
    public int guarded(int value) { try { return 100 / value; } catch (ArithmeticException ex) { return -1; } }
    public native void nativeMethod();
}
"#).unwrap();
    javac(
        temp.path(),
        &["--release", "8", "-d", "classes", "Fixture.java"],
    );
    fs::write(temp.path().join("module-info.java"), "module sample.module { exports sample; opens sample; uses java.lang.Runnable; provides java.lang.Runnable with sample.Fixture; }").unwrap();
    javac(
        temp.path(),
        &[
            "--release",
            "9",
            "-d",
            "modules",
            "module-info.java",
            "Fixture.java",
        ],
    );
    let mut vectors = vec![0; 4];
    let limits = Limits::default();
    let classes = [
        fs::read(temp.path().join("classes/sample/Fixture.class")).unwrap(),
        fs::read(temp.path().join("modules/module-info.class")).unwrap(),
        minimal(&[], false),
    ];
    for original in classes {
        assert!(classfile::inspect(&original, limits).is_ok());
        ordinary(&mut vectors, &original, limits);
        for position in 0..original.len() {
            ordinary(&mut vectors, &original[..position], limits);
            for byte in [0, 1, 0x7f, 0xff] {
                let mut mutated = original.clone();
                mutated[position] = byte;
                ordinary(&mut vectors, &mutated, limits);
            }
        }
        let mut trailing = original.clone();
        trailing.push(0);
        ordinary(&mut vectors, &trailing, limits);
        let mut pool = DataPoolBuilder::new();
        if let Some(transformed) = classfile::transform(&original, &mut pool, limits).unwrap() {
            vector(
                &mut vectors,
                true,
                &transformed,
                original.len(),
                &pool,
                limits,
            );
            for position in 0..transformed.len() {
                let mut mutated = transformed.clone();
                mutated[position] ^= 0xff;
                vector(&mut vectors, true, &mutated, original.len(), &pool, limits);
            }
            for max_bytes in [0, original.len() - 1, original.len(), original.len() + 1] {
                for max_elements in [0, 1, 4, 5, 65535] {
                    let policy = Limits {
                        max_bytes: max_bytes as u64,
                        max_elements,
                        ..limits
                    };
                    ordinary(&mut vectors, &original, policy);
                    vector(
                        &mut vectors,
                        true,
                        &transformed,
                        original.len(),
                        &pool,
                        policy,
                    );
                }
            }
        }
    }
    for modified in [
        vec![],
        vec![0],
        vec![0xc0, 0x80],
        vec![0xc0, 0x81],
        vec![0xc1, 0xbf],
        vec![0xe0, 0x80, 0x80],
        vec![0xed, 0xa0, 0x80],
        vec![0xed, 0xb0, 0x80],
        vec![0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80],
        vec![0xf0, 0x9f, 0x98, 0x80],
        vec![0x80],
        vec![0xc2],
        vec![0xe0, 0xa0],
    ] {
        let mut extra = vec![1];
        extra.extend((modified.len() as u16).to_be_bytes());
        extra.extend_from_slice(&modified);
        let original = minimal(&extra, false);
        ordinary(&mut vectors, &original, limits);
        let mut pool = DataPoolBuilder::new();
        let index = pool.intern(&modified);
        let mut transformed = minimal(&[0xff, index as u8], false);
        transformed[..4].copy_from_slice(&[0xca, 0xfe, 0xca, 0x70]);
        if classfile::inspect(&original, limits).is_ok() {
            assert_eq!(
                classfile::restore(&transformed, &pool, limits).unwrap(),
                original
            );
        }
        vector(
            &mut vectors,
            true,
            &transformed,
            original.len(),
            &pool,
            limits,
        );
    }
    for tag in 0..=255 {
        let mut extra = vec![tag];
        extra.extend([0; 8]);
        ordinary(&mut vectors, &minimal(&extra, tag == 5 || tag == 6), limits);
    }
    // Replace the first UTF-8 record by either external form and vary exact Modified UTF-8 lengths.
    let original = minimal(&[], false);
    for text in [
        "".to_owned(),
        "Test".to_owned(),
        "a\0🚀".to_owned(),
        "x".repeat(65535),
        "x".repeat(65536),
        "\0".repeat(32768),
        "🚀".repeat(10923),
    ] {
        let mut pool = DataPoolBuilder::new();
        let name = pool.intern(&text);
        let package = pool.intern("sample");
        for prefix in [None, Some(0), Some(package)] {
            let mut transformed = original[..10].to_vec();
            transformed[..4].copy_from_slice(&[0xca, 0xfe, 0xca, 0x70]);
            transformed.push(if prefix.is_some() { 0xfe } else { 0xff });
            if let Some(prefix) = prefix {
                binary::write_vuint(&mut transformed, prefix).unwrap();
            }
            binary::write_vuint(&mut transformed, name).unwrap();
            transformed.extend(&original[17..]);
            let length = classfile::restore(&transformed, &pool, limits)
                .map_or(original.len(), |value| value.len());
            for declared in [length.saturating_sub(1), length, length + 1] {
                vector(&mut vectors, true, &transformed, declared, &pool, limits);
            }
        }
    }
    // Exercise nonrecursive templates, multi-byte indices, and malformed expansion framing.
    for template in [
        b"(L\0\x01\x02;)L\0\x01\x02;".to_vec(),
        vec![0, 0, 2],
        vec![0, 0x81, 0, 0x82, 0],
        vec![0],
        vec![0, 1],
        vec![0, 1, 0],
        vec![0, 1, 127],
        vec![0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2],
        vec![b'x'; 65535],
        vec![b'x'; 65536],
    ] {
        for name in [b"String".to_vec(), vec![0, 1, 2], vec![b'x'; 65535]] {
            let mut pool = DataPoolBuilder::new();
            pool.intern("java/lang");
            pool.intern(name);
            let index = pool.intern(&template);
            let mut transformed = minimal(&[0xfd, index as u8], false);
            transformed[..4].copy_from_slice(&[0xca, 0xfe, 0xca, 0x70]);
            let length =
                classfile::restore(&transformed, &pool, limits).map_or(100, |value| value.len());
            for declared in [length.saturating_sub(1), length, length + 1] {
                vector(&mut vectors, true, &transformed, declared, &pool, limits);
            }
        }
    }
    let fixture = temp.path().join("classes.bin");
    fs::write(&fixture, vectors).unwrap();
    let classpath = std::env::join_paths([temp.path().join("harness"), bootstrap]).unwrap();
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
            .arg("org.glavo.janex.reader.internal.codec.ClassFileTest")
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

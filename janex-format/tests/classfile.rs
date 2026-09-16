// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Independent class-file bytes and real JDK transformation fixtures.

use janex_format::{
    ErrorKind,
    binary::Limits,
    classfile,
    data_pool::{DataPool, DataPoolBuilder},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// Builds one framed UTF-8 constant without requiring a semantically valid class body.
fn string_class(text: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52, 0, 2, 1];
    bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
    bytes.extend_from_slice(text);
    bytes
}

#[test]
fn descriptors_and_generic_signatures_share_byte_templates() {
    let mut pool = DataPoolBuilder::new();
    pool.intern("java/lang");
    pool.intern("String");
    pool.intern("Object");
    let original = string_class(b"(Ljava/lang/String;)Ljava/lang/Object;");
    let encoded = classfile::transform(&original, &mut pool, Limits::default())
        .unwrap()
        .unwrap();
    assert_eq!(&encoded[10..], &[0xfd, 4]);
    assert_eq!(pool.get(4).unwrap(), b"(L\0\x01\x02;)L\0\x01\x03;");
    let checkpoint = pool.len();
    assert_eq!(
        classfile::transform(&original, &mut pool, Limits::default())
            .unwrap()
            .unwrap(),
        encoded
    );
    assert_eq!(pool.len(), checkpoint);
    assert_eq!(
        classfile::restore(&encoded, &pool, Limits::default()).unwrap(),
        original
    );
    for text in [
        "[[Ljava/lang/String;",
        "<LONG:Ljava/lang/Object;:Ljava/io/Serializable;>(TLONG;[Ljava/lang/String;)TLONG;^Ljava/lang/Exception;",
        "Ljava/util/Map<Ljava/lang/String;+Ljava/util/List<-[Ljava/lang/Number;>;>;",
        "Lsample/Outer<TT;>.Inner<Ljava/lang/String;>;",
        "(LDefaultName;L包/类型;)LDefaultName;",
        "(ILjava/lang/String;[[DZ)V",
        "LLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLL",
        "(Ljava/lang/String;not-a-valid-descriptor",
    ] {
        let original = string_class(text.as_bytes());
        let encoded = classfile::transform(&original, &mut pool, Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            classfile::restore(&encoded, &pool, Limits::default()).unwrap(),
            original,
            "{text}"
        );
    }
    assert!(pool.find("java/util").is_some());
    assert!(pool.find("Map").is_some());
    assert!(pool.find("包").is_some());
    assert!(pool.find("类型").is_some());
}

#[test]
fn templates_bound_expansion_and_do_not_interpret_referenced_bytes() {
    for (template, package, name, expected) in [
        (
            vec![0, 0, 2],
            vec![],
            vec![0xc0, 0x80],
            Some(vec![0xc0, 0x80]),
        ),
        (
            vec![0, 1, 2],
            b"p".to_vec(),
            vec![0, 1, 2],
            Some(vec![b'p', b'/', 0, 1, 2]),
        ),
        (vec![0, 1], b"p".to_vec(), b"Name".to_vec(), None),
        (vec![0, 1, 127], b"p".to_vec(), b"Name".to_vec(), None),
        (vec![0, 1, 0], b"p".to_vec(), b"Name".to_vec(), None),
        (
            vec![0, 0, 2],
            b"p".to_vec(),
            vec![b'x'; 65535],
            Some(vec![b'x'; 65535]),
        ),
        (vec![0, 1, 2], b"p".to_vec(), vec![b'x'; 65535], None),
        (vec![b'x'; 65536], b"p".to_vec(), b"Name".to_vec(), None),
    ] {
        let mut pool = DataPoolBuilder::new();
        // Keep stable indices even for the unnamed-package vector.
        pool.intern(if package.is_empty() {
            b"unused".as_slice()
        } else {
            &package
        });
        pool.intern(&name);
        let index = pool.intern(&template);
        let mut encoded = vec![0xca, 0xfe, 0xca, 0x70, 0, 0, 0, 52, 0, 2, 0xfd];
        janex_format::binary::write_vuint(&mut encoded, index).unwrap();
        let result = classfile::restore(&encoded, &pool, Limits::default());
        match expected {
            Some(expected) => assert_eq!(result.unwrap(), string_class(&expected)),
            None => assert!(result.is_err()),
        }
    }
}

/// Builds a minimal class with unused constants that exercise UTF-16 and wide slots.
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52, 0, 10];
    for name in [b"sample/Example".as_slice(), b"java/lang/Object"] {
        let index = if name == b"sample/Example" { 1u16 } else { 3 };
        bytes.push(1);
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name);
        bytes.push(7);
        bytes.extend_from_slice(&index.to_be_bytes());
    }
    bytes.extend_from_slice(&[1, 0, 3, 0xed, 0xa0, 0x80]); // Unpaired high surrogate.
    bytes.extend_from_slice(&[5, 0, 0, 0, 0, 0, 0, 0, 1]); // Long, including reserved slot 7.
    bytes.extend_from_slice(&[6, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0]); // Double, including slot 9.
    bytes.extend_from_slice(&[0, 0x21, 0, 2, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

#[test]
fn class_transform_restores_exact_bytes_and_preserves_unpaired_surrogates() {
    let bytes = fixture();
    assert_eq!(
        classfile::inspect(&bytes, Limits::default()).unwrap().name,
        "sample/Example"
    );
    let mut pool = DataPoolBuilder::new();
    let encoded = classfile::transform(&bytes, &mut pool, Limits::default())
        .unwrap()
        .unwrap();
    assert_eq!(&encoded[..4], b"\xca\xfe\xca\x70");
    assert_eq!(&encoded[10..13], &[0xfe, 1, 2]);
    assert_eq!(pool.get(1).unwrap(), b"sample");
    assert_eq!(pool.get(2).unwrap(), b"Example");
    assert!(
        encoded
            .windows(6)
            .any(|window| window == [1, 0, 3, 0xed, 0xa0, 0x80])
    );
    assert_eq!(
        classfile::restore(&encoded, &pool, Limits::default()).unwrap(),
        bytes
    );
    assert!(classfile::restore(&encoded, &DataPool::new(), Limits::default()).is_err());
    for end in 0..bytes.len() {
        assert!(
            classfile::inspect(&bytes[..end], Limits::default()).is_err(),
            "prefix {end}"
        );
    }
    let body_start = encoded.len() - 14;
    for end in 0..body_start {
        assert!(
            classfile::restore(&encoded[..end], &pool, Limits::default()).is_err(),
            "transformed prefix {end}"
        );
    }
    for end in body_start..encoded.len() {
        let restored = classfile::restore(&encoded[..end], &pool, Limits::default()).unwrap();
        assert!(classfile::inspect(&restored, Limits::default()).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(classfile::inspect(&trailing, Limits::default()).is_err());
    let limits = Limits {
        max_bytes: encoded.len() as u64,
        ..Limits::default()
    };
    assert_eq!(
        classfile::restore(&encoded, &pool, limits)
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
}

#[test]
fn external_strings_check_modified_utf8_length_and_empty_class_names() {
    let bytes = fixture();
    let mut pool = DataPoolBuilder::new();
    let mut encoded = classfile::transform(&bytes, &mut pool, Limits::default())
        .unwrap()
        .unwrap();
    encoded[12] = 0;
    assert!(classfile::restore(&encoded, &pool, Limits::default()).is_err());
    let mut oversized = DataPoolBuilder::new();
    oversized.intern("sample");
    oversized.intern([0xc0, 0x80].repeat(40000));
    encoded[12] = 2;
    assert!(classfile::restore(&encoded, &oversized, Limits::default()).is_err());
    let mut invalid = bytes;
    invalid[10] = 5;
    assert!(classfile::inspect(&invalid, Limits::default()).is_err());
}

#[test]
fn external_constants_copy_raw_modified_utf8_without_transcoding() {
    let base = fixture();
    let position = base
        .windows(6)
        .position(|bytes| bytes == [1, 0, 3, 0xed, 0xa0, 0x80])
        .unwrap();
    for raw in [
        vec![0xc0, 0x80],
        vec![0xed, 0xa0, 0x80],
        vec![0xed, 0xb0, 0x80],
        vec![0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80],
        vec![b'x'; 65535],
    ] {
        let mut ordinary = base[..position].to_vec();
        ordinary.push(1);
        ordinary.extend_from_slice(&(raw.len() as u16).to_be_bytes());
        ordinary.extend_from_slice(&raw);
        ordinary.extend_from_slice(&base[position + 6..]);
        classfile::inspect(&ordinary, Limits::default()).unwrap();
        let mut pool = DataPoolBuilder::new();
        pool.intern(&raw);
        let mut transformed = base[..position].to_vec();
        transformed[..4].copy_from_slice(b"\xca\xfe\xca\x70");
        transformed.extend_from_slice(&[0xff, 1]);
        transformed.extend_from_slice(&base[position + 6..]);
        assert_eq!(
            classfile::restore(&transformed, &pool, Limits::default()).unwrap(),
            ordinary
        );
        for invalid in [vec![0], vec![0xf0, 0x9f, 0x98, 0x80]] {
            let mut bad_pool = DataPoolBuilder::new();
            bad_pool.intern(&invalid);
            let restored = classfile::restore(&transformed, &bad_pool, Limits::default()).unwrap();
            assert_eq!(
                &restored[position + 3..position + 3 + invalid.len()],
                invalid
            );
            assert!(classfile::inspect(&restored, Limits::default()).is_err());
        }
        let mut oversized = DataPoolBuilder::new();
        oversized.intern(vec![b'x'; 65536]);
        assert!(classfile::restore(&transformed, &oversized, Limits::default()).is_err());
    }
}

#[test]
fn transform_preserves_uninterpreted_versions_references_and_class_bodies() {
    let mut original = fixture();
    original[6..8].fill(0);
    let body_start = original.len() - 14;
    original[body_start..].fill(0xff);
    // Retain an uninterpreted class name that cannot use the split-name form.
    original[13..27].copy_from_slice(b"/ample_Example");
    let mut pool = DataPoolBuilder::new();
    let encoded = classfile::transform(&original, &mut pool, Limits::default())
        .unwrap()
        .unwrap();
    assert_eq!(
        classfile::restore(&encoded, &pool, Limits::default()).unwrap(),
        original
    );
    assert!(classfile::inspect(&original, Limits::default()).is_err());
}

/// Owns an exclusively created scratch directory for one JDK test.
struct Scratch(PathBuf);

impl Scratch {
    /// Creates a unique absolute directory without reusing existing paths.
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("janex-classfile-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for Scratch {
    /// Removes only this test's exclusively owned scratch directory.
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs a JDK tool and includes both streams when it fails.
fn tool(directory: &Path, program: &str, args: &[&str]) -> std::process::Output {
    let output = Command::new(program)
        .args(args)
        .current_dir(directory)
        .output()
        .expect("JDK tools must be available on PATH");
    assert!(
        output.status.success(),
        "{program} {args:?}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn jdk_compiled_classes_and_module_descriptor_round_trip_and_execute() {
    let scratch = Scratch::new();
    let root = &scratch.0;
    fs::create_dir_all(root.join("src/sample")).unwrap();
    fs::write(
        root.join("src/module-info.java"),
        "module sample.app { requires static java.logging; exports sample; }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/sample/Example.java"),
        r#"package sample;
public class Example {
    public static void main(String[] args) {
        long wide = 123456789012345L;
        double fraction = 123.25;
        String value = "\u0000\ud83d\ude00\ud800";
        Runnable action = () -> {
            if (wide != 123456789012345L || fraction != 123.25 || value.length() != 4)
                throw new AssertionError();
            System.out.println("ok");
        };
        action.run();
    }
}
"#,
    )
    .unwrap();
    tool(
        root,
        "javac",
        &[
            "--release",
            "11",
            "-d",
            "classes",
            "src/module-info.java",
            "src/sample/Example.java",
        ],
    );
    let mut pool = DataPoolBuilder::new();
    for path in ["classes/sample/Example.class", "classes/module-info.class"] {
        let original = fs::read(root.join(path)).unwrap();
        let encoded = classfile::transform(&original, &mut pool, Limits::default())
            .unwrap()
            .unwrap();
        let restored = classfile::restore(&encoded, &pool, Limits::default()).unwrap();
        assert_eq!(restored, original);
        fs::write(root.join(path), restored).unwrap();
    }
    let output = tool(root, "java", &["-cp", "classes", "sample.Example"]);
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "ok");
    tool(
        root,
        "jar",
        &[
            "--create",
            "--file",
            "app.jar",
            "--main-class",
            "sample.Example",
            "--module-version",
            "1.2.3",
            "-C",
            "classes",
            ".",
        ],
    );
    fs::create_dir(root.join("extracted")).unwrap();
    tool(
        &root.join("extracted"),
        "jar",
        &["--extract", "--file", "../app.jar", "module-info.class"],
    );
    let descriptor = fs::read(root.join("extracted/module-info.class")).unwrap();
    let encoded = classfile::transform(&descriptor, &mut pool, Limits::default())
        .unwrap()
        .unwrap();
    assert_eq!(
        classfile::restore(&encoded, &pool, Limits::default()).unwrap(),
        descriptor
    );
    let module = classfile::inspect(&descriptor, Limits::default())
        .unwrap()
        .module
        .unwrap();
    assert_eq!(module.name, "sample.app");
    assert_eq!(module.version.as_deref(), Some("1.2.3"));
    assert_eq!(module.main_class.as_deref(), Some("sample.Example"));
    assert!(
        module
            .requires
            .iter()
            .any(|requirement| requirement.name == "java.base")
    );
    assert!(
        module
            .requires
            .iter()
            .any(|requirement| requirement.name == "java.logging" && requirement.flags & 0x40 != 0)
    );
    let output = tool(
        root,
        "java",
        &["--module-path", "app.jar", "--module", "sample.app"],
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "ok");
}

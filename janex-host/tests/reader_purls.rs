// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Package URL syntax, type constraints, exact decoding, and inactive-branch parity.

use janex_format::{
    application::{Application, PathEntry},
    binary::Limits,
    cbor::{self, Value},
    purl,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Encodes one trusted harness byte sequence.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Encodes a component using the ECMA-427 unreserved set and literal colon.
fn escape(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b".-_~:".contains(&byte) {
            result.push(char::from(byte));
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

/// Records parser results and an application whose inactive branch contains this reference.
fn vector(output: &mut Vec<u8>, uri: &str) {
    bytes(output, uri.as_bytes());
    let parsed = purl::parse(uri);
    output.push(u8::from(parsed.is_ok()));
    if let Ok(purl) = parsed {
        assert_eq!(purl.to_string(), uri);
        for value in [
            purl.ty(),
            purl.namespace().unwrap_or(""),
            purl.name(),
            purl.version().unwrap_or(""),
            purl.subpath().unwrap_or(""),
        ] {
            bytes(output, value.as_bytes());
        }
        let mut qualifiers: Vec<_> = purl.qualifiers().iter().collect();
        qualifiers.sort();
        output.extend((qualifiers.len() as u32).to_be_bytes());
        for (key, value) in qualifiers {
            bytes(output, key.as_bytes());
            bytes(output, value.as_bytes());
        }
    }
    let reference = Value::map([
        (Value::uint(0), Value::uint(1)),
        (Value::uint(1), Value::text(uri)),
    ])
    .unwrap();
    for module in [false, true] {
        output.push(u8::from(PathEntry::from_value(&reference, module).is_ok()));
    }
    let inactive = Value::map([
        (
            Value::uint(0),
            Value::map([(Value::uint(1), Value::text("never"))]).unwrap(),
        ),
        (Value::uint(2), Value::array([reference])),
    ])
    .unwrap();
    let config = Value::map([(Value::uint(6), Value::array([inactive]))]).unwrap();
    let value = Value::map([(
        Value::uint(0),
        Value::map([(Value::uint(0), config)]).unwrap(),
    )])
    .unwrap();
    let info = Value::map([
        (Value::uint(0), Value::text("main")),
        (Value::uint(1), Value::text("janex.java")),
    ])
    .unwrap();
    let mut encoded = b"JANEXAPP".to_vec();
    cbor::write_sized(&mut encoded, &value).unwrap();
    bytes(output, &encoded);
    bytes(output, info.as_bytes());
    output.push(u8::from(
        Application::decode(&encoded, info, Limits::default()).is_ok(),
    ));
}

#[test]
fn java_package_urls_match_native_components_type_rules_and_inactive_validation() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/bootstrap.jar");
    let compiled = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&bootstrap)
        .arg("-d")
        .arg(temp.path())
        .arg(
            project.join("janex-reader/src/testFixtures/java/org/janex/reader/PackageUrlTest.java"),
        )
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut uris = BTreeSet::new();
    let types = [
        "generic",
        "janex",
        "maven",
        "apk",
        "bitbucket",
        "bitnami",
        "cargo",
        "chrome-extension",
        "cocoapods",
        "composer",
        "conda",
        "cpan",
        "cran",
        "deb",
        "gem",
        "github",
        "golang",
        "hackage",
        "hex",
        "huggingface",
        "julia",
        "mlflow",
        "npm",
        "nuget",
        "oci",
        "otp",
        "pub",
        "pypi",
        "qpkg",
        "rpm",
        "swift",
        "vcpkg",
        "vscode-extension",
    ];
    for ty in types {
        for namespace in ["", "ns/", "Ns/", "%40scope/", "./", "ns/deep/"] {
            for name in [
                "name",
                "Name",
                "a_b",
                "a::b",
                "abcdefghijklmnopabcdefghijklmnop",
            ] {
                for suffix in [
                    "",
                    "@1",
                    "@A",
                    "@1.2.3.4.5",
                    "?uuid=1",
                    "?repository_url=https:%2F%2Fdatabricks.example",
                ] {
                    uris.insert(format!("pkg:{ty}/{namespace}{name}{suffix}"));
                }
            }
        }
    }
    for ty in ["a+b", ".type", "-type", "1type", "UPPER", "a_type", "a.b-c"] {
        uris.insert(format!("pkg:{ty}/name"));
    }
    for tail in [
        "",
        "/",
        "name@",
        "name?",
        "name#",
        "name?_=v",
        "name?.a=v",
        "name?a=v&a=w",
        "name?z=v&a=w",
        "name?a=",
        "name?a=v=extra",
        "name?a=v&extra",
        "ns//name",
        "ns/../name",
        "name#../p",
        "name#p//q",
        "name#p%2Fq",
    ] {
        uris.insert(format!("pkg:generic/{tail}"));
    }
    for character in (0..=255).filter_map(char::from_u32).chain([
        '中',
        '🚀',
        '\u{130}',
        '\u{1c5}',
        '\u{1c90}',
        '\u{a7cb}',
        '\u{1e900}',
    ]) {
        let part = escape(&format!("a{character}b"));
        for uri in [
            format!("pkg:generic/{part}@1"),
            format!("pkg:generic/{part}/name"),
            format!("pkg:generic/name@{part}"),
            format!("pkg:generic/name?q={part}"),
            format!("pkg:generic/name#{part}"),
            format!("pkg:github/{part}/name"),
            format!("pkg:npm/{part}"),
            format!("pkg:huggingface/ns/name@{part}"),
        ] {
            uris.insert(uri.clone());
            uris.insert(uri.replace("pkg:", "PKG:"));
            uris.insert(uri.replace("pkg:", "pkg://"));
            for (position, _) in uri.match_indices('%') {
                let mut variant = uri.clone();
                variant.replace_range(
                    position..position + 3,
                    &uri[position..position + 3].to_ascii_lowercase(),
                );
                uris.insert(variant);
            }
        }
    }
    let mut vectors = Vec::new();
    vectors.extend((uris.len() as u32).to_be_bytes());
    for uri in uris {
        vector(&mut vectors, &uri);
    }
    // Exercise every scalar value so compiler Unicode updates cannot silently diverge from Java.
    for ch in (0..=0x10ffff).filter_map(char::from_u32) {
        vectors.push(u8::from(ch.to_lowercase().eq(std::iter::once(ch))));
    }
    let fixture = temp.path().join("purls.bin");
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
            .arg("org.janex.reader.PackageUrlTest")
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

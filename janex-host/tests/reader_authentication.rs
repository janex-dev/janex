// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Java caller authentication over exact inputs with native CMS and OpenPGP fixture validation.

use janex_format::{
    application::PathEntry,
    binary::{self, Limits},
    cbor::Value,
    checksum::{Algorithm, Checksum},
    container::{Reader, Verification},
};
use janex_host::{
    authentication::{self, MATERIAL_LIMITS},
    pack::{PackOptions, pack},
};
use janex_signature::{cms, openpgp};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

/// Writes a counted byte string for the Java harness.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Replaces or removes one CBOR field while preserving the other values.
fn field(value: Value, key: u64, replacement: Option<Value>) -> Value {
    let mut entries = value.as_map().unwrap();
    entries.retain(|(candidate, _)| candidate.as_u64().ok() != Some(key));
    if let Some(value) = replacement {
        entries.push((Value::uint(key), value));
    }
    Value::map(entries).unwrap()
}

/// Rebuilds verification data over a deliberately nonminimal but valid Sized prefix.
fn encode(
    original: &[u8],
    kind: u8,
    coverage: u8,
    mutation: u8,
    payload: impl FnOnce(&[u8]) -> Vec<u8>,
) -> Vec<u8> {
    let mut reader = Reader::open(Cursor::new(original), 0, Limits::default()).unwrap();
    let mut metadata = reader.metadata().clone();
    if coverage == 1 {
        metadata = field(metadata, 1, None);
    } else if coverage >= 2 {
        let mut sections = metadata.required(0).unwrap().as_array().unwrap();
        let replacement = if coverage == 2 {
            None
        } else {
            let id = sections[0].required(1).unwrap().as_u64().unwrap();
            Some(Value::bytes(
                &Checksum::compute(
                    Algorithm::Xxh3_64,
                    reader.read_section(id).unwrap().as_slice(),
                )
                .unwrap()
                .encode(),
            ))
        };
        sections[0] = field(sections[0].clone(), 3, replacement);
        metadata = field(metadata, 0, Some(Value::array(sections)));
    }
    let mut input = b"METADATA\0\0\0\0\x01\0\0\0".to_vec();
    let mut size = Vec::new();
    binary::write_vuint(&mut size, metadata.as_bytes().len() as u64).unwrap();
    *size.last_mut().unwrap() |= 0x80;
    size.push(0);
    input.extend(size);
    input.extend(metadata.as_bytes());
    input.push(kind);
    let mut payload = payload(&input);
    if mutation == 1 {
        *payload.last_mut().unwrap() ^= 1;
    }
    binary::write_sized(&mut input, &payload).unwrap();
    let metadata_start = original.len()
        - u64::from_le_bytes(
            original[original.len() - 16..original.len() - 8]
                .try_into()
                .unwrap(),
        ) as usize;
    let mut output = original[..metadata_start].to_vec();
    if mutation == 2 {
        output[16] ^= 1;
    }
    output.extend(&input);
    let length = output.len() + 24;
    output.extend(b"JANEXEND");
    output.extend((input.len() as u64 + 24).to_le_bytes());
    output.extend((length as u64).to_le_bytes());
    output
}

#[test]
fn java_authentication_is_explicit_and_precedes_integrity_and_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixtures = project.join("janex-signature/tests/fixtures");
    let cms = authentication::load_cms_signer(
        &fixtures.join("cms/rsa256.cert.pem"),
        &fixtures.join("cms/rsa256.key.pem"),
        None,
        MATERIAL_LIMITS,
        || panic!("fixture key is not encrypted"),
    )
    .unwrap();
    let pgp = authentication::load_openpgp_signer(
        &fixtures.join("openpgp/ed25519.secret.pgp"),
        None,
        None,
        MATERIAL_LIMITS,
        || panic!("fixture key is not encrypted"),
    )
    .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("data.txt"), b"selected resource").unwrap();
    let mut jar = vec![0; 22];
    jar[..4].copy_from_slice(b"PK\x05\x06");
    let mut options = PackOptions::new(&source, temp.path().join("base.janex"));
    options.main_class = Some("Main".into());
    options.external_class_path.push(PathEntry::External {
        uri: "https://example.invalid/library.jar".into(),
        checksum: Some(Checksum::compute(Algorithm::Sha256, jar.as_slice()).unwrap()),
    });
    pack(&options).unwrap();
    let original = fs::read(&options.output).unwrap();
    let mut vectors = vec![0; 4];
    let mut count = 0u32;
    for kind in 0..=3 {
        for coverage in 0..4 {
            for mutation in 0..=2 {
                if kind == 0 && mutation == 1 || coverage == 2 && mutation == 2 {
                    continue;
                }
                let encoded = encode(&original, kind, coverage, mutation, |input| match kind {
                    0 => Vec::new(),
                    1 => Checksum::compute(Algorithm::Sha256, input)
                        .unwrap()
                        .encode(),
                    2 => pgp.sign(input, SystemTime::now()).unwrap(),
                    3 => cms::sign(input, &[&cms], SystemTime::now()).unwrap(),
                    _ => unreachable!(),
                });
                let mut reader = Reader::open(Cursor::new(&encoded), 0, Limits::default()).unwrap();
                let (payload, trusted) = match reader.verification() {
                    Verification::None => (Vec::new(), false),
                    Verification::Checksum(checksum) => (checksum.encode(), false),
                    Verification::OpenPgp(payload) => {
                        let trusted = openpgp::OpenPgpSignature::decode(payload, MATERIAL_LIMITS)
                            .and_then(|signature| {
                                pgp.certificate().authenticate(
                                    reader.verification_input(),
                                    &signature,
                                    SystemTime::now(),
                                )
                            })
                            .is_ok();
                        (payload.clone(), trusted)
                    }
                    Verification::Cms(payload) => {
                        let trusted = cms::CmsSignature::decode(payload, MATERIAL_LIMITS)
                            .and_then(|signature| {
                                signature.authenticate(
                                    reader.verification_input(),
                                    &[cms.certificate().clone()],
                                    &[],
                                    &[],
                                    SystemTime::now(),
                                )
                            })
                            .is_ok();
                        (payload.clone(), trusted)
                    }
                };
                if kind >= 2 {
                    assert_eq!(trusted, mutation != 1, "signed fixture trust");
                }
                let integrity = reader.verify_checksums();
                assert_eq!(
                    integrity.is_ok(),
                    mutation != 2 && !(kind == 1 && mutation == 1)
                );
                bytes(&mut vectors, &encoded);
                vectors.push(kind);
                bytes(&mut vectors, reader.verification_input());
                bytes(&mut vectors, &payload);
                vectors.push(u8::from(trusted));
                vectors.push(u8::from(integrity.is_ok()));
                vectors.extend(
                    integrity
                        .as_ref()
                        .map_or(0, |report| report.checksums_verified as u32)
                        .to_be_bytes(),
                );
                vectors.push(u8::from(
                    integrity.is_ok_and(|report| report.complete_secure_coverage),
                ));
                count += 1;
            }
        }
    }
    vectors[..4].copy_from_slice(&count.to_be_bytes());
    let fixture = temp.path().join("authentication.bin");
    fs::write(&fixture, vectors).unwrap();
    let bootstrap = project.join("janex-bootstrap/bootstrap.jar");
    let output = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&bootstrap)
        .arg("-d")
        .arg(temp.path())
        .arg(project.join("janex-bootstrap/src/testFixtures/java/org/janex/bootstrap/ReaderAuthenticationTest.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
            .arg("org.janex.bootstrap.ReaderAuthenticationTest")
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

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Complete-container authentication, explicit trust policy, and key-loading boundaries.

use janex_core::{
    authentication::{self, CmsAlgorithm, MATERIAL_LIMITS},
    pack::{PackOptions, PackSigner, pack},
    run::{RunOptions, prepare},
};
use janex_format::{
    ErrorKind,
    binary::Limits,
    cbor::Value,
    container::{Reader, Writer},
    signature::cms,
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};
use zeroize::Zeroizing;

/// Resolves public fixture material without accessing a user key store.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../janex-format/tests/fixtures/cms")
        .join(name)
}

#[test]
fn trusted_metadata_without_complete_content_coverage_cannot_execute() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("source")).unwrap();
    let signer = Arc::new(
        authentication::load_cms_signer(
            &fixture("rsa256.cert.pem"),
            &fixture("rsa256.key.pem"),
            Some(CmsAlgorithm::RsaSha256),
            MATERIAL_LIMITS,
            || panic!("unencrypted keys must not request passwords"),
        )
        .unwrap(),
    );
    let mut packing =
        PackOptions::new(temp.path().join("source"), temp.path().join("signed.janex"));
    packing.main_class = Some("Main".into());
    packing.signer = Some(PackSigner::Cms(signer.clone()));
    pack(&packing).unwrap();
    let original = fs::read(&packing.output).unwrap();
    let mut reader = Reader::open_auto(Cursor::new(original), Limits::default()).unwrap();
    let mut writer = Writer::new(Vec::new()).unwrap();
    for section in reader.sections().cloned().collect::<Vec<_>>() {
        writer
            .write_section(
                section.id(),
                section.kind(),
                &reader.read_section(section.id()).unwrap(),
                section.type_info().unwrap(),
            )
            .unwrap();
    }
    // Omitting external-region constraints leaves those bytes outside the authenticated scope.
    let bytes = writer
        .finish_with(Value::empty_map(), 3, |input| {
            cms::sign(input, &[&signer], SystemTime::now())
        })
        .unwrap();
    let incomplete = temp.path().join("incomplete.janex");
    fs::write(&incomplete, bytes).unwrap();
    let mut options = RunOptions::new(&incomplete);
    options.java.java = Some(temp.path().join("must-not-start-java"));
    options.cms_trust.signers.push(signer.certificate().clone());
    let error = prepare(&options).unwrap_err();
    assert!(matches!(&error, janex_core::Error::Format(error) if error.kind() == ErrorKind::Trust));
    assert!(error.to_string().contains("complete container"));
    packing.output = temp.path().join("unsigned.janex");
    packing.signer = None;
    pack(&packing).unwrap();
    options.target = packing.output;
    options.allow_unsigned = true;
    assert!(
        prepare(&options)
            .unwrap_err()
            .to_string()
            .contains("signer pins require")
    );
}

#[test]
fn encrypted_key_loading_requests_one_password_and_enforces_input_limits() {
    let mut requests = 0;
    authentication::load_cms_signer(
        &fixture("p256.cert.pem"),
        &fixture("p256.encrypted.pem"),
        None,
        MATERIAL_LIMITS,
        || {
            requests += 1;
            Ok(Zeroizing::new(b"public-fixture-password".to_vec()))
        },
    )
    .unwrap();
    assert_eq!(requests, 1);
    assert!(
        authentication::load_certificate(
            &fixture("p256.cert.pem"),
            Limits {
                max_bytes: 16,
                ..MATERIAL_LIMITS
            }
        )
        .is_err()
    );
}

#[test]
fn openpgp_loader_requests_only_the_selected_keys_password_once() {
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../janex-format/tests/fixtures/openpgp");
    let mut requests = 0;
    let signer = authentication::load_openpgp_signer(
        &directory.join("encrypted.secret.asc"),
        None,
        None,
        MATERIAL_LIMITS,
        || {
            requests += 1;
            Ok(Zeroizing::new(b"public-fixture-password".to_vec()))
        },
    )
    .unwrap();
    assert_eq!(requests, 1);
    assert!(!signer.is_encrypted());
    for _ in 0..2 {
        signer.sign(b"test input", SystemTime::now()).unwrap();
    }
    let never = || panic!("password must not be requested");
    authentication::load_openpgp_signer(
        &directory.join("ed25519.secret.pgp"),
        None,
        None,
        MATERIAL_LIMITS,
        never,
    )
    .unwrap();
    assert!(
        authentication::load_openpgp_signer(
            &directory.join("encrypted.secret.pgp"),
            Some("short id"),
            None,
            MATERIAL_LIMITS,
            never
        )
        .is_err()
    );
    assert!(
        authentication::load_openpgp_signer(
            &directory.join("encrypted.secret.pgp"),
            None,
            None,
            Limits {
                max_bytes: 16,
                ..MATERIAL_LIMITS
            },
            never
        )
        .is_err()
    );
}

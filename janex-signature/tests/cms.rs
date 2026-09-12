// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Independent CMS fixtures, protected attributes, signature algorithms, and explicit certificate pins.

use ::cms::{content_info::ContentInfo, signed_data::SignedData};
use der::{
    Any, Decode, Encode,
    asn1::{ObjectIdentifier, SetOfVec},
};
use janex_signature::{
    ErrorKind, Limits,
    cms::{self, Algorithm, CmsSignature, CmsSigner, SignerCertificate},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Exact detached bytes shared by the independently generated fixtures.
const INPUT: &[u8] = include_bytes!("fixtures/cms/input.bin");

/// Supplies a fixed verification instant within all fixture certificate validity periods.
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_893_456_000)
}

/// Loads a fixture certificate without importing any system trust state.
fn certificate(name: &str) -> SignerCertificate {
    let bytes: &[u8] = match name {
        "rsa256" => include_bytes!("fixtures/cms/rsa256.cert.pem"),
        "rsa512" => include_bytes!("fixtures/cms/rsa512.cert.pem"),
        "p256" => include_bytes!("fixtures/cms/p256.cert.pem"),
        "p384" => include_bytes!("fixtures/cms/p384.cert.pem"),
        _ => panic!("unknown fixture"),
    };
    SignerCertificate::decode(bytes, Limits::default()).unwrap()
}

/// Re-encodes a mutated CMS fixture with canonical DER framing.
fn mutate(bytes: &[u8], change: impl FnOnce(&mut SignedData)) -> Vec<u8> {
    let mut info = ContentInfo::from_der(bytes).unwrap();
    let mut data = info.content.decode_as::<SignedData>().unwrap();
    change(&mut data);
    info.content = Any::encode_from(&data).unwrap();
    info.to_der().unwrap()
}

#[test]
fn independent_openssl_signatures_verify_for_every_supported_combination() {
    for (name, algorithm, bytes) in [
        (
            "rsa256",
            Algorithm::RsaSha256,
            include_bytes!("fixtures/cms/rsa256.cms.der").as_slice(),
        ),
        (
            "rsa512",
            Algorithm::RsaSha512,
            include_bytes!("fixtures/cms/rsa512.cms.der").as_slice(),
        ),
        (
            "p256",
            Algorithm::EcdsaP256Sha256,
            include_bytes!("fixtures/cms/p256.cms.der").as_slice(),
        ),
        (
            "p384",
            Algorithm::EcdsaP384Sha384,
            include_bytes!("fixtures/cms/p384.cms.der").as_slice(),
        ),
    ] {
        let certificate = certificate(name);
        let signature = CmsSignature::decode(bytes, Limits::default()).unwrap();
        assert_eq!(
            signature.verify_signature(INPUT, &certificate).unwrap(),
            algorithm
        );
        let verified = signature
            .verify_pinned(INPUT, std::slice::from_ref(&certificate), now())
            .unwrap();
        assert_eq!(verified[0].fingerprint, *certificate.fingerprint());
        assert_eq!(verified[0].algorithm, algorithm);
        assert_eq!(
            signature
                .verify_pinned(b"altered content", &[certificate], now())
                .unwrap_err()
                .kind(),
            ErrorKind::Verification
        );
        for end in 0..bytes.len() {
            assert!(
                CmsSignature::decode(&bytes[..end], Limits::default()).is_err(),
                "{name}: {end}"
            );
        }
        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert!(CmsSignature::decode(&trailing, Limits::default()).is_err());
    }
}

#[test]
fn signing_and_encrypted_pkcs8_round_trip_without_exposing_secret_material() {
    for (name, algorithm, key, encrypted) in [
        (
            "rsa256",
            Algorithm::RsaSha256,
            include_bytes!("fixtures/cms/rsa256.key.pem").as_slice(),
            include_bytes!("fixtures/cms/rsa256.encrypted.pem").as_slice(),
        ),
        (
            "rsa512",
            Algorithm::RsaSha512,
            include_bytes!("fixtures/cms/rsa512.key.pem").as_slice(),
            include_bytes!("fixtures/cms/rsa512.encrypted.pem").as_slice(),
        ),
        (
            "p256",
            Algorithm::EcdsaP256Sha256,
            include_bytes!("fixtures/cms/p256.key.pem").as_slice(),
            include_bytes!("fixtures/cms/p256.encrypted.pem").as_slice(),
        ),
        (
            "p384",
            Algorithm::EcdsaP384Sha384,
            include_bytes!("fixtures/cms/p384.key.pem").as_slice(),
            include_bytes!("fixtures/cms/p384.encrypted.pem").as_slice(),
        ),
    ] {
        let certificate = certificate(name);
        let signer =
            CmsSigner::from_pkcs8(certificate.clone(), key, None, algorithm, Limits::default())
                .unwrap();
        let bytes = cms::sign(INPUT, &[&signer], now()).unwrap();
        CmsSignature::decode(&bytes, Limits::default())
            .unwrap()
            .verify_pinned(INPUT, std::slice::from_ref(&certificate), now())
            .unwrap();
        assert!(!format!("{signer:?}").contains("PRIVATE KEY"));
        assert!(
            CmsSigner::from_pkcs8(
                certificate.clone(),
                encrypted,
                None,
                algorithm,
                Limits::default()
            )
            .is_err()
        );
        assert!(
            CmsSigner::from_pkcs8(
                certificate.clone(),
                encrypted,
                Some(b"wrong"),
                algorithm,
                Limits::default()
            )
            .is_err()
        );
        let encrypted = CmsSigner::from_pkcs8(
            certificate.clone(),
            encrypted,
            Some(b"public-fixture-password"),
            algorithm,
            Limits::default(),
        )
        .unwrap();
        let bytes = cms::sign(INPUT, &[&encrypted], now()).unwrap();
        CmsSignature::decode(&bytes, Limits::default())
            .unwrap()
            .verify_pinned(INPUT, &[certificate], now())
            .unwrap();
    }
    assert!(
        CmsSigner::from_pkcs8(
            certificate("rsa512"),
            include_bytes!("fixtures/cms/rsa256.key.pem"),
            None,
            Algorithm::RsaSha256,
            Limits::default()
        )
        .is_err()
    );
}

#[test]
fn all_required_signers_must_be_present_and_valid_at_the_policy_time() {
    let a = certificate("rsa256");
    let b = certificate("p256");
    let signer_a = CmsSigner::from_pkcs8(
        a.clone(),
        include_bytes!("fixtures/cms/rsa256.key.pem"),
        None,
        Algorithm::RsaSha256,
        Limits::default(),
    )
    .unwrap();
    let signer_b = CmsSigner::from_pkcs8(
        b.clone(),
        include_bytes!("fixtures/cms/p256.key.pem"),
        None,
        Algorithm::EcdsaP256Sha256,
        Limits::default(),
    )
    .unwrap();
    let bytes = cms::sign(INPUT, &[&signer_a, &signer_b], now()).unwrap();
    let signature = CmsSignature::decode(&bytes, Limits::default()).unwrap();
    assert_eq!(
        signature
            .verify_pinned(INPUT, &[a.clone(), b.clone()], now())
            .unwrap()
            .len(),
        2
    );
    assert!(signature.verify_pinned(INPUT, &[], now()).is_err());
    assert!(
        signature
            .verify_pinned(INPUT, &[a.clone(), a.clone()], now())
            .is_err()
    );
    assert!(
        signature
            .verify_pinned(INPUT, &[a.clone(), certificate("rsa512")], now())
            .is_err()
    );
    assert_eq!(
        signature
            .verify_pinned(INPUT, std::slice::from_ref(&a), UNIX_EPOCH)
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    assert!(
        signature
            .verify_pinned(
                INPUT,
                &[a],
                UNIX_EPOCH + Duration::from_secs(10_000_000_000)
            )
            .is_err()
    );
    let bytes = cms::sign(INPUT, &[&signer_b], now()).unwrap();
    assert!(
        CmsSignature::decode(&bytes, Limits::default())
            .unwrap()
            .verify_pinned(INPUT, &[certificate("rsa256"), b], now())
            .is_err()
    );
}

#[test]
fn malformed_protected_attributes_and_algorithm_substitution_are_rejected() {
    let certificate = certificate("rsa256");
    let missing = CmsSignature::decode(
        include_bytes!("fixtures/cms/missing-protection.cms.der"),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        missing
            .verify_signature(INPUT, &certificate)
            .unwrap_err()
            .kind(),
        ErrorKind::Invalid
    );
    let original = include_bytes!("fixtures/cms/rsa256.cms.der");
    for field in [
        "digest",
        "signature-algorithm",
        "signature",
        "duplicate-digest",
        "no-attributes",
    ] {
        let bytes = mutate(original, |data| {
            let mut signer = data.signer_infos.0.get(0).unwrap().clone();
            match field {
                "digest" => {
                    signer.digest_alg.oid = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3")
                }
                "signature-algorithm" => {
                    signer.signature_algorithm.oid =
                        ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13")
                }
                "signature" => {
                    signer.signature = der::asn1::OctetString::new(vec![0; 256]).unwrap()
                }
                "duplicate-digest" => {
                    let attributes = signer.signed_attrs.as_mut().unwrap();
                    let mut duplicate = attributes
                        .iter()
                        .find(|attribute| {
                            attribute.oid == ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4")
                        })
                        .unwrap()
                        .clone();
                    duplicate.values = SetOfVec::try_from(vec![
                        Any::encode_from(&der::asn1::OctetString::new(vec![0; 32]).unwrap())
                            .unwrap(),
                    ])
                    .unwrap();
                    attributes.insert(duplicate).unwrap();
                }
                "no-attributes" => signer.signed_attrs = None,
                _ => unreachable!(),
            }
            data.signer_infos.0 = SetOfVec::try_from(vec![signer]).unwrap();
        });
        assert!(
            CmsSignature::decode(&bytes, Limits::default())
                .unwrap()
                .verify_signature(INPUT, &certificate)
                .is_err(),
            "{field}"
        );
    }
    let limits = Limits {
        max_elements: 3,
        ..Limits::default()
    };
    assert_eq!(
        CmsSignature::decode(original, limits).unwrap_err().kind(),
        ErrorKind::Limit
    );
    let bytes = mutate(original, |data| {
        data.encap_content_info.econtent =
            Some(Any::encode_from(&der::asn1::OctetString::new(INPUT).unwrap()).unwrap())
    });
    assert!(CmsSignature::decode(&bytes, Limits::default()).is_err());
}

#[test]
fn offline_crls_require_a_valid_issuer_and_reject_revoked_or_stale_material() {
    let limits = Limits::default();
    let issued =
        SignerCertificate::decode(include_bytes!("fixtures/cms/issued.cert.pem"), limits).unwrap();
    let issuer =
        SignerCertificate::decode(include_bytes!("fixtures/cms/ca.cert.pem"), limits).unwrap();
    let signer = CmsSigner::from_pkcs8(
        issued.clone(),
        include_bytes!("fixtures/cms/rsa256.key.pem"),
        None,
        Algorithm::RsaSha256,
        limits,
    )
    .unwrap();
    let bytes = cms::sign(INPUT, &[&signer], now()).unwrap();
    let signature = CmsSignature::decode(&bytes, limits).unwrap();
    let good =
        cms::RevocationList::decode(include_bytes!("fixtures/cms/good.crl.pem"), limits).unwrap();
    let result = signature
        .authenticate(
            INPUT,
            std::slice::from_ref(&issued),
            std::slice::from_ref(&issuer),
            std::slice::from_ref(&good),
            now(),
        )
        .unwrap();
    assert_eq!(result[0].revocation_lists_checked, 1);
    assert!(
        signature
            .authenticate(INPUT, std::slice::from_ref(&issued), &[], &[good], now())
            .is_err()
    );
    for bytes in [
        include_bytes!("fixtures/cms/stale.crl.pem").as_slice(),
        include_bytes!("fixtures/cms/revoked.crl.pem").as_slice(),
    ] {
        let list = cms::RevocationList::decode(bytes, limits).unwrap();
        assert_eq!(
            signature
                .authenticate(
                    INPUT,
                    std::slice::from_ref(&issued),
                    std::slice::from_ref(&issuer),
                    &[list],
                    now()
                )
                .unwrap_err()
                .kind(),
            ErrorKind::Trust
        );
    }
    let (_, revoked) =
        der::pem::decode_vec(include_bytes!("fixtures/cms/revoked.crl.pem")).unwrap();
    let embedded = mutate(&bytes, |data| {
        let issuer = x509_cert::Certificate::from_der(&issuer.to_der().unwrap()).unwrap();
        data.certificates
            .as_mut()
            .unwrap()
            .0
            .insert(::cms::cert::CertificateChoices::Certificate(issuer))
            .unwrap();
        data.crls = Some(::cms::revocation::RevocationInfoChoices(
            SetOfVec::try_from(vec![::cms::revocation::RevocationInfoChoice::Crl(
                x509_cert::crl::CertificateList::from_der(&revoked).unwrap(),
            )])
            .unwrap(),
        ));
    });
    assert!(
        CmsSignature::decode(&embedded, limits)
            .unwrap()
            .authenticate(INPUT, &[issued], &[], &[], now())
            .unwrap_err()
            .to_string()
            .contains("revoked")
    );
    let wrong_usage =
        SignerCertificate::decode(include_bytes!("fixtures/cms/wrong-usage.cert.pem"), limits)
            .unwrap();
    assert_eq!(
        wrong_usage.validate(now()).unwrap_err().kind(),
        ErrorKind::Trust
    );
}

#[test]
fn excessive_password_derivation_is_rejected_before_decryption() {
    let (_, document) = pkcs8::SecretDocument::from_pem(
        std::str::from_utf8(include_bytes!("fixtures/cms/rsa256.encrypted.pem")).unwrap(),
    )
    .unwrap();
    let mut encrypted = pkcs8::EncryptedPrivateKeyInfo::from_der(document.as_bytes()).unwrap();
    let mut parameters = encrypted.encryption_algorithm.pbes2().unwrap().clone();
    if let pkcs8::pkcs5::pbes2::Kdf::Pbkdf2(params) = &mut parameters.kdf {
        params.iteration_count = u32::MAX;
    }
    encrypted.encryption_algorithm = parameters.into();
    let bytes = encrypted.to_der().unwrap();
    assert_eq!(
        CmsSigner::key_is_encrypted(&bytes, Limits::default())
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
    assert_eq!(
        CmsSigner::from_pkcs8(
            certificate("rsa256"),
            &bytes,
            Some(b"public-fixture-password"),
            Algorithm::RsaSha256,
            Limits::default()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Limit
    );
}

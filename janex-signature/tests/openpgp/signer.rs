// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Secret-key selection, unlocking, and certificate policy for generated document signatures.

use super::*;
use janex_signature::openpgp::{KeyCertificate, OpenPgpSigner};

/// Loads a test certificate with default algorithm selection at its creation time.
fn signer(name: &str) -> OpenPgpSigner {
    OpenPgpSigner::decode(
        &fixture(name, "secret"),
        None,
        None,
        Limits::default(),
        instant(0),
    )
    .unwrap()
}

#[test]
fn selects_certified_primary_keys_and_signing_subkeys() {
    for name in ["rsa256", "rsa512", "p256", "p384", "ed25519", "subkey"] {
        let signer = signer(name);
        assert!(!signer.is_encrypted());
        let expected = decode(&fixture(name, "signature"))
            .unwrap()
            .issuer()
            .clone();
        assert_eq!(signer.fingerprint(), expected);
        assert_eq!(
            signer.certificate().fingerprint(),
            public(name).fingerprint()
        );
        let signer = signer.unlock(b"ignored for unencrypted keys").unwrap();
        let bytes = signer.sign(INPUT, instant(1)).unwrap();
        let certificate =
            KeyCertificate::decode(&fixture(name, "public"), Limits::default()).unwrap();
        certificate
            .authenticate(INPUT, &decode(&bytes).unwrap(), instant(1))
            .unwrap();
        let selected = OpenPgpSigner::decode(
            &fixture(name, "secret"),
            Some(&expected.to_string().to_lowercase()),
            None,
            Limits::default(),
            instant(1),
        )
        .unwrap();
        assert_eq!(selected.fingerprint(), expected);
    }
}

#[test]
fn rejects_ineligible_keys_and_invalid_selection_before_unlocking() {
    for name in ["subkey", "expired"] {
        assert!(
            OpenPgpSigner::decode(
                &fixture(name, "secret"),
                None,
                None,
                Limits::default(),
                instant(86_400)
            )
            .is_err()
        );
        assert!(signer(name).sign(INPUT, instant(86_400)).is_err());
    }
    let primary = public("subkey").fingerprint().to_string();
    assert!(
        OpenPgpSigner::decode(
            &fixture("subkey", "secret"),
            Some(&primary),
            None,
            Limits::default(),
            instant(0)
        )
        .is_err()
    );
    for fingerprint in [
        "",
        "01234567",
        "z123456789012345678901234567890123456789",
        "0123456789012345678901234567890123456789",
    ] {
        assert!(
            OpenPgpSigner::decode(
                &fixture("ed25519", "secret"),
                Some(fingerprint),
                None,
                Limits::default(),
                instant(0)
            )
            .is_err()
        );
    }
    assert!(
        OpenPgpSigner::decode(
            &fixture("ed25519", "public"),
            None,
            None,
            Limits::default(),
            instant(0)
        )
        .is_err()
    );
    assert!(
        OpenPgpSigner::decode(
            &fixture("v6", "secret"),
            None,
            None,
            Limits::default(),
            instant(0)
        )
        .is_err()
    );
    assert!(
        OpenPgpSigner::decode(
            &fixture("ed25519", "secret"),
            None,
            Some(Algorithm::RsaSha256),
            Limits::default(),
            instant(0)
        )
        .is_err()
    );
    let signer = OpenPgpSigner::decode(
        &fixture("ed25519", "secret"),
        None,
        Some(Algorithm::Ed25519Sha512),
        Limits::default(),
        instant(0),
    )
    .unwrap();
    assert_eq!(signer.algorithm(), Algorithm::Ed25519Sha512);
    assert_eq!(
        decode(&signer.sign(INPUT, instant(0)).unwrap())
            .unwrap()
            .verify_signature(INPUT, &public("ed25519"))
            .unwrap(),
        Algorithm::Ed25519Sha512
    );
}

/// Replaces a fixture's secret primary packet while retaining its original public certifications.
fn replace_primary(name: &str, key: &SecretKey) -> Vec<u8> {
    let original = fixture(name, "secret");
    let mut rest = original.as_slice();
    let header = PacketHeader::try_from_reader(&mut rest).unwrap();
    let PacketLength::Fixed(length) = header.packet_length() else {
        panic!("fixture length")
    };
    let skip = original.len() - rest.len() + length as usize;
    let body = key.to_bytes().unwrap();
    let mut bytes = PacketHeader::new_fixed(Tag::SecretKey, body.len() as u32)
        .to_bytes()
        .unwrap();
    bytes.extend(body);
    bytes.extend_from_slice(&original[skip..]);
    bytes
}

#[test]
fn unlocks_once_and_rejects_wrong_password_or_mismatched_private_material() {
    let mut key = secret("ed25519");
    let password = Password::from(&b"public test password"[..]);
    key.set_password(rand::rngs::OsRng, &password).unwrap();
    let bytes = replace_primary("ed25519", &key);
    let load = || OpenPgpSigner::decode(&bytes, None, None, Limits::default(), instant(0)).unwrap();
    let signer = load();
    assert!(signer.is_encrypted());
    assert!(signer.sign(INPUT, instant(0)).is_err());
    assert!(load().unlock(b"wrong password").is_err());
    let signer = signer.unlock(b"public test password").unwrap();
    assert!(!signer.is_encrypted());
    for seconds in [0, 1, 2] {
        signer
            .certificate()
            .authenticate(
                INPUT,
                &decode(&signer.sign(INPUT, instant(seconds)).unwrap()).unwrap(),
                instant(seconds),
            )
            .unwrap();
    }
    let debug = format!("{signer:?}");
    assert!(!debug.contains("SecretParams"));
    assert!(!debug.contains("public test password"));

    // The second test key supplies an unrelated Ed25519 secret scalar with the original public key.
    let wrong =
        SecretKey::new(public("ed25519"), secret("expired").secret_params().clone()).unwrap();
    let bytes = replace_primary("ed25519", &wrong);
    let signer = OpenPgpSigner::decode(&bytes, None, None, Limits::default(), instant(0)).unwrap();
    assert!(signer.unlock(b"").is_err());
}

#[test]
fn secret_packet_bounds_and_trailing_data_are_checked() {
    let bytes = fixture("ed25519", "secret");
    for length in 0..bytes.len() {
        assert!(
            OpenPgpSigner::decode(&bytes[..length], None, None, Limits::default(), instant(0))
                .is_err()
        );
    }
    let limits = Limits {
        max_bytes: bytes.len() as u64 - 1,
        ..Limits::default()
    };
    assert_eq!(
        OpenPgpSigner::decode(&bytes, None, None, limits, instant(0))
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
    let limits = Limits {
        max_elements: 1,
        ..Limits::default()
    };
    assert_eq!(
        OpenPgpSigner::decode(&bytes, None, None, limits, instant(0))
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(OpenPgpSigner::decode(&trailing, None, None, Limits::default(), instant(0)).is_err());
    trailing.extend_from_slice(&bytes);
    assert!(OpenPgpSigner::decode(&trailing, None, None, Limits::default(), instant(0)).is_err());
}

#[test]
fn malformed_rsa_factors_return_an_error_instead_of_panicking() {
    // n = 9, e = 3, d = 1, p = q = 3 passes the library's basic RSA consistency
    // checks, but the coefficient p^-1 mod q does not exist and cannot be serialized.
    let mut body = vec![4];
    body.extend_from_slice(&CREATED.to_be_bytes());
    body.extend_from_slice(&[1, 0, 4, 9, 0, 2, 3, 0]);
    let secret = [0, 1, 1, 0, 2, 3, 0, 2, 3, 0, 0];
    body.extend_from_slice(&secret);
    let checksum: u16 = secret.iter().map(|byte| u16::from(*byte)).sum();
    body.extend_from_slice(&checksum.to_be_bytes());
    let mut bytes = PacketHeader::new_fixed(Tag::SecretKey, body.len() as u32)
        .to_bytes()
        .unwrap();
    bytes.extend(body);
    assert!(OpenPgpSigner::decode(&bytes, None, None, Limits::default(), instant(0)).is_err());
}

#[test]
fn independent_gnupg_encrypted_subkeys_and_private_armor_unlock() {
    let binary = fixture("encrypted", "secret");
    let armored = include_bytes!("../fixtures/openpgp/encrypted.secret.asc");
    for bytes in [binary.as_slice(), armored.as_slice()] {
        let signer =
            OpenPgpSigner::decode(bytes, None, None, Limits::default(), instant(0)).unwrap();
        assert!(signer.is_encrypted());
        assert_ne!(signer.fingerprint(), signer.certificate().fingerprint());
        assert_eq!(
            &signer.fingerprint(),
            decode(&fixture("encrypted", "signature")).unwrap().issuer()
        );
        let signer = signer.unlock(b"public-fixture-password").unwrap();
        let signed = signer.sign(INPUT, instant(1)).unwrap();
        signer
            .certificate()
            .authenticate(INPUT, &decode(&signed).unwrap(), instant(1))
            .unwrap();
    }
    for bytes in [
        [armored.as_slice(), armored.as_slice()].concat(),
        [armored.as_slice(), b"suffix"].concat(),
        [b"prefix", armored.as_slice()].concat(),
        String::from_utf8(armored.to_vec())
            .unwrap()
            .replace("PRIVATE KEY", "PUBLIC KEY")
            .into_bytes(),
    ] {
        assert!(OpenPgpSigner::decode(&bytes, None, None, Limits::default(), instant(0)).is_err());
    }
    let limits = Limits {
        max_bytes: armored.len() as u64 - 1,
        ..Limits::default()
    };
    assert_eq!(
        OpenPgpSigner::decode(armored, None, None, limits, instant(0))
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
}

/// Adds a direct self-signature to a version-6 private-key packet.
fn certified_v6(key: &SecretKey) -> Vec<u8> {
    let mut config = SignatureConfig::v6(
        rand::rngs::OsRng,
        SignatureType::Key,
        key.algorithm(),
        HashAlgorithm::Sha512,
    )
    .unwrap();
    config.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::SignatureCreationTime(Timestamp::from_secs(
            CREATED,
        )))
        .unwrap(),
        Subpacket::regular(SubpacketData::IssuerFingerprint(key.fingerprint())).unwrap(),
    ];
    let signature = config
        .sign_key(&secret("v6"), &Password::empty(), key.public_key())
        .unwrap();
    let mut bytes = replace_primary("v6", key);
    bytes.extend(packet(&signature.to_bytes().unwrap()));
    bytes
}

#[test]
fn version_six_aead_unlocks_and_derivation_costs_are_bounded_before_decryption() {
    use pgp::{
        crypto::{aead::AeadAlgorithm, sym::SymmetricKeyAlgorithm},
        types::{EncryptedSecretParams, S2kParams, SecretParams, StringToKey},
    };
    let password = Password::from(&b"public test password"[..]);
    let mut key = secret("v6");
    let protection = S2kParams::Aead {
        sym_alg: SymmetricKeyAlgorithm::AES256,
        aead_mode: AeadAlgorithm::Ocb,
        s2k: StringToKey::Argon2 {
            salt: [1; 16],
            t: 1,
            p: 1,
            m_enc: 10,
        },
        nonce: vec![2; 15].into(),
    };
    key.set_password_with_s2k(&password, protection).unwrap();
    let bytes = certified_v6(&key);
    let signer = OpenPgpSigner::decode(&bytes, None, None, Limits::default(), instant(0)).unwrap();
    assert!(signer.is_encrypted());
    let signer = signer.unlock(b"public test password").unwrap();
    let signed = signer.sign(INPUT, instant(1)).unwrap();
    signer
        .certificate()
        .authenticate(INPUT, &decode(&signed).unwrap(), instant(1))
        .unwrap();

    for (t, p, m_enc, kind) in [
        (11, 1, 10, ErrorKind::Limit),
        (1, 17, 10, ErrorKind::Limit),
        (1, 1, 19, ErrorKind::Limit),
        (5, 1, 18, ErrorKind::Limit),
        (1, 1, 255, ErrorKind::Limit),
        (0, 1, 10, ErrorKind::Invalid),
        (1, 0, 10, ErrorKind::Invalid),
        (1, 2, 3, ErrorKind::Invalid),
    ] {
        // Encode adversarial parameters without actually allocating or deriving their claimed cost.
        let SecretParams::Encrypted(encrypted) = key.secret_params() else {
            panic!("encrypted fixture")
        };
        let params = S2kParams::Aead {
            sym_alg: SymmetricKeyAlgorithm::AES256,
            aead_mode: AeadAlgorithm::Ocb,
            s2k: StringToKey::Argon2 {
                salt: [1; 16],
                t,
                p,
                m_enc,
            },
            nonce: vec![2; 15].into(),
        };
        let modified = SecretKey::new(
            public("v6"),
            SecretParams::Encrypted(EncryptedSecretParams::new(
                encrypted.data().to_vec().into(),
                params,
            )),
        )
        .unwrap();
        let bytes = certified_v6(&modified);
        assert_eq!(
            OpenPgpSigner::decode(&bytes, None, None, Limits::default(), instant(0))
                .unwrap_err()
                .kind(),
            kind,
            "t={t}, p={p}, m={m_enc}"
        );
    }
}

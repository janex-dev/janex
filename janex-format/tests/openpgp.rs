// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Independent OpenPGP signatures, exact-byte verification, profile rejection, and parsing limits.

#[path = "openpgp/signer.rs"]
mod signer;
#[path = "openpgp/trust.rs"]
mod trust;

use janex_format::{
    ErrorKind,
    binary::Limits,
    signature::openpgp::{self, Algorithm, OpenPgpSignature},
};
use pgp::{
    crypto::{hash::HashAlgorithm, public_key::PublicKeyAlgorithm},
    packet::{
        PacketHeader, PublicKey, SecretKey, Signature, SignatureConfig, SignatureType, Subpacket,
        SubpacketData,
    },
    ser::Serialize,
    types::{Duration as PgpDuration, KeyDetails, KeyId, PacketLength, Password, Tag, Timestamp},
};
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Exact detached document, including a NUL and mixed line endings.
const INPUT: &[u8] = include_bytes!("fixtures/openpgp/input.bin");
/// Fixed creation instant shared by independent fixtures.
const CREATED: u32 = 1_767_225_600;

/// Loads a public test artifact without touching a user keyring.
fn fixture(name: &str, suffix: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/openpgp")
            .join(format!("{name}.{suffix}.pgp")),
    )
    .unwrap()
}

/// Extracts the first definite-length packet from trusted fixture material.
fn first_packet(bytes: &[u8]) -> (PacketHeader, &[u8]) {
    let mut bytes = bytes;
    let header = PacketHeader::try_from_reader(&mut bytes).unwrap();
    let PacketLength::Fixed(length) = header.packet_length() else {
        panic!("fixture packet length")
    };
    (header, &bytes[..length as usize])
}

/// Reads the public primary-key packet; key certifications are outside mechanism tests.
fn public(name: &str) -> PublicKey {
    let bytes = fixture(name, "public");
    let (header, body) = first_packet(&bytes);
    PublicKey::try_from_reader(header, body).unwrap()
}

/// Reads an unencrypted public-test secret primary-key packet.
fn secret(name: &str) -> SecretKey {
    let bytes = fixture(name, "secret");
    let (header, body) = first_packet(&bytes);
    SecretKey::try_from_reader(header, body).unwrap()
}

/// Uses a definite packet length for a modified signature body.
fn packet(body: &[u8]) -> Vec<u8> {
    let mut result = PacketHeader::new_fixed(Tag::Signature, body.len() as u32)
        .to_bytes()
        .unwrap();
    result.extend_from_slice(body);
    result
}

/// Creates an independently configurable, cryptographically valid v4 signature for profile tests.
fn configured(change: impl FnOnce(&mut SignatureConfig)) -> Vec<u8> {
    let key = secret("ed25519");
    let mut config = SignatureConfig::v4(
        SignatureType::Binary,
        key.algorithm(),
        HashAlgorithm::Sha256,
    );
    config.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::SignatureCreationTime(Timestamp::from_secs(
            CREATED,
        )))
        .unwrap(),
        Subpacket::regular(SubpacketData::IssuerFingerprint(key.fingerprint())).unwrap(),
    ];
    change(&mut config);
    let signature = config.sign(&key, &Password::empty(), INPUT).unwrap();
    packet(&signature.to_bytes().unwrap())
}

/// Decodes under the normal format limits.
fn decode(bytes: &[u8]) -> janex_format::Result<OpenPgpSignature> {
    OpenPgpSignature::decode(bytes, Limits::default())
}

/// Returns a timestamp offset from the fixture's creation time.
fn instant(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(u64::from(CREATED) + seconds)
}

#[test]
fn independent_gnupg_and_openssl_signatures_verify() {
    for (name, algorithm) in [
        ("rsa256", Algorithm::RsaSha256),
        ("rsa512", Algorithm::RsaSha512),
        ("p256", Algorithm::EcdsaP256Sha256),
        ("p384", Algorithm::EcdsaP384Sha384),
        ("ed25519", Algorithm::Ed25519Sha256),
        ("v6", Algorithm::Ed25519Sha512),
    ] {
        let key = public(name);
        let bytes = fixture(name, "signature");
        let signature = decode(&bytes).unwrap();
        assert_eq!(
            signature.verify_signature(INPUT, &key).unwrap(),
            algorithm,
            "{name}"
        );
        assert_eq!(signature.issuer(), &key.fingerprint());
        assert_eq!(signature.created(), instant(0));
        signature.validate_time(instant(1)).unwrap();
        assert_eq!(
            signature
                .verify_signature(b"changed", &key)
                .unwrap_err()
                .kind(),
            ErrorKind::Verification
        );
        let wrong = public(if name == "rsa256" { "rsa512" } else { "rsa256" });
        assert_eq!(
            signature
                .verify_signature(INPUT, &wrong)
                .unwrap_err()
                .kind(),
            ErrorKind::Verification
        );
        for length in 0..bytes.len() {
            assert!(decode(&bytes[..length]).is_err(), "{name} prefix {length}");
        }
        for suffix in [vec![0], bytes.clone(), vec![0xca, 3, b'P', b'G', b'P']] {
            let mut trailing = bytes.clone();
            trailing.extend(suffix);
            assert!(decode(&trailing).is_err());
        }
        let mut body = first_packet(&bytes).1.to_vec();
        *body.last_mut().unwrap() ^= 1;
        assert_eq!(
            decode(&packet(&body))
                .unwrap()
                .verify_signature(INPUT, &key)
                .unwrap_err()
                .kind(),
            ErrorKind::Verification
        );
    }
}

#[test]
fn generated_signatures_round_trip_for_supported_keys_and_versions() {
    for (name, algorithm) in [
        ("rsa256", Algorithm::RsaSha256),
        ("rsa512", Algorithm::RsaSha512),
        ("p256", Algorithm::EcdsaP256Sha256),
        ("p384", Algorithm::EcdsaP384Sha384),
        ("ed25519", Algorithm::Ed25519Sha256),
        ("ed25519", Algorithm::Ed25519Sha512),
        ("v6", Algorithm::Ed25519Sha256),
        ("v6", Algorithm::Ed25519Sha512),
    ] {
        let bytes = openpgp::sign(
            INPUT,
            &secret(name),
            &Password::empty(),
            algorithm,
            instant(1),
        )
        .unwrap();
        let signature = decode(&bytes).unwrap();
        assert_eq!(
            signature.verify_signature(INPUT, &public(name)).unwrap(),
            algorithm
        );
        assert_eq!(signature.created(), instant(1));
        let (header, body) = first_packet(&bytes);
        Signature::try_from_reader(header, body)
            .unwrap()
            .verify(&public(name), INPUT)
            .unwrap();
    }
    assert!(
        openpgp::sign(
            INPUT,
            &secret("p256"),
            &Password::empty(),
            Algorithm::RsaSha256,
            instant(1)
        )
        .is_err()
    );
    assert!(
        openpgp::sign(
            INPUT,
            &secret("p256"),
            &Password::empty(),
            Algorithm::EcdsaP256Sha256,
            UNIX_EPOCH
        )
        .is_err()
    );
}

#[test]
fn profile_requires_unique_hashed_identity_and_time_and_binary_document() {
    for field in 0..2 {
        let missing = configured(|config| {
            config
                .unhashed_subpackets
                .push(config.hashed_subpackets.remove(field));
        });
        assert_eq!(decode(&missing).unwrap_err().kind(), ErrorKind::Invalid);
        let duplicate = configured(|config| {
            config
                .hashed_subpackets
                .push(config.hashed_subpackets[field].clone())
        });
        assert_eq!(decode(&duplicate).unwrap_err().kind(), ErrorKind::Invalid);
    }
    let text = configured(|config| config.typ = SignatureType::Text);
    assert_eq!(decode(&text).unwrap_err().kind(), ErrorKind::Invalid);
    for version in [0, 2, 3, 5, 7, 255] {
        let mut body = first_packet(&fixture("ed25519", "signature")).1.to_vec();
        body[0] = version;
        assert_eq!(
            decode(&packet(&body)).unwrap_err().kind(),
            ErrorKind::Invalid
        );
    }
    let wrong_issuer = configured(|config| {
        config.hashed_subpackets[1] = Subpacket::regular(SubpacketData::IssuerFingerprint(
            public("rsa256").fingerprint(),
        ))
        .unwrap()
    });
    assert_eq!(
        decode(&wrong_issuer)
            .unwrap()
            .verify_signature(INPUT, &public("ed25519"))
            .unwrap_err()
            .kind(),
        ErrorKind::Verification
    );
    assert!(decode(b"-----BEGIN PGP SIGNATURE-----\n").is_err());
    let mut wrong_packet = fixture("ed25519", "signature");
    wrong_packet[0] = 0xcb;
    assert!(decode(&wrong_packet).is_err());
}

#[test]
fn unhashed_identity_is_advisory_and_unknown_critical_subpackets_are_rejected() {
    let advisory = configured(|config| {
        config.unhashed_subpackets = vec![
            Subpacket::regular(SubpacketData::IssuerKeyId(KeyId::from([0; 8]))).unwrap(),
            Subpacket::regular(SubpacketData::IssuerFingerprint(
                public("rsa256").fingerprint(),
            ))
            .unwrap(),
        ];
    });
    decode(&advisory)
        .unwrap()
        .verify_signature(INPUT, &public("ed25519"))
        .unwrap();
    for hashed in [true, false] {
        for tag in [100, 110] {
            let noncritical = configured(|config| {
                let area = if hashed {
                    &mut config.hashed_subpackets
                } else {
                    &mut config.unhashed_subpackets
                };
                area.push(
                    Subpacket::regular(SubpacketData::Experimental(tag, vec![1, 2].into()))
                        .unwrap(),
                );
            });
            decode(&noncritical)
                .unwrap()
                .verify_signature(INPUT, &public("ed25519"))
                .unwrap();
            // Mark the existing subpacket critical without asking rPGP to sign unknown semantics.
            let mut body = first_packet(&noncritical).1.to_vec();
            let index = body
                .windows(4)
                .position(|bytes| bytes == [3, tag, 1, 2])
                .unwrap();
            body[index + 1] |= 0x80;
            assert_eq!(
                decode(&packet(&body)).unwrap_err().kind(),
                ErrorKind::Invalid
            );
        }
    }
}

#[test]
fn signature_lifetime_has_exact_boundaries_and_ignores_unhashed_expiration() {
    let bytes = configured(|config| {
        config.hashed_subpackets.push(
            Subpacket::regular(SubpacketData::SignatureExpirationTime(
                PgpDuration::from_secs(60),
            ))
            .unwrap(),
        )
    });
    let signature = decode(&bytes).unwrap();
    signature
        .verify_signature(INPUT, &public("ed25519"))
        .unwrap();
    assert_eq!(
        signature
            .validate_time(instant(0) - Duration::from_secs(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    signature.validate_time(instant(0)).unwrap();
    signature.validate_time(instant(59)).unwrap();
    assert_eq!(
        signature.validate_time(instant(60)).unwrap_err().kind(),
        ErrorKind::Trust
    );
    let duplicate = configured(|config| {
        for _ in 0..2 {
            config.hashed_subpackets.push(
                Subpacket::regular(SubpacketData::SignatureExpirationTime(
                    PgpDuration::from_secs(60),
                ))
                .unwrap(),
            );
        }
    });
    assert!(decode(&duplicate).is_err());
    let unhashed = configured(|config| {
        config.unhashed_subpackets.push(
            Subpacket::regular(SubpacketData::SignatureExpirationTime(
                PgpDuration::from_secs(1),
            ))
            .unwrap(),
        )
    });
    decode(&unhashed)
        .unwrap()
        .validate_time(instant(60))
        .unwrap();
}

#[test]
fn unsupported_algorithms_and_resource_limits_fail_before_verification() {
    let bytes = fixture("ed25519", "signature");
    for hash in [1, 2, 3, 11, 255] {
        let mut body = first_packet(&bytes).1.to_vec();
        body[3] = hash;
        assert_eq!(
            decode(&packet(&body)).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
    }
    for public in [
        PublicKeyAlgorithm::DSA,
        PublicKeyAlgorithm::ECDH,
        PublicKeyAlgorithm::RSAEncrypt,
    ] {
        let mut body = first_packet(&bytes).1.to_vec();
        body[2] = public.into();
        assert_eq!(
            decode(&packet(&body)).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
    }
    for limits in [
        Limits {
            max_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_elements: 1,
            ..Limits::default()
        },
    ] {
        assert_eq!(
            OpenPgpSignature::decode(&bytes, limits).unwrap_err().kind(),
            ErrorKind::Limit
        );
    }
    // Embedded signatures are recursively framed before rPGP can allocate or recurse.
    let inner = Signature::try_from_reader(first_packet(&bytes).0, first_packet(&bytes).1).unwrap();
    let nested = configured(|config| {
        config
            .unhashed_subpackets
            .push(Subpacket::regular(SubpacketData::EmbeddedSignature(Box::new(inner))).unwrap())
    });
    assert_eq!(
        OpenPgpSignature::decode(
            &nested,
            Limits {
                max_depth: 0,
                ..Limits::default()
            }
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Limit
    );
    decode(&nested)
        .unwrap()
        .verify_signature(INPUT, &public("ed25519"))
        .unwrap();
    let mut body = first_packet(&bytes).1.to_vec();
    body.extend_from_slice(&[0]);
    assert!(decode(&packet(&body)).is_err());
    assert!(decode(&[0xc2, 224, 4]).is_err());
}

#[test]
fn verification_hashes_original_bytes_even_when_library_decoding_normalizes_a_value() {
    let bytes = configured(|config| {
        config
            .hashed_subpackets
            .push(Subpacket::regular(SubpacketData::Revocable(false)).unwrap());
    });
    let mut body = first_packet(&bytes).1.to_vec();
    let index = body
        .windows(3)
        .position(|bytes| bytes == [2, 7, 0])
        .unwrap();
    body[index + 2] = 2;
    let changed = packet(&body);
    let key = public("ed25519");
    // rPGP maps both zero and two to false and re-encodes false as zero when hashing.
    // The Janex verifier must not accept that normalization of the signed wire bytes.
    let (header, body) = first_packet(&changed);
    Signature::try_from_reader(header, body)
        .unwrap()
        .verify(&key, INPUT)
        .unwrap();
    assert_eq!(
        decode(&changed)
            .unwrap()
            .verify_signature(INPUT, &key)
            .unwrap_err()
            .kind(),
        ErrorKind::Verification
    );
}

#[test]
fn packet_headers_and_nonminimal_subpacket_lengths_do_not_change_signed_fields() {
    let bytes = fixture("ed25519", "signature");
    let body = first_packet(&bytes).1;
    let key = public("ed25519");
    for mut framed in [
        vec![0x88, body.len() as u8],
        [vec![0x89], (body.len() as u16).to_be_bytes().to_vec()].concat(),
        [vec![0x8a], (body.len() as u32).to_be_bytes().to_vec()].concat(),
        vec![0x8b],
        [vec![0xc2, 255], (body.len() as u32).to_be_bytes().to_vec()].concat(),
    ] {
        framed.extend_from_slice(body);
        decode(&framed)
            .unwrap()
            .verify_signature(INPUT, &key)
            .unwrap();
    }
    let bytes = configured(|config| {
        config.hashed_subpackets[0].len = pgp::packet::SubpacketLength::Five(5);
    });
    decode(&bytes)
        .unwrap()
        .verify_signature(INPUT, &key)
        .unwrap();
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Primary-key pins, subkey bindings, validity, and supplied revocation evidence.

use super::{CREATED, INPUT, decode, first_packet, fixture, instant, packet, public, secret};
use janex_format::{
    ErrorKind,
    binary::Limits,
    signature::openpgp::{self, Algorithm, KeyCertificate},
};
use pgp::{
    crypto::hash::HashAlgorithm,
    packet::{KeyFlags, PacketHeader, SignatureConfig, SignatureType, Subpacket, SubpacketData},
    ser::Serialize,
    types::{Duration, KeyDetails, KeyVersion, Password, Tag, Timestamp},
};
use rand::rngs::OsRng;

/// Decodes one explicitly pinned binary certificate.
fn certificate(bytes: &[u8]) -> KeyCertificate {
    KeyCertificate::decode(bytes, Limits::default()).unwrap()
}

/// Encodes a complete packet for certificate composition.
fn tagged(tag: Tag, body: &[u8]) -> Vec<u8> {
    let mut output = PacketHeader::new_fixed(tag, body.len() as u32)
        .to_bytes()
        .unwrap();
    output.extend_from_slice(body);
    output
}

/// Constructs protected creation and issuer fields for a certificate-forming signature.
fn config(name: &str, typ: SignatureType, time: u32) -> SignatureConfig {
    let key = public(name);
    let mut config = if key.version() == KeyVersion::V6 {
        SignatureConfig::v6(OsRng, typ, key.algorithm(), HashAlgorithm::Sha512).unwrap()
    } else {
        SignatureConfig::v4(typ, key.algorithm(), HashAlgorithm::Sha256)
    };
    config.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::SignatureCreationTime(Timestamp::from_secs(
            time,
        )))
        .unwrap(),
        Subpacket::regular(SubpacketData::IssuerFingerprint(key.fingerprint())).unwrap(),
    ];
    config
}

/// Adds authenticated usage restrictions to a self-signature or binding.
fn flags(config: &mut SignatureConfig, sign: bool, certify: bool) {
    let mut flags = KeyFlags::default();
    flags.set_sign(sign);
    flags.set_certify(certify);
    config
        .hashed_subpackets
        .push(Subpacket::regular(SubpacketData::KeyFlags(flags)).unwrap());
}

/// Produces a direct self-signature with caller-chosen policy fields.
fn direct(name: &str, time: u32, change: impl FnOnce(&mut SignatureConfig)) -> Vec<u8> {
    let mut config = config(name, SignatureType::Key, time);
    change(&mut config);
    packet(
        &config
            .sign_key(&secret(name), &Password::empty(), &public(name))
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
}

/// Builds a minimal certificate with one direct self-signature.
fn with_policy(name: &str, time: u32, change: impl FnOnce(&mut SignatureConfig)) -> Vec<u8> {
    let mut bytes = tagged(Tag::PublicKey, first_packet(&fixture(name, "public")).1);
    bytes.extend(direct(name, time, change));
    bytes
}

/// Produces an Ed25519-primary/P-256-subkey certificate with configurable binding statements.
fn with_subkey(reverse: bool, change: impl FnOnce(&mut SignatureConfig)) -> Vec<u8> {
    let mut bytes = with_policy("ed25519", CREATED, |config| flags(config, false, true));
    bytes.extend(tagged(
        Tag::PublicSubkey,
        first_packet(&fixture("p256", "public")).1,
    ));
    let mut binding = config("ed25519", SignatureType::SubkeyBinding, CREATED);
    flags(&mut binding, true, false);
    if reverse {
        let reverse = config("p256", SignatureType::KeyBinding, CREATED)
            .sign_primary_key_binding(
                &secret("p256"),
                &public("p256"),
                &Password::empty(),
                &public("ed25519"),
            )
            .unwrap();
        binding
            .unhashed_subpackets
            .push(Subpacket::regular(SubpacketData::EmbeddedSignature(Box::new(reverse))).unwrap());
    }
    change(&mut binding);
    let binding = binding
        .sign_subkey_binding(
            &secret("ed25519"),
            &public("ed25519"),
            &Password::empty(),
            &public("p256"),
        )
        .unwrap();
    bytes.extend(packet(&binding.to_bytes().unwrap()));
    bytes
}

#[test]
fn independent_gnupg_primary_and_subkey_certificates_authenticate() {
    for name in [
        "rsa256", "rsa512", "p256", "p384", "ed25519", "subkey", "expired",
    ] {
        let cert = certificate(&fixture(name, "public"));
        let signature = decode(&fixture(name, "signature")).unwrap();
        let verified = cert.authenticate(INPUT, &signature, instant(1)).unwrap();
        assert_eq!(verified.primary_fingerprint, cert.fingerprint());
        assert_eq!(&verified.signing_fingerprint, signature.issuer());
        assert_eq!(
            verified.primary_fingerprint == verified.signing_fingerprint,
            name != "subkey"
        );
        assert!(
            certificate(&fixture(
                if name == "rsa256" { "rsa512" } else { "rsa256" },
                "public"
            ))
            .authenticate(INPUT, &signature, instant(1))
            .is_err()
        );
    }
}

#[test]
fn supplied_gnupg_revocation_and_key_expiration_prevent_authentication() {
    let revoked = certificate(&fixture("subkey", "revoked.public"));
    let signature = decode(&fixture("subkey", "signature")).unwrap();
    assert_eq!(
        revoked
            .authenticate(INPUT, &signature, instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    for name in ["subkey", "expired"] {
        let cert = certificate(&fixture(name, "public"));
        let signature = decode(&fixture(name, "signature")).unwrap();
        cert.authenticate(INPUT, &signature, instant(86_399))
            .unwrap();
        assert_eq!(
            cert.authenticate(INPUT, &signature, instant(86_400))
                .unwrap_err()
                .kind(),
            ErrorKind::Trust
        );
    }
}

#[test]
fn v6_direct_self_signature_is_required_and_primary_usage_is_enforced() {
    let bare = certificate(&fixture("v6", "public"));
    let signature = decode(&fixture("v6", "signature")).unwrap();
    assert_eq!(
        bare.authenticate(INPUT, &signature, instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    let cert = certificate(&with_policy("v6", CREATED, |config| {
        flags(config, true, true)
    }));
    cert.authenticate(INPUT, &signature, instant(1)).unwrap();
    let no_sign = certificate(&with_policy("v6", CREATED, |config| {
        flags(config, false, true)
    }));
    assert_eq!(
        no_sign
            .authenticate(INPUT, &signature, instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
}

#[test]
fn signing_subkeys_require_both_bindings_and_current_usage() {
    let signature = decode(&fixture("p256", "signature")).unwrap();
    let cert = certificate(&with_subkey(true, |_| {}));
    cert.authenticate(INPUT, &signature, instant(1)).unwrap();
    let missing_reverse = certificate(&with_subkey(false, |_| {}));
    assert_eq!(
        missing_reverse
            .authenticate(INPUT, &signature, instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    let no_sign = certificate(&with_subkey(true, |config| {
        config
            .hashed_subpackets
            .retain(|packet| !matches!(packet.data, SubpacketData::KeyFlags(_)));
        flags(config, false, false);
    }));
    assert_eq!(
        no_sign
            .authenticate(INPUT, &signature, instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    let wrong_reverse = certificate(&with_subkey(false, |binding| {
        let reverse = config("ed25519", SignatureType::KeyBinding, CREATED)
            .sign_primary_key_binding(
                &secret("ed25519"),
                &public("p256"),
                &Password::empty(),
                &public("ed25519"),
            )
            .unwrap();
        binding
            .unhashed_subpackets
            .push(Subpacket::regular(SubpacketData::EmbeddedSignature(Box::new(reverse))).unwrap());
    }));
    assert_eq!(
        wrong_reverse
            .authenticate(INPUT, &signature, instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
}

#[test]
fn newest_policy_does_not_fall_back_after_expiry_and_is_checked_at_signing_time() {
    let mut bytes = with_policy("ed25519", CREATED, |config| flags(config, true, true));
    bytes.extend(direct("ed25519", CREATED + 10, |config| {
        flags(config, true, true);
        config.hashed_subpackets.push(
            Subpacket::regular(SubpacketData::SignatureExpirationTime(Duration::from_secs(
                10,
            )))
            .unwrap(),
        );
    }));
    let cert = certificate(&bytes);
    cert.validate_signer(&cert.fingerprint(), instant(19))
        .unwrap();
    assert_eq!(
        cert.validate_signer(&cert.fingerprint(), instant(20))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    // Current signing permission cannot retroactively authorize a document made without it.
    let mut bytes = with_policy("ed25519", CREATED, |config| flags(config, false, true));
    bytes.extend(direct("ed25519", CREATED + 10, |config| {
        flags(config, true, true)
    }));
    let cert = certificate(&bytes);
    let early = decode(&fixture("ed25519", "signature")).unwrap();
    assert_eq!(
        cert.authenticate(INPUT, &early, instant(11))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    let later = openpgp::sign(
        INPUT,
        &secret("ed25519"),
        &Password::empty(),
        Algorithm::Ed25519Sha256,
        instant(10),
    )
    .unwrap();
    cert.authenticate(INPUT, &decode(&later).unwrap(), instant(11))
        .unwrap();
}

#[test]
fn subkey_revocations_are_verified_and_apply_retroactively() {
    let bytes = with_subkey(true, |_| {});
    let mut config = config("ed25519", SignatureType::SubkeyRevocation, CREATED + 10);
    config.hashed_subpackets.push(
        Subpacket::regular(SubpacketData::SignatureExpirationTime(Duration::from_secs(
            1,
        )))
        .unwrap(),
    );
    let revocation = config
        .sign_subkey_binding(
            &secret("ed25519"),
            &public("ed25519"),
            &Password::empty(),
            &public("p256"),
        )
        .unwrap();
    let mut revoked = bytes.clone();
    revoked.extend(packet(&revocation.to_bytes().unwrap()));
    let signature = decode(&fixture("p256", "signature")).unwrap();
    for now in [instant(0), instant(11), instant(60)] {
        assert_eq!(
            certificate(&revoked)
                .authenticate(INPUT, &signature, now)
                .unwrap_err()
                .kind(),
            ErrorKind::Trust
        );
    }
    let mut damaged = revocation.to_bytes().unwrap();
    *damaged.last_mut().unwrap() ^= 1;
    let mut invalid_revocation = bytes;
    invalid_revocation.extend(packet(&damaged));
    certificate(&invalid_revocation)
        .authenticate(INPUT, &signature, instant(60))
        .unwrap();
}

#[test]
fn unknown_critical_policy_and_unhashed_policy_cannot_authorize_signing() {
    let bytes = with_policy("ed25519", CREATED, |config| {
        flags(config, false, true);
        let mut allowed = KeyFlags::default();
        allowed.set_sign(true);
        config
            .unhashed_subpackets
            .push(Subpacket::regular(SubpacketData::KeyFlags(allowed)).unwrap());
    });
    let cert = certificate(&bytes);
    assert_eq!(
        cert.validate_signer(&cert.fingerprint(), instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    let mut bytes = with_policy("ed25519", CREATED, |config| {
        config.hashed_subpackets.push(
            Subpacket::critical(SubpacketData::Experimental(100, vec![1, 2].into())).unwrap(),
        );
    });
    let cert = certificate(&bytes);
    assert_eq!(
        cert.validate_signer(&cert.fingerprint(), instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Invalid
    );
    // A second primary key must never become trusted merely by sharing a file with a pin.
    bytes.extend(fixture("rsa256", "public"));
    assert_eq!(
        KeyCertificate::decode(&bytes, Limits::default())
            .unwrap_err()
            .kind(),
        ErrorKind::Invalid
    );
    assert!(KeyCertificate::decode(&fixture("ed25519", "secret"), Limits::default()).is_err());
}

#[test]
fn certificate_and_reverse_binding_verification_preserve_original_bytes() {
    let mut bytes = with_policy("ed25519", CREATED, |config| {
        config
            .hashed_subpackets
            .push(Subpacket::regular(SubpacketData::Revocable(false)).unwrap());
    });
    let index = bytes
        .windows(3)
        .position(|bytes| bytes == [2, 7, 0])
        .unwrap();
    bytes[index + 2] = 2;
    let cert = certificate(&bytes);
    assert_eq!(
        cert.validate_signer(&cert.fingerprint(), instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    let mut bytes = with_subkey(false, |binding| {
        let mut reverse = config("p256", SignatureType::KeyBinding, CREATED);
        reverse
            .hashed_subpackets
            .push(Subpacket::regular(SubpacketData::Revocable(false)).unwrap());
        let reverse = reverse
            .sign_primary_key_binding(
                &secret("p256"),
                &public("p256"),
                &Password::empty(),
                &public("ed25519"),
            )
            .unwrap();
        binding
            .unhashed_subpackets
            .push(Subpacket::regular(SubpacketData::EmbeddedSignature(Box::new(reverse))).unwrap());
    });
    certificate(&bytes)
        .validate_signer(&public("p256").fingerprint(), instant(1))
        .unwrap();
    let index = bytes
        .windows(3)
        .position(|bytes| bytes == [2, 7, 0])
        .unwrap();
    bytes[index + 2] = 2;
    assert_eq!(
        certificate(&bytes)
            .validate_signer(&public("p256").fingerprint(), instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
}

#[test]
fn parsing_limits_apply_to_key_packets_and_nested_bindings() {
    let bytes = with_subkey(true, |_| {});
    for limits in [
        Limits {
            max_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_elements: 1,
            ..Limits::default()
        },
        Limits {
            max_depth: 0,
            ..Limits::default()
        },
    ] {
        assert_eq!(
            KeyCertificate::decode(&bytes, limits).unwrap_err().kind(),
            ErrorKind::Limit
        );
    }
    let cert = certificate(&bytes);
    let mut duplicate = bytes.clone();
    duplicate.extend(tagged(
        Tag::PublicSubkey,
        first_packet(&fixture("p256", "public")).1,
    ));
    assert!(KeyCertificate::decode(&duplicate, Limits::default()).is_err());
    let signature = decode(&fixture("p256", "signature")).unwrap();
    cert.authenticate(INPUT, &signature, instant(1)).unwrap();
    let (_, primary) = first_packet(&bytes);
    let mut malformed = primary.to_vec();
    malformed.push(0);
    assert!(
        KeyCertificate::decode(&tagged(Tag::PublicKey, &malformed), Limits::default()).is_err()
    );
}

#[test]
fn public_key_armor_is_single_complete_and_bounded() {
    let bytes = include_bytes!("../fixtures/openpgp/ed25519.public.asc");
    let signature = decode(&fixture("ed25519", "signature")).unwrap();
    certificate(bytes)
        .authenticate(INPUT, &signature, instant(1))
        .unwrap();
    let mut surrounded = b" \r\n\t".to_vec();
    surrounded.extend_from_slice(bytes);
    surrounded.extend_from_slice(b"\r\n\t ");
    certificate(&surrounded)
        .authenticate(INPUT, &signature, instant(1))
        .unwrap();
    for suffix in [
        bytes.as_slice(),
        b"ignored text".as_slice(),
        fixture("rsa256", "public").as_slice(),
    ] {
        let mut joined = bytes.to_vec();
        joined.extend_from_slice(suffix);
        assert!(KeyCertificate::decode(&joined, Limits::default()).is_err());
    }
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    for changed in [
        text.replace("PUBLIC KEY BLOCK", "PRIVATE KEY BLOCK"),
        text.replace("END PGP PUBLIC KEY BLOCK", "END PGP SIGNATURE"),
    ] {
        assert!(KeyCertificate::decode(changed.as_bytes(), Limits::default()).is_err());
    }
    let footer = text.find("-----END").unwrap();
    for length in 0..footer {
        assert!(KeyCertificate::decode(&bytes[..length], Limits::default()).is_err());
    }
}

#[test]
fn identity_revocation_recertification_and_third_party_signatures_do_not_expand_trust() {
    let uid = pgp::packet::UserId::from_str(pgp::types::PacketHeaderVersion::New, "Test identity")
        .unwrap();
    let statement = |issuer: &str, typ, time| {
        let mut config = config(issuer, typ, time);
        flags(&mut config, true, true);
        let signature = config
            .sign_certification_third_party(
                &secret(issuer),
                &Password::empty(),
                &public("ed25519"),
                Tag::UserId,
                &uid,
            )
            .unwrap();
        packet(&signature.to_bytes().unwrap())
    };
    let mut bytes = tagged(
        Tag::PublicKey,
        first_packet(&fixture("ed25519", "public")).1,
    );
    bytes.extend(tagged(Tag::UserId, &uid.to_bytes().unwrap()));
    bytes.extend(statement("p256", SignatureType::CertPositive, CREATED));
    let cert = certificate(&bytes);
    assert_eq!(
        cert.validate_signer(&cert.fingerprint(), instant(1))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    bytes.extend(statement("ed25519", SignatureType::CertPositive, CREATED));
    certificate(&bytes)
        .validate_signer(&public("ed25519").fingerprint(), instant(1))
        .unwrap();
    bytes.extend(statement(
        "ed25519",
        SignatureType::CertRevocation,
        CREATED + 10,
    ));
    let cert = certificate(&bytes);
    cert.validate_signer(&cert.fingerprint(), instant(9))
        .unwrap();
    assert_eq!(
        cert.validate_signer(&cert.fingerprint(), instant(10))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
    bytes.extend(statement(
        "ed25519",
        SignatureType::CertPositive,
        CREATED + 11,
    ));
    certificate(&bytes)
        .validate_signer(&public("ed25519").fingerprint(), instant(11))
        .unwrap();
}

#[test]
fn simultaneous_self_signatures_must_agree_on_key_policy() {
    let mut bytes = with_policy("ed25519", CREATED, |config| flags(config, true, true));
    bytes.extend(direct("ed25519", CREATED, |config| {
        flags(config, true, true);
        config
            .hashed_subpackets
            .push(Subpacket::regular(SubpacketData::IsPrimary(true)).unwrap());
    }));
    let cert = certificate(&bytes);
    cert.validate_signer(&cert.fingerprint(), instant(1))
        .unwrap();
    bytes.extend(direct("ed25519", CREATED, |config| {
        flags(config, false, true)
    }));
    let cert = certificate(&bytes);
    assert!(
        cert.validate_signer(&cert.fingerprint(), instant(1))
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
}

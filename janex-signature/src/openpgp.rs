// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Janex OpenPGP document signatures over the original protected bytes.
//!
//! Cryptographic verification does not establish publisher trust. The caller must separately
//! authorize the supplied key and validate its certifications, signing usage, expiry, and revocation.
//! [`KeyCertificate::authenticate`] applies explicit primary-key pins and offline certification policy.

mod framing;
mod signer;
mod trust;

pub use signer::OpenPgpSigner;
pub use trust::{KeyCertificate, VerifiedSigner};

use super::{trust as trust_error, unsupported, verification};
use crate::{Result, binary::Limits, error::invalid};
use pgp::{
    crypto::{hash::HashAlgorithm, public_key::PublicKeyAlgorithm},
    packet::{
        PacketHeader, Signature, SignatureConfig, SignatureType, SignatureVersionSpecific,
        Subpacket, SubpacketData,
    },
    ser::Serialize,
    types::{
        EcdsaPublicParams, EddsaLegacyPublicParams, Fingerprint, KeyDetails, KeyVersion, Password,
        PublicParams, SigningKey, Tag, Timestamp, VerifyingKey,
    },
};
use rand::rngs::OsRng;
use rsa::traits::PublicKeyParts;
use std::{
    io::Cursor,
    time::{SystemTime, UNIX_EPOCH},
};

/// Supported public-key and content-hash combinations for document signatures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Algorithm {
    /// RSA with a 2048–8192-bit modulus and SHA-256.
    RsaSha256,
    /// RSA with a 2048–8192-bit modulus and SHA-512.
    RsaSha512,
    /// ECDSA on NIST P-256 with SHA-256.
    EcdsaP256Sha256,
    /// ECDSA on NIST P-384 with SHA-384.
    EcdsaP384Sha384,
    /// Ed25519 with SHA-256, using either OpenPGP Ed25519 representation.
    Ed25519Sha256,
    /// Ed25519 with SHA-512, using either OpenPGP Ed25519 representation.
    Ed25519Sha512,
}

impl Algorithm {
    /// Returns the digest used for the document and protected signature fields.
    fn hash(self) -> HashAlgorithm {
        match self {
            Self::RsaSha256 | Self::EcdsaP256Sha256 | Self::Ed25519Sha256 => HashAlgorithm::Sha256,
            Self::RsaSha512 | Self::Ed25519Sha512 => HashAlgorithm::Sha512,
            Self::EcdsaP384Sha384 => HashAlgorithm::Sha384,
        }
    }
}

/// A single detached binary document signature satisfying the Janex packet profile.
#[derive(Clone, Debug)]
pub struct OpenPgpSignature {
    /// Parsed cryptographic parameters and signature values.
    signature: Signature,
    /// Original signature fields through the hashed subpacket area, without re-encoding.
    protected: Vec<u8>,
    /// The unique fingerprint from the hashed subpacket area.
    issuer: Fingerprint,
    /// The unique authenticated creation time.
    created: Timestamp,
    /// Authenticated lifetime in seconds, with zero denoting no expiration.
    lifetime: u32,
}

impl OpenPgpSignature {
    /// Decodes exactly one binary Signature packet under the supplied byte and structure limits.
    ///
    /// Rejects armor, other packet types, trailing bytes, unsupported versions or algorithms,
    /// non-binary signature types, missing or duplicate protected identity/time fields, and
    /// unsupported critical subpackets. This does not verify the signature or establish trust.
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        limits.bytes(bytes.len() as u64)?;
        limits.elements(1)?;
        let mut rest = bytes;
        let (header, body) = framing::packet(&mut rest)?;
        if header.tag() != Tag::Signature || !rest.is_empty() {
            return Err(invalid(
                "OpenPGP payload must contain exactly one Signature packet",
            ));
        }
        let hashed_end = framing::signature(body, limits)?;
        check_algorithms(body[2].into(), body[3].into())?;
        let mut cursor = Cursor::new(body);
        let signature = Signature::try_from_reader(header, &mut cursor)
            .map_err(|_| invalid("invalid OpenPGP signature packet"))?;
        if cursor.position() != body.len() as u64 {
            return Err(invalid("trailing bytes in OpenPGP signature packet"));
        }
        let config = signature.config().expect("prevalidated signature version");
        if config.typ != SignatureType::Binary {
            return Err(invalid(
                "Janex requires an OpenPGP binary document signature",
            ));
        }
        let mut issuer = None;
        let mut created = None;
        let mut lifetime = None;
        for packet in config.hashed_subpackets() {
            match &packet.data {
                SubpacketData::IssuerFingerprint(value) => unique(&mut issuer, value.clone())?,
                SubpacketData::SignatureCreationTime(value) => unique(&mut created, *value)?,
                SubpacketData::SignatureExpirationTime(value) => {
                    unique(&mut lifetime, value.as_secs())?
                }
                _ => (),
            }
        }
        for packet in config
            .hashed_subpackets()
            .chain(config.unhashed_subpackets())
        {
            if packet.is_critical
                && !matches!(
                    packet.data,
                    SubpacketData::IssuerFingerprint(_)
                        | SubpacketData::IssuerKeyId(_)
                        | SubpacketData::SignatureCreationTime(_)
                        | SubpacketData::SignatureExpirationTime(_)
                )
            {
                return Err(invalid(
                    "unsupported critical OpenPGP document-signature subpacket",
                ));
            }
        }
        let issuer = issuer.ok_or_else(|| invalid("missing hashed OpenPGP issuer fingerprint"))?;
        let created =
            created.ok_or_else(|| invalid("missing hashed OpenPGP signature creation time"))?;
        if !matches!(
            (body[0], issuer.version()),
            (4, Some(KeyVersion::V4)) | (6, Some(KeyVersion::V6))
        ) {
            return Err(invalid(
                "OpenPGP issuer fingerprint and signature versions differ",
            ));
        }
        Ok(Self {
            signature,
            protected: body[..hashed_end].to_vec(),
            issuer,
            created,
            lifetime: lifetime.unwrap_or(0),
        })
    }

    /// Returns the fingerprint from the hashed subpacket area; it is untrusted until verified.
    pub fn issuer(&self) -> &Fingerprint {
        &self.issuer
    }

    /// Returns the claimed creation time; it is untrusted until verified.
    pub fn created(&self) -> SystemTime {
        self.created.into()
    }

    /// Checks the signature's creation and expiration times against `now`.
    ///
    /// Expiration is exclusive. A zero lifetime means no signature expiration. This does not
    /// validate the key's own lifetime, bindings, permitted usage, revocation, or cryptographic proof.
    pub fn validate_time(&self, now: SystemTime) -> Result<()> {
        let seconds = now
            .duration_since(UNIX_EPOCH)
            .map_err(|_| trust_error("OpenPGP verification time precedes the Unix epoch"))?
            .as_secs();
        let created = u64::from(self.created.as_secs());
        if seconds < created {
            return Err(trust_error("OpenPGP signature is not yet valid"));
        }
        if self.lifetime != 0 && seconds >= created + u64::from(self.lifetime) {
            return Err(trust_error("OpenPGP signature has expired"));
        }
        Ok(())
    }

    /// Verifies the exact document and original protected bytes with the supplied key.
    ///
    /// Checks the hashed issuer fingerprint, key/signature version and algorithm agreement,
    /// supported curve or RSA strength, and the cryptographic signature. Unhashed issuer fields
    /// do not select or authorize a key. The caller must separately establish trust and validate
    /// signature time and key policy; success here alone never authorizes execution.
    pub fn verify_signature(&self, input: &[u8], key: &impl VerifyingKey) -> Result<Algorithm> {
        if key.fingerprint() != self.issuer
            || key.version() != self.issuer.version().expect("validated issuer")
        {
            return Err(verification(
                "OpenPGP hashed issuer does not match the verifying key",
            ));
        }
        verify_raw(&self.signature, &self.protected, &[input], key)
    }
}

/// Verifies raw protected fields for either a document or certificate-forming signature.
fn verify_raw(
    signature: &Signature,
    protected: &[u8],
    input: &[&[u8]],
    key: &impl VerifyingKey,
) -> Result<Algorithm> {
    let config = signature.config().expect("decoded signature");
    if key.algorithm() != config.pub_alg || u8::from(key.version()) != protected[0] {
        return Err(verification(
            "OpenPGP signature and key algorithms or versions differ",
        ));
    }
    let algorithm = key_algorithm(key, config.hash_alg)?;
    let mut hasher = config
        .hash_alg
        .new_hasher()
        .map_err(|_| unsupported("unsupported OpenPGP digest"))?;
    if let SignatureVersionSpecific::V6 { salt } = &config.version_specific {
        hasher.update(salt);
    }
    for bytes in input {
        hasher.update(bytes);
    }
    hasher.update(protected);
    hasher.update(&[protected[0], 0xff]);
    let length =
        u32::try_from(protected.len()).map_err(|_| invalid("OpenPGP hashed area exceeds u32"))?;
    hasher.update(&length.to_be_bytes());
    let hash = hasher.finalize();
    if signature.signed_hash_value().expect("decoded signature") != hash[..2] {
        return Err(verification("OpenPGP signed digest mismatch"));
    }
    key.verify(
        config.hash_alg,
        &hash,
        signature.signature().expect("decoded signature"),
    )
    .map_err(|_| verification("invalid OpenPGP signature"))?;
    Ok(algorithm)
}

/// Signs the exact document with one caller-authorized primary key or signing subkey.
///
/// Emits one version-4 or version-6 binary Signature packet, according to the key version,
/// with hashed creation time and issuer fingerprint. The caller owns key unlocking and must
/// validate the key's certifications, usage, expiry, and revocation before signing. This function
/// checks the algorithm and key strength, but does not establish trust or load a key store.
pub fn sign(
    input: &[u8],
    key: &impl SigningKey,
    password: &Password,
    algorithm: Algorithm,
    now: SystemTime,
) -> Result<Vec<u8>> {
    if key_algorithm(key, algorithm.hash())? != algorithm {
        return Err(invalid("OpenPGP signing algorithm does not match the key"));
    }
    let created = Timestamp::try_from(now)
        .map_err(|_| invalid("OpenPGP signing time does not fit u32 seconds"))?;
    if created < key.created_at() {
        return Err(trust_error("OpenPGP signing time precedes key creation"));
    }
    let mut config = match key.version() {
        KeyVersion::V4 => {
            SignatureConfig::v4(SignatureType::Binary, key.algorithm(), algorithm.hash())
        }
        KeyVersion::V6 => SignatureConfig::v6(
            &mut OsRng,
            SignatureType::Binary,
            key.algorithm(),
            algorithm.hash(),
        )
        .map_err(|_| invalid("cannot configure OpenPGP signature"))?,
        _ => unreachable!("validated signing key version"),
    };
    config.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::SignatureCreationTime(created))
            .map_err(|_| invalid("invalid OpenPGP creation time"))?,
        Subpacket::regular(SubpacketData::IssuerFingerprint(key.fingerprint()))
            .map_err(|_| invalid("invalid OpenPGP issuer fingerprint"))?,
    ];
    let signature = config
        .sign(key, password, input)
        .map_err(|_| verification("OpenPGP signing failed"))?;
    let body = signature
        .to_bytes()
        .map_err(|_| invalid("cannot encode OpenPGP signature"))?;
    let length = u32::try_from(body.len()).map_err(|_| invalid("OpenPGP signature exceeds u32"))?;
    let mut output = PacketHeader::new_fixed(Tag::Signature, length)
        .to_bytes()
        .map_err(|_| invalid("cannot encode OpenPGP signature header"))?;
    output.extend_from_slice(&body);
    Ok(output)
}

/// Rejects conflicting or duplicate authenticated scalar fields.
fn unique<T>(slot: &mut Option<T>, value: T) -> Result<()> {
    if slot.replace(value).is_some() {
        return Err(invalid(
            "duplicate hashed OpenPGP identity or time subpacket",
        ));
    }
    Ok(())
}

/// Rejects unsupported packet algorithms before signature parsing or hashing.
fn check_algorithms(public: PublicKeyAlgorithm, hash: HashAlgorithm) -> Result<()> {
    if !matches!(
        public,
        PublicKeyAlgorithm::RSA
            | PublicKeyAlgorithm::RSASign
            | PublicKeyAlgorithm::ECDSA
            | PublicKeyAlgorithm::EdDSALegacy
            | PublicKeyAlgorithm::Ed25519
    ) {
        return Err(unsupported("unsupported OpenPGP signing algorithm"));
    }
    if !matches!(
        hash,
        HashAlgorithm::Sha256 | HashAlgorithm::Sha384 | HashAlgorithm::Sha512
    ) {
        return Err(unsupported("unsupported OpenPGP signature digest"));
    }
    Ok(())
}

/// Resolves an allowed combination and checks the concrete public-key parameters.
fn key_algorithm(key: &(impl KeyDetails + ?Sized), hash: HashAlgorithm) -> Result<Algorithm> {
    if !matches!(key.version(), KeyVersion::V4 | KeyVersion::V6) {
        return Err(unsupported("OpenPGP signing keys must use version 4 or 6"));
    }
    check_algorithms(key.algorithm(), hash)?;
    let algorithm = match (key.public_params(), hash) {
        (PublicParams::RSA(parameters), hash)
            if matches!(
                key.algorithm(),
                PublicKeyAlgorithm::RSA | PublicKeyAlgorithm::RSASign
            ) =>
        {
            if !(2048..=8192).contains(&parameters.key.n().bits()) {
                return Err(unsupported(
                    "OpenPGP RSA modulus must contain 2048 to 8192 bits",
                ));
            }
            match hash {
                HashAlgorithm::Sha256 => Algorithm::RsaSha256,
                HashAlgorithm::Sha512 => Algorithm::RsaSha512,
                _ => return Err(unsupported("unsupported OpenPGP RSA digest")),
            }
        }
        (PublicParams::ECDSA(EcdsaPublicParams::P256 { .. }), HashAlgorithm::Sha256) => {
            Algorithm::EcdsaP256Sha256
        }
        (PublicParams::ECDSA(EcdsaPublicParams::P384 { .. }), HashAlgorithm::Sha384) => {
            Algorithm::EcdsaP384Sha384
        }
        (
            PublicParams::Ed25519(_)
            | PublicParams::EdDSALegacy(EddsaLegacyPublicParams::Ed25519 { .. }),
            hash,
        ) => match hash {
            HashAlgorithm::Sha256 => Algorithm::Ed25519Sha256,
            HashAlgorithm::Sha512 => Algorithm::Ed25519Sha512,
            _ => return Err(unsupported("unsupported OpenPGP Ed25519 digest")),
        },
        _ => {
            return Err(unsupported(
                "unsupported OpenPGP key and digest combination",
            ));
        }
    };
    Ok(algorithm)
}

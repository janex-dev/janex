// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Transferable secret keys, bounded password derivation, and certified signing-key selection.

use super::{
    Algorithm, KeyCertificate, OpenPgpSignature, framing, key_algorithm, trust_error, unsupported,
    verification,
};
use crate::{
    Error, ErrorKind, Result,
    binary::{Decoder, Limits},
    error::invalid,
};
use pgp::{
    armor::BlockType,
    crypto::{hash::HashAlgorithm, sym::SymmetricKeyAlgorithm},
    packet::{PacketHeader, PublicKey, SecretKey, SecretSubkey},
    ser::Serialize,
    types::{
        Fingerprint, KeyDetails, KeyVersion, Password, PublicParams, S2kParams, SecretParams,
        StringToKey, Tag,
    },
};
use std::{fmt, io::Cursor, time::SystemTime};
use zeroize::Zeroizing;

/// A selected private signing key and the certificate authorizing its use.
///
/// Decoding validates the certificate and selects a key before any password is requested. Encrypted
/// material must be unlocked before signing. Debug output excludes private parameters and passwords.
pub struct OpenPgpSigner {
    /// The selected primary key or signing subkey, with its original packet type.
    key: SecretMaterial,
    /// Public evidence used to recheck signing authorization at each signing instant.
    certificate: KeyCertificate,
    /// Supported signature and digest combination selected when loading the key.
    algorithm: Algorithm,
    /// Bounds used to validate generated signatures.
    limits: Limits,
}

/// Retains the packet type required for authenticated secret-key decryption.
enum SecretMaterial {
    /// A private primary-key packet.
    Primary(Box<SecretKey>),
    /// A private subkey packet.
    Subkey(Box<SecretSubkey>),
}

impl fmt::Debug for OpenPgpSigner {
    /// Reports public identity, algorithm, and lock state without exposing secret material.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenPgpSigner")
            .field("fingerprint", &self.fingerprint())
            .field("algorithm", &self.algorithm)
            .field("encrypted", &self.is_encrypted())
            .finish_non_exhaustive()
    }
}

impl OpenPgpSigner {
    /// Loads one binary or ASCII-armored transferable secret key and selects an authorized signer.
    ///
    /// `fingerprint`, when present, must be the complete hexadecimal primary or subkey fingerprint;
    /// letter case is ignored. Otherwise the newest eligible secret signing subkey is preferred,
    /// followed by the primary key. Equal creation times are ordered by ascending fingerprint.
    /// An absent algorithm selects SHA-256, except P-384 uses SHA-384. Explicit selections must
    /// match the key. Key certifications, usage, expiration, and supplied revocations are checked
    /// at `now`. Public-only keys cannot be selected.
    ///
    /// Accepts canonical version-4 or version-6 secret packets with AES CFB/SHA-1 integrity or AEAD
    /// protection. Password derivation supports salted or iterated SHA-1/SHA-2 for v4, salted or
    /// iterated SHA-2 for v6, and Argon2 with AEAD. Argon2 is limited to 256 MiB, 10 passes, 16 lanes,
    /// and 1 GiB of memory times passes. Unsupported protection or excessive costs fail before
    /// decryption. This method retains only the selected private key and never requests a password.
    pub fn decode(
        bytes: &[u8],
        fingerprint: Option<&str>,
        algorithm: Option<Algorithm>,
        limits: Limits,
        now: SystemTime,
    ) -> Result<Self> {
        limits.bytes(bytes.len() as u64)?;
        if let Some(fingerprint) = fingerprint
            && (!matches!(fingerprint.len(), 40 | 64)
                || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(invalid(
                "OpenPGP signing-key selection requires a complete hexadecimal fingerprint",
            ));
        }
        let decoded;
        let trimmed = bytes.trim_ascii_start();
        let mut rest = if trimmed.starts_with(b"-----BEGIN ") {
            decoded = framing::armor(trimmed, BlockType::PrivateKey, limits)?;
            decoded.as_slice()
        } else {
            bytes
        };
        let mut public = Vec::new();
        let mut keys = Vec::new();
        let mut count = 0;
        while !rest.is_empty() {
            count += 1;
            limits.elements(count)?;
            let (header, body) = framing::packet(&mut rest)?;
            if matches!(header.tag(), Tag::SecretKey | Tag::SecretSubkey) {
                let mut cursor = Cursor::new(body);
                let public = PublicKey::try_from_reader(
                    PacketHeader::new_fixed(Tag::PublicKey, body.len() as u32),
                    &mut cursor,
                )
                .map_err(|_| invalid("invalid OpenPGP secret-key public parameters"))?;
                let secret = &body[cursor.position() as usize..];
                if secret.first() == Some(&0)
                    && matches!(public.public_params(), PublicParams::RSA(_))
                {
                    check_rsa_coefficient(secret, limits)?;
                }
            }
            let material = match header.tag() {
                Tag::SecretKey => Some(SecretMaterial::Primary(Box::new(
                    SecretKey::try_from_reader(header, body)
                        .map_err(|_| invalid("invalid OpenPGP secret primary key"))?,
                ))),
                Tag::SecretSubkey => Some(SecretMaterial::Subkey(Box::new(
                    SecretSubkey::try_from_reader(header, body)
                        .map_err(|_| invalid("invalid OpenPGP secret subkey"))?,
                ))),
                _ => None,
            };
            if let Some(material) = material {
                let (tag, public_body) = match &material {
                    SecretMaterial::Primary(key) => (Tag::PublicKey, key.public_key().to_bytes()),
                    SecretMaterial::Subkey(key) => (Tag::PublicSubkey, key.public_key().to_bytes()),
                };
                let public_body =
                    public_body.map_err(|_| invalid("cannot encode OpenPGP public key"))?;
                if !body.starts_with(&public_body) {
                    return Err(invalid("noncanonical OpenPGP public-key body"));
                }
                let mut canonical = Zeroizing::new(Vec::new());
                match &material {
                    SecretMaterial::Primary(key) => key.to_writer(&mut *canonical),
                    SecretMaterial::Subkey(key) => key.to_writer(&mut *canonical),
                }
                .map_err(|_| invalid("cannot encode OpenPGP secret key"))?;
                if canonical.as_slice() != body {
                    return Err(invalid("noncanonical OpenPGP secret-key body"));
                }
                append_packet(&mut public, tag, &public_body)?;
                keys.push(material);
            } else {
                append_packet(&mut public, header.tag(), body)?;
            }
        }
        let certificate = KeyCertificate::decode(&public, limits)?;
        keys.sort_by_key(|key| {
            (
                matches!(key, SecretMaterial::Primary(_)),
                std::cmp::Reverse(key.details().created_at()),
                key.details().fingerprint().to_string(),
            )
        });
        let mut failure = None;
        for key in keys {
            if fingerprint.is_some_and(|wanted| {
                !key.details()
                    .fingerprint()
                    .to_string()
                    .eq_ignore_ascii_case(wanted)
            }) {
                continue;
            }
            let candidate = (|| {
                certificate.validate_signer(&key.details().fingerprint(), now)?;
                let selected = key_algorithm(
                    key.details(),
                    algorithm
                        .map(Algorithm::hash)
                        .unwrap_or_else(|| key.details().public_params().hash_alg()),
                )?;
                if algorithm.is_some_and(|requested| selected != requested) {
                    return Err(invalid("OpenPGP signing algorithm does not match the key"));
                }
                Ok(selected)
            })();
            match candidate {
                Ok(algorithm) => {
                    check_protection(key.params(), key.details().version())?;
                    return Ok(Self {
                        key,
                        certificate,
                        algorithm,
                        limits,
                    });
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        Err(failure.unwrap_or_else(|| trust_error("no eligible OpenPGP secret signing key")))
    }

    /// Returns the public certificate used to authorize this signing key.
    pub fn certificate(&self) -> &KeyCertificate {
        &self.certificate
    }

    /// Returns the selected primary or subkey fingerprint.
    pub fn fingerprint(&self) -> Fingerprint {
        self.key.details().fingerprint()
    }

    /// Returns the selected signature and digest combination.
    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    /// Returns whether the selected key still requires password decryption.
    pub fn is_encrypted(&self) -> bool {
        self.key.params().is_encrypted()
    }

    /// Decrypts the selected key and checks that its secret parameters match its public key.
    ///
    /// Unencrypted keys ignore `password` but still undergo the consistency check. On success,
    /// the returned signer retains unlocked key material, not the password. On failure the consumed
    /// signer is dropped; callers wishing to retry must decode their input again.
    pub fn unlock(mut self, password: &[u8]) -> Result<Self> {
        let password = Password::from(password);
        match &mut self.key {
            SecretMaterial::Primary(key) => key.remove_password(&password),
            SecretMaterial::Subkey(key) => key.remove_password(&password),
        }
        .map_err(|_| verification("cannot unlock OpenPGP private key"))?;
        self.checked_signature(
            b"Janex OpenPGP private-key consistency check",
            self.key.details().created_at().into(),
        )?;
        Ok(self)
    }

    /// Signs the exact input after rechecking the key's authorization at `now`.
    ///
    /// Fails if the selected key is still encrypted or its current certificate policy disallows
    /// signing. The generated proof is verified before returning one binary Signature packet.
    pub fn sign(&self, input: &[u8], now: SystemTime) -> Result<Vec<u8>> {
        if self.is_encrypted() {
            return Err(invalid(
                "OpenPGP signing key must be unlocked before signing",
            ));
        }
        self.certificate.validate_signer(&self.fingerprint(), now)?;
        self.checked_signature(input, now)
    }

    /// Generates a signature and checks private/public consistency without rechecking policy.
    fn checked_signature(&self, input: &[u8], now: SystemTime) -> Result<Vec<u8>> {
        let password = Password::empty();
        let bytes = match &self.key {
            SecretMaterial::Primary(key) => {
                super::sign(input, &**key, &password, self.algorithm, now)?
            }
            SecretMaterial::Subkey(key) => {
                super::sign(input, &**key, &password, self.algorithm, now)?
            }
        };
        let signature = OpenPgpSignature::decode(&bytes, self.limits)?;
        match &self.key {
            SecretMaterial::Primary(key) => signature.verify_signature(input, key.public_key()),
            SecretMaterial::Subkey(key) => signature.verify_signature(input, key.public_key()),
        }?;
        Ok(bytes)
    }
}

/// Checks the supplied RSA coefficient before serialization recomputes its modular inverse.
///
/// The underlying RSA parser can accept non-coprime factors; its OpenPGP serializer assumes the
/// inverse exists. Requiring p * u = 1 (mod q) establishes that precondition without computing one.
fn check_rsa_coefficient(bytes: &[u8], limits: Limits) -> Result<()> {
    let mut reader = Decoder::new(bytes, limits)?;
    if reader.u8()? != 0 {
        return Err(invalid("expected unencrypted OpenPGP RSA parameters"));
    }
    let mut parameters = Zeroizing::new(Vec::new());
    for _ in 0..4 {
        let bits = u16::from_be_bytes(reader.take(2)?.try_into().expect("fixed length"));
        if bits > 8192 {
            return Err(unsupported(
                "OpenPGP RSA private parameters exceed 8192 bits",
            ));
        }
        parameters.push(rsa::BigUint::from_bytes_be(
            reader.take(usize::from(bits).div_ceil(8))?,
        ));
    }
    let q = &parameters[2];
    let one = rsa::BigUint::from(1_u8);
    if q <= &one || (&parameters[1] * &parameters[3]) % q != one {
        return Err(invalid("invalid OpenPGP RSA private coefficient"));
    }
    Ok(())
}

impl SecretMaterial {
    /// Borrows public identity and algorithm information without exposing the private parameters.
    fn details(&self) -> &dyn KeyDetails {
        match self {
            Self::Primary(key) => key.public_key(),
            Self::Subkey(key) => key.public_key(),
        }
    }

    /// Borrows the selected packet's secret protection parameters.
    fn params(&self) -> &SecretParams {
        match self {
            Self::Primary(key) => key.secret_params(),
            Self::Subkey(key) => key.secret_params(),
        }
    }
}

/// Appends one public packet without re-encoding certification bodies.
fn append_packet(output: &mut Vec<u8>, tag: Tag, body: &[u8]) -> Result<()> {
    let length =
        u32::try_from(body.len()).map_err(|_| invalid("OpenPGP key packet exceeds u32"))?;
    PacketHeader::new_fixed(tag, length)
        .to_writer(output)
        .map_err(|_| invalid("cannot encode OpenPGP public packet header"))?;
    output.extend_from_slice(body);
    Ok(())
}

/// Rejects unsupported encryption and unbounded derivation before a password can be requested.
fn check_protection(params: &SecretParams, version: KeyVersion) -> Result<()> {
    let SecretParams::Encrypted(encrypted) = params else {
        return Ok(());
    };
    let (sym_alg, s2k, aead) = match encrypted.string_to_key_params() {
        S2kParams::Cfb { sym_alg, s2k, .. } => (sym_alg, s2k, false),
        S2kParams::Aead { sym_alg, s2k, .. } => (sym_alg, s2k, true),
        _ => return Err(unsupported("unsupported OpenPGP secret-key protection")),
    };
    if !matches!(
        sym_alg,
        SymmetricKeyAlgorithm::AES128
            | SymmetricKeyAlgorithm::AES192
            | SymmetricKeyAlgorithm::AES256
    ) {
        return Err(unsupported("OpenPGP secret-key protection requires AES"));
    }
    match s2k {
        StringToKey::Salted { hash_alg, .. } | StringToKey::IteratedAndSalted { hash_alg, .. }
            if matches!(
                hash_alg,
                HashAlgorithm::Sha256 | HashAlgorithm::Sha384 | HashAlgorithm::Sha512
            ) || (version == KeyVersion::V4 && *hash_alg == HashAlgorithm::Sha1) =>
        {
            Ok(())
        }
        StringToKey::Argon2 { t, p, m_enc, .. } if aead => {
            let memory = 1_u64.checked_shl(u32::from(*m_enc)).unwrap_or(u64::MAX);
            if *t == 0 || *p == 0 || memory < 8 * u64::from(*p) {
                return Err(invalid("invalid OpenPGP Argon2 parameters"));
            }
            if *t > 10
                || *p > 16
                || memory > 262_144
                || memory.saturating_mul(u64::from(*t)) > 1_048_576
            {
                return Err(Error::new(
                    ErrorKind::Limit,
                    "OpenPGP Argon2 derivation exceeds resource limits",
                ));
            }
            Ok(())
        }
        _ => Err(unsupported("unsupported OpenPGP password derivation")),
    }
}

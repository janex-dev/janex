// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Explicit primary-key trust and offline certification policy.

use super::{Algorithm, OpenPgpSignature, framing, trust_error, unique, unsupported, verify_raw};
use crate::{ErrorKind, Result, binary::Limits, error::invalid};
use pgp::{
    armor::BlockType,
    packet::{
        KeyFlags, PacketHeader, PublicKey, PublicSubkey, Signature, SignatureType, SubpacketData,
    },
    ser::Serialize,
    types::{Fingerprint, KeyDetails, KeyVersion, Tag, VerifyingKey},
};
use std::{
    io::Cursor,
    time::{SystemTime, UNIX_EPOCH},
};

/// One explicitly supplied primary key and its certifications, identities, and public subkeys.
///
/// Decoding does not establish trust. Passing this certificate to [`Self::authenticate`] pins its
/// primary key directly; user IDs and third-party certifications do not authorize other keys.
#[derive(Clone, Debug)]
pub struct KeyCertificate {
    /// Original binary public certificate, including certification and revocation packets.
    encoded: Vec<u8>,
    /// The caller-pinned primary public key.
    primary: PublicKey,
    /// Canonical key body preceded by its OpenPGP certification hash prefix.
    primary_input: Vec<u8>,
    /// Direct-key self-signatures and revocations.
    direct: Vec<KeySignature>,
    /// User ID and user-attribute certifications.
    identities: Vec<Identity>,
    /// Subkeys with their own binding and revocation packets.
    subkeys: Vec<Subkey>,
    /// Parsing limits retained for embedded cross-certifications.
    limits: Limits,
}

/// Records a trusted primary identity and the concrete key that authenticated a document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedSigner {
    /// The explicitly pinned primary-key fingerprint.
    pub primary_fingerprint: Fingerprint,
    /// The actual primary or subkey fingerprint in the document's hashed issuer field.
    pub signing_fingerprint: Fingerprint,
    /// The document signature's validated algorithm and digest combination.
    pub algorithm: Algorithm,
}

/// A user identity with its complete certification input and attached signatures.
#[derive(Clone, Debug)]
struct Identity {
    /// Exact user ID or user-attribute packet body with its certification hash prefix.
    input: Vec<u8>,
    /// Certifications and certification revocations for this identity.
    signatures: Vec<KeySignature>,
}

/// A public subkey and the evidence required to authorize it for signing.
#[derive(Clone, Debug)]
struct Subkey {
    /// The public subkey, including its own version and creation time.
    key: PublicSubkey,
    /// Exact subkey body with its certification hash prefix.
    input: Vec<u8>,
    /// Primary-issued bindings and subkey revocations.
    signatures: Vec<KeySignature>,
}

/// A certificate-forming signature retaining the exact bytes used by its issuer.
#[derive(Clone, Debug)]
struct KeySignature {
    /// Parsed fields and cryptographic signature values.
    signature: Signature,
    /// Original complete packet body, including embedded signatures.
    body: Vec<u8>,
    /// End of the original protected fields within the body.
    protected_end: usize,
}

/// Authenticated policy fields of a certificate-forming signature.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Policy {
    /// Signature creation time in Unix seconds.
    created: u64,
    /// Signature lifetime in seconds; zero means no expiration.
    lifetime: u32,
    /// Key lifetime measured from the key packet's creation; zero means no expiration.
    key_lifetime: u32,
    /// Explicit key usage, or no restriction when absent.
    flags: Option<KeyFlags>,
}

impl KeyCertificate {
    /// Decodes one binary or ASCII-armored transferable public key under the supplied limits.
    ///
    /// Accepts version-4 and version-6 keys, user IDs, opaque user attributes, signatures, and
    /// valid Marker packets. Subkeys must use their primary key's packet version.
    /// Rejects secret keys, multiple primary keys, duplicate subkeys,
    /// unsupported packets, malformed framing, and noncanonical public-key bodies. Armor must
    /// contain one public-key block with only surrounding ASCII whitespace. Its optional CRC24
    /// is not used for authentication. No key store, network, or system trust is consulted.
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        limits.bytes(bytes.len() as u64)?;
        let decoded;
        let trimmed = bytes.trim_ascii_start();
        let bytes = if trimmed.starts_with(b"-----BEGIN ") {
            decoded = framing::armor(trimmed, BlockType::PublicKey, limits)?;
            decoded.as_slice()
        } else {
            bytes
        };
        let mut rest = bytes;
        let mut certificate: Option<Self> = None;
        let mut component = None;
        let mut count = 0;
        while !rest.is_empty() {
            count += 1;
            limits.elements(count)?;
            let (header, body) = framing::packet(&mut rest)?;
            if header.tag() == Tag::Marker && body == b"PGP" {
                continue;
            }
            if certificate.is_none() {
                if header.tag() != Tag::PublicKey {
                    return Err(invalid(
                        "OpenPGP trust material must start with a public primary key",
                    ));
                }
                let primary = PublicKey::try_from_reader(header, body)
                    .map_err(|_| invalid("invalid OpenPGP primary key"))?;
                check_key(&primary, body)?;
                certificate = Some(Self {
                    encoded: bytes.to_vec(),
                    primary_input: key_input(body)?,
                    primary,
                    direct: Vec::new(),
                    identities: Vec::new(),
                    subkeys: Vec::new(),
                    limits,
                });
                continue;
            }
            let cert = certificate.as_mut().expect("primary key decoded");
            match header.tag() {
                Tag::PublicKey => {
                    return Err(invalid(
                        "OpenPGP trust material contains multiple primary keys",
                    ));
                }
                Tag::UserId | Tag::UserAttribute => {
                    let mut input = Vec::new();
                    input.push(if header.tag() == Tag::UserId {
                        0xb4
                    } else {
                        0xd1
                    });
                    input.extend_from_slice(
                        &u32::try_from(body.len())
                            .map_err(|_| invalid("OpenPGP identity exceeds u32"))?
                            .to_be_bytes(),
                    );
                    input.extend_from_slice(body);
                    cert.identities.push(Identity {
                        input,
                        signatures: Vec::new(),
                    });
                    component = Some((false, cert.identities.len() - 1));
                }
                Tag::PublicSubkey => {
                    let key = PublicSubkey::try_from_reader(header, body)
                        .map_err(|_| invalid("invalid OpenPGP public subkey"))?;
                    check_key(&key, body)?;
                    if cert.primary.version() != key.version() {
                        return Err(unsupported(
                            "mixed OpenPGP primary and subkey versions are not supported",
                        ));
                    }
                    if key.fingerprint() == cert.primary.fingerprint()
                        || cert
                            .subkeys
                            .iter()
                            .any(|subkey| subkey.key.fingerprint() == key.fingerprint())
                    {
                        return Err(invalid("duplicate OpenPGP subkey fingerprint"));
                    }
                    let input = key_input(body)?;
                    cert.subkeys.push(Subkey {
                        key,
                        input,
                        signatures: Vec::new(),
                    });
                    component = Some((true, cert.subkeys.len() - 1));
                }
                Tag::Signature => {
                    let signature = KeySignature::decode(header, body, limits)?;
                    match (signature.typ(), component) {
                        (SignatureType::Key | SignatureType::KeyRevocation, _)
                        | (SignatureType::CertRevocation, None) => cert.direct.push(signature),
                        (_, Some((true, index))) => cert.subkeys[index].signatures.push(signature),
                        (_, Some((false, index))) => {
                            cert.identities[index].signatures.push(signature)
                        }
                        _ => {
                            return Err(invalid(
                                "OpenPGP certification has no matching key component",
                            ));
                        }
                    }
                }
                _ => return Err(unsupported("unsupported packet in OpenPGP trust material")),
            }
        }
        certificate.ok_or_else(|| invalid("missing OpenPGP primary public key"))
    }

    /// Returns the identity pinned when this certificate is used for authentication.
    pub fn fingerprint(&self) -> Fingerprint {
        self.primary.fingerprint()
    }

    /// Returns the binary public certificate without re-encoding signed packet bodies.
    pub fn as_bytes(&self) -> &[u8] {
        &self.encoded
    }

    /// Authenticates a document using this explicitly trusted primary key or a valid signing subkey.
    ///
    /// Checks the document proof and signature lifetime, then key policy both at signature creation
    /// and at `now`. Uses the newest applicable self-signature; an expired newest signature does
    /// not reactivate older policy. Key flags restrict usage when present. Signing subkeys require
    /// a primary-issued binding and an embedded subkey-issued reverse binding over both keys.
    ///
    /// Valid supplied primary-key and subkey revocations are permanent and apply retroactively.
    /// Certification revocations suppress older certifications; a later certification may replace
    /// them. Only self-issued revocation is supported; designated revokers are rejected explicitly.
    /// No revocation information beyond the supplied certificate is implied.
    pub fn authenticate(
        &self,
        input: &[u8],
        signature: &OpenPgpSignature,
        now: SystemTime,
    ) -> Result<VerifiedSigner> {
        let algorithm = if signature.issuer() == &self.primary.fingerprint() {
            signature.verify_signature(input, &self.primary)?
        } else {
            let subkey = self
                .subkeys
                .iter()
                .find(|subkey| &subkey.key.fingerprint() == signature.issuer())
                .ok_or_else(|| {
                    trust_error("OpenPGP signer is not part of the pinned certificate")
                })?;
            signature.verify_signature(input, &subkey.key)?
        };
        signature.validate_time(now)?;
        self.validate_signer(signature.issuer(), signature.created())?;
        self.validate_signer(signature.issuer(), now)?;
        Ok(VerifiedSigner {
            primary_fingerprint: self.fingerprint(),
            signing_fingerprint: signature.issuer().clone(),
            algorithm,
        })
    }

    /// Checks whether the primary key or named subkey is authorized to sign at `now`.
    ///
    /// Uses the same certification, usage, expiration, and revocation policy as authentication,
    /// without verifying a document. A bare key without a valid self-signature is insufficient.
    pub fn validate_signer(&self, fingerprint: &Fingerprint, now: SystemTime) -> Result<()> {
        let now = now
            .duration_since(UNIX_EPOCH)
            .map_err(|_| trust_error("OpenPGP policy time precedes the Unix epoch"))?
            .as_secs();
        let primary_policy = self.primary_policy(now)?;
        if fingerprint == &self.primary.fingerprint() {
            return primary_policy.usage(false);
        }
        primary_policy.usage(true)?;
        let subkey = self
            .subkeys
            .iter()
            .find(|subkey| &subkey.key.fingerprint() == fingerprint)
            .ok_or_else(|| trust_error("OpenPGP signer is not part of the pinned certificate"))?;
        let (binding, policy) = select(
            &subkey.signatures,
            &self.primary,
            &[&self.primary_input, &subkey.input],
            SignatureType::SubkeyBinding,
            Some(SignatureType::SubkeyRevocation),
            now,
        )?
        .ok_or_else(|| trust_error("OpenPGP signing subkey has no current binding"))?;
        policy.validity(&subkey.key, now)?;
        policy.usage(false)?;
        let mut valid_reverse = false;
        for body in framing::embedded(&binding.body, self.limits)? {
            let reverse = KeySignature::decode(
                PacketHeader::new_fixed(Tag::Signature, body.len() as u32),
                body,
                self.limits,
            )?;
            if reverse.typ() != SignatureType::KeyBinding {
                continue;
            }
            if let Some(policy) =
                reverse.verify(&subkey.key, &[&self.primary_input, &subkey.input])?
                && policy.validity(&subkey.key, now).is_ok()
            {
                valid_reverse = true;
            }
        }
        if !valid_reverse {
            return Err(trust_error(
                "OpenPGP signing subkey lacks a valid reverse binding",
            ));
        }
        Ok(())
    }

    /// Resolves the primary key's current self-certified policy, including identity revocations.
    fn primary_policy(&self, now: u64) -> Result<Policy> {
        let mut latest = select(
            &self.direct,
            &self.primary,
            &[&self.primary_input],
            SignatureType::Key,
            Some(SignatureType::KeyRevocation),
            now,
        )?;
        if self.primary.version() == KeyVersion::V4 {
            for identity in &self.identities {
                if let Some(candidate) = select(
                    &identity.signatures,
                    &self.primary,
                    &[&self.primary_input, &identity.input],
                    SignatureType::CertGeneric,
                    None,
                    now,
                )? {
                    latest = newer(latest, candidate)?;
                }
            }
        }
        let (_, policy) = latest
            .ok_or_else(|| trust_error("OpenPGP primary key has no current self-signature"))?;
        policy.validity(&self.primary, now)?;
        Ok(policy)
    }
}

impl KeySignature {
    /// Decodes a bounded certificate signature while retaining all original protected bytes.
    fn decode(header: PacketHeader, body: &[u8], limits: Limits) -> Result<Self> {
        let protected_end = framing::signature(body, limits)?;
        let mut cursor = Cursor::new(body);
        let signature = Signature::try_from_reader(header, &mut cursor)
            .map_err(|_| invalid("invalid OpenPGP certification signature"))?;
        if cursor.position() != body.len() as u64 {
            return Err(invalid("trailing bytes in OpenPGP certification signature"));
        }
        Ok(Self {
            signature,
            body: body.to_vec(),
            protected_end,
        })
    }

    /// Returns the parsed statement type.
    fn typ(&self) -> SignatureType {
        self.signature.typ().expect("decoded signature")
    }

    /// Verifies a self-issued statement, ignoring unrelated or cryptographically invalid signatures.
    fn verify(&self, key: &impl VerifyingKey, input: &[&[u8]]) -> Result<Option<Policy>> {
        let config = self.signature.config().expect("decoded signature");
        let mut issuer = None;
        for packet in config.hashed_subpackets() {
            if let SubpacketData::IssuerFingerprint(fingerprint) = &packet.data {
                unique(&mut issuer, fingerprint)?;
            }
        }
        if issuer.is_some_and(|fingerprint| fingerprint != &key.fingerprint()) {
            return Ok(None);
        }
        match verify_raw(
            &self.signature,
            &self.body[..self.protected_end],
            input,
            key,
        ) {
            Ok(_) => (),
            Err(error) if error.kind() == ErrorKind::Verification => return Ok(None),
            Err(error) => return Err(error),
        }
        let mut created = None;
        let mut lifetime = None;
        let mut key_lifetime = None;
        let mut flags = None;
        for packet in config.hashed_subpackets() {
            match &packet.data {
                SubpacketData::SignatureCreationTime(value) => {
                    unique(&mut created, u64::from(value.as_secs()))?
                }
                SubpacketData::SignatureExpirationTime(value) => {
                    unique(&mut lifetime, value.as_secs())?
                }
                SubpacketData::KeyExpirationTime(value) => {
                    unique(&mut key_lifetime, value.as_secs())?
                }
                SubpacketData::KeyFlags(value) => unique(&mut flags, value.clone())?,
                SubpacketData::RevocationKey(_) => {
                    return Err(unsupported("OpenPGP designated revokers are not supported"));
                }
                _ => (),
            }
            if packet.is_critical
                && !matches!(
                    packet.data,
                    SubpacketData::SignatureCreationTime(_)
                        | SubpacketData::SignatureExpirationTime(_)
                        | SubpacketData::KeyExpirationTime(_)
                        | SubpacketData::IssuerFingerprint(_)
                        | SubpacketData::IssuerKeyId(_)
                        | SubpacketData::KeyFlags(_)
                        | SubpacketData::RevocationReason(_, _)
                        | SubpacketData::EmbeddedSignature(_)
                        | SubpacketData::PreferredSymmetricAlgorithms(_)
                        | SubpacketData::PreferredHashAlgorithms(_)
                        | SubpacketData::PreferredCompressionAlgorithms(_)
                        | SubpacketData::PreferredAeadAlgorithms(_)
                        | SubpacketData::Features(_)
                        | SubpacketData::IsPrimary(_)
                        | SubpacketData::ExportableCertification(_)
                )
            {
                return Err(invalid(
                    "unsupported critical OpenPGP certification subpacket",
                ));
            }
        }
        let created =
            created.ok_or_else(|| invalid("OpenPGP self-signature lacks hashed creation time"))?;
        if created < u64::from(key.created_at().as_secs()) {
            return Ok(None);
        }
        Ok(Some(Policy {
            created,
            lifetime: lifetime.unwrap_or(0),
            key_lifetime: key_lifetime.unwrap_or(0),
            flags,
        }))
    }
}

impl Policy {
    /// Checks key creation and key/signature expiration with exclusive expiration boundaries.
    fn validity(&self, key: &impl KeyDetails, now: u64) -> Result<()> {
        let created = u64::from(key.created_at().as_secs());
        if created > now || self.created > now || self.created < created {
            return Err(trust_error("OpenPGP key or certification is not yet valid"));
        }
        if (self.lifetime != 0 && now >= self.created + u64::from(self.lifetime))
            || (self.key_lifetime != 0 && now >= created + u64::from(self.key_lifetime))
        {
            return Err(trust_error("OpenPGP key or certification has expired"));
        }
        Ok(())
    }

    /// Enforces signing or certification usage when explicit key flags are present.
    fn usage(&self, certify: bool) -> Result<()> {
        if self.flags.as_ref().is_some_and(|flags| {
            if certify {
                !flags.certify()
            } else {
                !flags.sign()
            }
        }) {
            return Err(trust_error(
                "OpenPGP key flags do not permit the required signing usage",
            ));
        }
        Ok(())
    }
}

/// Selects the latest valid self-issued statement without falling back after its expiration.
fn select<'a>(
    signatures: &'a [KeySignature],
    key: &impl VerifyingKey,
    input: &[&[u8]],
    typ: SignatureType,
    revocation: Option<SignatureType>,
    now: u64,
) -> Result<Option<(&'a KeySignature, Policy)>> {
    let mut latest = None;
    let mut revoked_at = None;
    for signature in signatures {
        let is_statement = signature.typ() == typ
            || (typ == SignatureType::CertGeneric && signature.signature.is_certification());
        if !is_statement
            && Some(signature.typ()) != revocation
            && signature.typ() != SignatureType::CertRevocation
        {
            continue;
        }
        let Some(policy) = signature.verify(key, input)? else {
            continue;
        };
        if Some(signature.typ()) == revocation {
            return Err(trust_error("OpenPGP key has been revoked"));
        }
        if policy.created > now {
            continue;
        }
        if signature.typ() == SignatureType::CertRevocation {
            revoked_at = Some(revoked_at.unwrap_or(0).max(policy.created));
        } else {
            latest = newer(latest, (signature, policy))?;
        }
    }
    Ok(latest.filter(|(_, policy)| revoked_at.is_none_or(|revoked| revoked < policy.created)))
}

/// Chooses the newest statement and rejects contradictory policy issued at the same instant.
fn newer<'a>(
    previous: Option<(&'a KeySignature, Policy)>,
    candidate: (&'a KeySignature, Policy),
) -> Result<Option<(&'a KeySignature, Policy)>> {
    if let Some((_, policy)) = previous.as_ref() {
        if policy.created > candidate.1.created {
            return Ok(previous);
        }
        if policy.created == candidate.1.created && policy != &candidate.1 {
            return Err(trust_error(
                "ambiguous OpenPGP self-signatures at the same creation time",
            ));
        }
    }
    Ok(Some(candidate))
}

/// Requires supported key versions and exact key-body serialization before deriving fingerprints.
fn check_key(key: &(impl KeyDetails + Serialize), body: &[u8]) -> Result<()> {
    if !matches!(key.version(), KeyVersion::V4 | KeyVersion::V6) {
        return Err(unsupported("OpenPGP public keys must use version 4 or 6"));
    }
    if key
        .to_bytes()
        .map_err(|_| invalid("cannot encode OpenPGP public key"))?
        != body
    {
        return Err(invalid("noncanonical OpenPGP public-key body"));
    }
    Ok(())
}

/// Prefixes an exact public-key body for certificate-forming signature hashing.
fn key_input(body: &[u8]) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    match body.first() {
        Some(4) => {
            result.push(0x99);
            result.extend_from_slice(
                &u16::try_from(body.len())
                    .map_err(|_| invalid("version-4 public key exceeds u16"))?
                    .to_be_bytes(),
            );
        }
        Some(6) => {
            result.push(0x9b);
            result.extend_from_slice(
                &u32::try_from(body.len())
                    .map_err(|_| invalid("version-6 public key exceeds u32"))?
                    .to_be_bytes(),
            );
        }
        _ => return Err(unsupported("unsupported OpenPGP public-key version")),
    }
    result.extend_from_slice(body);
    Ok(result)
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Detached CMS signatures with protected algorithms and explicitly pinned signer certificates.

mod revocation;
pub use revocation::RevocationList;

use super::{decode_der, der_error, der_limits, trust, unsupported, verification};
use crate::{Result, binary::Limits, error::invalid};
use ::cms::{
    cert::{CertificateChoices, IssuerAndSerialNumber},
    content_info::{CmsVersion, ContentInfo},
    signed_data::{
        CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo,
        SignerInfos,
    },
};
use der::{
    Any, Decode, Encode, Sequence,
    asn1::{ObjectIdentifier, OctetString, SetOfVec},
};
use pkcs8::{DecodePrivateKey, DecodePublicKey, SecretDocument};
use rsa::traits::PublicKeyParts;
use sha2_legacy::{Digest, Sha256, Sha384, Sha512};
use signature::{RandomizedSigner, SignatureEncoding, Signer, Verifier};
use std::{collections::BTreeSet, fmt, time::SystemTime};
use x509_cert::{
    Certificate,
    attr::{Attribute, Attributes},
    ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectKeyIdentifier},
    spki::AlgorithmIdentifierOwned,
};

/// CMS data content type.
const DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
/// CMS signed-data content type.
const SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
/// Signed content-type attribute.
const CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
/// Signed message-digest attribute.
const MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
/// RFC 6211 algorithm-protection attribute.
const ALGORITHM_PROTECTION: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.52");
/// SHA-256 digest algorithm.
const SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
/// SHA-384 digest algorithm.
const SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
/// SHA-512 digest algorithm.
const SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
/// RSA public keys and PKCS#1 v1.5 CMS signatures.
const RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
/// SHA-256 with RSA PKCS#1 v1.5.
const RSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
/// SHA-512 with RSA PKCS#1 v1.5.
const RSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
/// Elliptic-curve public key algorithm.
const EC: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
/// NIST P-256 named curve.
const P256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
/// NIST P-384 named curve.
const P384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");
/// ECDSA with SHA-256.
const ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
/// ECDSA with SHA-384.
const ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
/// Extended key usage for code signing.
const CODE_SIGNING: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.3");
/// Extended key usage permitting any purpose.
const ANY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.37.0");

/// Supported CMS signing combinations; verification uses the same digest for content and attributes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Algorithm {
    /// RSA PKCS#1 v1.5 with SHA-256; RSA keys must contain 2048 to 8192 bits.
    RsaSha256,
    /// RSA PKCS#1 v1.5 with SHA-512; RSA keys must contain 2048 to 8192 bits.
    RsaSha512,
    /// ECDSA over NIST P-256 with SHA-256.
    EcdsaP256Sha256,
    /// ECDSA over NIST P-384 with SHA-384.
    EcdsaP384Sha384,
}

impl Algorithm {
    /// Returns the content digest identifier with absent parameters.
    fn digest_algorithm(self) -> AlgorithmIdentifierOwned {
        identifier(
            match self {
                Self::RsaSha256 | Self::EcdsaP256Sha256 => SHA256,
                Self::RsaSha512 => SHA512,
                Self::EcdsaP384Sha384 => SHA384,
            },
            false,
        )
    }

    /// Returns the CMS signature identifier, including required RSA NULL parameters.
    fn signature_algorithm(self) -> AlgorithmIdentifierOwned {
        match self {
            Self::RsaSha256 | Self::RsaSha512 => identifier(RSA, true),
            Self::EcdsaP256Sha256 => identifier(ECDSA_SHA256, false),
            Self::EcdsaP384Sha384 => identifier(ECDSA_SHA384, false),
        }
    }

    /// Computes a digest over exactly the supplied detached content bytes.
    fn digest(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::RsaSha256 | Self::EcdsaP256Sha256 => Sha256::digest(bytes).to_vec(),
            Self::RsaSha512 => Sha512::digest(bytes).to_vec(),
            Self::EcdsaP384Sha384 => Sha384::digest(bytes).to_vec(),
        }
    }
}

/// An explicitly supplied X.509 certificate, independent of chain or publisher trust.
#[derive(Clone, Debug)]
pub struct SignerCertificate {
    /// Validated canonical certificate structure.
    certificate: Certificate,
    /// SHA-256 over the complete certificate DER, used as a stable identity.
    fingerprint: [u8; 32],
}

impl SignerCertificate {
    /// Reads one DER or PEM certificate under the supplied format limits.
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        limits.bytes(bytes.len() as u64)?;
        let der = pem_or_der(bytes, "CERTIFICATE")?;
        let certificate = decode_der(&der, limits)?;
        Ok(Self {
            certificate,
            fingerprint: Sha256::digest(&der).into(),
        })
    }

    /// Returns the SHA-256 fingerprint of the complete certificate DER.
    pub fn fingerprint(&self) -> &[u8; 32] {
        &self.fingerprint
    }

    /// Encodes this certificate as canonical DER.
    pub fn to_der(&self) -> Result<Vec<u8>> {
        self.certificate.to_der().map_err(der_error)
    }

    /// Selects SHA-256 for RSA or P-256, and SHA-384 for P-384.
    pub fn preferred_algorithm(&self) -> Result<Algorithm> {
        Ok(match public_key(&self.certificate)? {
            PublicKey::Rsa(_) => Algorithm::RsaSha256,
            PublicKey::P256(_) => Algorithm::EcdsaP256Sha256,
            PublicKey::P384(_) => Algorithm::EcdsaP384Sha384,
        })
    }

    /// Checks validity at `now`, signing usage, supported key strength, and critical extensions.
    ///
    /// Direct certificate pinning does not validate an issuer chain or establish revocation status.
    /// If KeyUsage is present, digitalSignature must be set. If ExtendedKeyUsage is present,
    /// it must permit code signing or any purpose. Unknown critical extensions are rejected.
    pub fn validate(&self, now: SystemTime) -> Result<()> {
        let tbs = &self.certificate.tbs_certificate;
        if now < tbs.validity.not_before.to_system_time()
            || now > tbs.validity.not_after.to_system_time()
        {
            return Err(trust(
                "CMS signer certificate is not valid at the verification time",
            ));
        }
        let mut seen = BTreeSet::new();
        for extension in tbs.extensions.as_deref().unwrap_or_default() {
            if !seen.insert(extension.extn_id) {
                return Err(invalid("duplicate certificate extension"));
            }
            let oid = extension.extn_id.to_string();
            if extension.critical
                && !matches!(oid.as_str(), "2.5.29.15" | "2.5.29.19" | "2.5.29.37")
            {
                return Err(unsupported(format!(
                    "unsupported critical certificate extension: {oid}"
                )));
            }
        }
        if let Some((_, usage)) = tbs.get::<KeyUsage>().map_err(der_error)?
            && !usage.digital_signature()
        {
            return Err(trust(
                "CMS signer certificate does not permit digital signatures",
            ));
        }
        if let Some((_, usage)) = tbs.get::<ExtendedKeyUsage>().map_err(der_error)?
            && !usage.0.contains(&CODE_SIGNING)
            && !usage.0.contains(&ANY_USAGE)
        {
            return Err(trust("CMS signer certificate does not permit code signing"));
        }
        tbs.get::<BasicConstraints>().map_err(der_error)?;
        public_key(&self.certificate)?;
        Ok(())
    }
}

/// A decoded detached CMS envelope; construction alone makes no authentication claim.
#[derive(Clone, Debug)]
pub struct CmsSignature {
    /// Canonical CMS structure, with original detached document bytes supplied only for verification.
    data: SignedData,
}

impl CmsSignature {
    /// Reads exactly one canonical DER ContentInfo containing detached SignedData for id-data.
    ///
    /// Signature attributes, cryptographic validity, and certificate policy are checked separately.
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        let info: ContentInfo = decode_der(bytes, limits)?;
        if info.content_type != SIGNED_DATA {
            return Err(invalid("CMS content type is not signedData"));
        }
        let data_bytes = info.content.to_der().map_err(der_error)?;
        let data: SignedData = decode_der(&data_bytes, limits)?;
        if data.encap_content_info.econtent_type != DATA
            || data.encap_content_info.econtent.is_some()
        {
            return Err(invalid(
                "CMS signature must contain detached id-data content",
            ));
        }
        if data.signer_infos.0.is_empty() {
            return Err(invalid("CMS signerInfos is empty"));
        }
        validate_versions(&data)?;
        Ok(Self { data })
    }

    /// Verifies at least one matching SignerInfo under the Janex CMS profile using `certificate`.
    ///
    /// Checks the signer identifier, signed content digest, protected algorithms, key strength,
    /// and cryptographic signature. It does not check certificate time, usage, revocation, or trust.
    pub fn verify_signature(
        &self,
        input: &[u8],
        certificate: &SignerCertificate,
    ) -> Result<Algorithm> {
        let mut failure = None;
        for signer in self.data.signer_infos.0.iter() {
            if !matches_certificate(signer, &certificate.certificate)? {
                continue;
            }
            match verify_signer(&self.data, signer, input, &certificate.certificate) {
                Ok(algorithm) => return Ok(algorithm),
                Err(error) => failure = Some(error),
            }
        }
        Err(failure.unwrap_or_else(|| trust("required CMS signer is missing")))
    }

    /// Verifies every explicitly required certificate and its time and code-signing usage.
    ///
    /// An empty set or duplicate certificate pin is an error. Embedded certificates do not
    /// establish trust. Revocation status must additionally be established by the caller.
    pub fn verify_pinned(
        &self,
        input: &[u8],
        required: &[SignerCertificate],
        now: SystemTime,
    ) -> Result<Vec<VerifiedSigner>> {
        if required.is_empty() {
            return Err(trust(
                "CMS authentication requires an explicit signer certificate",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        for certificate in required {
            if !seen.insert(certificate.fingerprint) {
                return Err(trust("duplicate required CMS signer certificate"));
            }
            certificate.validate(now)?;
            let algorithm = self.verify_signature(input, certificate)?;
            result.push(VerifiedSigner {
                fingerprint: certificate.fingerprint,
                algorithm,
                revocation_lists_checked: 0,
            });
        }
        Ok(result)
    }

    /// Authenticates explicit signer pins and checks supplied or embedded complete direct CRLs.
    ///
    /// Issuer certificates are used only when their keys verify the pinned signer certificate
    /// and the matching CRL. They do not introduce additional trusted signers. Matching lists
    /// must be current and valid; revoked signers fail. No list means no revocation assertion.
    /// Delta, partitioned, and indirect CRLs are unsupported. No network or system store is used.
    pub fn authenticate(
        &self,
        input: &[u8],
        required: &[SignerCertificate],
        issuers: &[SignerCertificate],
        crls: &[RevocationList],
        now: SystemTime,
    ) -> Result<Vec<VerifiedSigner>> {
        let mut result = self.verify_pinned(input, required, now)?;
        for (certificate, verified) in required.iter().zip(&mut result) {
            verified.revocation_lists_checked =
                revocation::check(&self.data, certificate, issuers, crls, now)?;
        }
        Ok(result)
    }
}

/// Cryptographically verified signer identity after explicit certificate pin and validity checks.
#[derive(Clone, Debug)]
pub struct VerifiedSigner {
    /// SHA-256 fingerprint of the caller-supplied certificate.
    pub fingerprint: [u8; 32],
    /// Validated signature and digest combination.
    pub algorithm: Algorithm,
    /// Number of current authenticated CRLs checked; zero makes no revocation-status assertion.
    pub revocation_lists_checked: usize,
}

/// A private signing key paired with its matching certificate; Debug never exposes key material.
pub struct CmsSigner {
    /// Caller-supplied matching certificate.
    certificate: SignerCertificate,
    /// Requested signing combination.
    algorithm: Algorithm,
    /// Owned secret key, zeroized by its underlying cryptographic type on drop.
    key: PrivateKey,
}

impl fmt::Debug for CmsSigner {
    /// Displays only the public algorithm choice.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CmsSigner")
            .field("algorithm", &self.algorithm)
            .finish_non_exhaustive()
    }
}

impl CmsSigner {
    /// Returns whether a DER or PEM PKCS#8 document is encrypted, without decrypting it.
    pub fn key_is_encrypted(key: &[u8], limits: Limits) -> Result<bool> {
        let document = key_document(key, limits)?;
        if let Ok(encrypted) = pkcs8::EncryptedPrivateKeyInfo::from_der(document.as_bytes()) {
            check_key_derivation(&encrypted)?;
            Ok(true)
        } else {
            pkcs8::PrivateKeyInfo::from_der(document.as_bytes())
                .map_err(|_| invalid("invalid PKCS#8 key"))?;
            Ok(false)
        }
    }

    /// Loads an unencrypted or encrypted PKCS#8 key in DER or PEM and checks its certificate match.
    ///
    /// Encrypted keys require `password`; passwords and decrypted key bytes are never included
    /// in errors. Certificate validity and usage are checked when signing at the supplied time.
    /// PBKDF2 is limited to 10 million iterations. Scrypt is limited to 256 MiB of working
    /// buffers and `N * r * p <= 2^24`; excessive costs fail before key derivation starts.
    pub fn from_pkcs8(
        certificate: SignerCertificate,
        key: &[u8],
        password: Option<&[u8]>,
        algorithm: Algorithm,
        limits: Limits,
    ) -> Result<Self> {
        let document = key_document(key, limits)?;
        let document = if let Ok(encrypted) =
            pkcs8::EncryptedPrivateKeyInfo::from_der(document.as_bytes())
        {
            check_key_derivation(&encrypted)?;
            encrypted
                .decrypt(password.ok_or_else(|| trust("encrypted PKCS#8 key requires a password"))?)
                .map_err(|_| trust("cannot decrypt PKCS#8 key"))?
        } else {
            document
        };
        let key = match algorithm {
            Algorithm::RsaSha256 | Algorithm::RsaSha512 => PrivateKey::Rsa(Box::new(
                rsa::RsaPrivateKey::from_pkcs8_der(document.as_bytes())
                    .map_err(|_| invalid("invalid RSA PKCS#8 key"))?,
            )),
            Algorithm::EcdsaP256Sha256 => PrivateKey::P256(
                p256::ecdsa::SigningKey::from_pkcs8_der(document.as_bytes())
                    .map_err(|_| invalid("invalid P-256 PKCS#8 key"))?,
            ),
            Algorithm::EcdsaP384Sha384 => PrivateKey::P384(
                p384::ecdsa::SigningKey::from_pkcs8_der(document.as_bytes())
                    .map_err(|_| invalid("invalid P-384 PKCS#8 key"))?,
            ),
        };
        let public = public_key(&certificate.certificate)?;
        let matches = match (&key, &public) {
            (PrivateKey::Rsa(secret), PublicKey::Rsa(public)) => {
                rsa::RsaPublicKey::from(secret.as_ref()) == *public
            }
            (PrivateKey::P256(secret), PublicKey::P256(public)) => secret.verifying_key() == public,
            (PrivateKey::P384(secret), PublicKey::P384(public)) => secret.verifying_key() == public,
            _ => false,
        };
        if !matches {
            return Err(trust(
                "CMS private key does not match its signer certificate",
            ));
        }
        Ok(Self {
            certificate,
            algorithm,
            key,
        })
    }

    /// Returns the matching public certificate.
    pub fn certificate(&self) -> &SignerCertificate {
        &self.certificate
    }

    /// Builds a SignerInfo over content-type, message-digest, and algorithm-protection attributes.
    fn sign_info(&self, input: &[u8], now: SystemTime) -> Result<SignerInfo> {
        self.certificate.validate(now)?;
        let protection = AlgorithmProtection {
            digest_algorithm: self.algorithm.digest_algorithm(),
            signature_algorithm: Some(self.algorithm.signature_algorithm()),
            mac_algorithm: None,
        };
        let attributes = SetOfVec::try_from(vec![
            attribute(CONTENT_TYPE, Any::encode_from(&DATA).map_err(der_error)?)?,
            attribute(
                MESSAGE_DIGEST,
                Any::encode_from(
                    &OctetString::new(self.algorithm.digest(input)).map_err(der_error)?,
                )
                .map_err(der_error)?,
            )?,
            attribute(
                ALGORITHM_PROTECTION,
                Any::encode_from(&protection).map_err(der_error)?,
            )?,
        ])
        .map_err(der_error)?;
        let bytes = attributes.to_der().map_err(der_error)?;
        let signature = match (&self.key, self.algorithm) {
            (PrivateKey::Rsa(key), Algorithm::RsaSha256) => {
                rsa::pkcs1v15::SigningKey::<Sha256>::new(key.as_ref().clone())
                    .try_sign_with_rng(&mut rand::rngs::OsRng, &bytes)
                    .map_err(|_| verification("RSA signing failed"))?
                    .to_vec()
            }
            (PrivateKey::Rsa(key), Algorithm::RsaSha512) => {
                rsa::pkcs1v15::SigningKey::<Sha512>::new(key.as_ref().clone())
                    .try_sign_with_rng(&mut rand::rngs::OsRng, &bytes)
                    .map_err(|_| verification("RSA signing failed"))?
                    .to_vec()
            }
            (PrivateKey::P256(key), Algorithm::EcdsaP256Sha256) => {
                let signature: p256::ecdsa::Signature = key
                    .try_sign(&bytes)
                    .map_err(|_| verification("P-256 signing failed"))?;
                signature.to_der().as_bytes().to_vec()
            }
            (PrivateKey::P384(key), Algorithm::EcdsaP384Sha384) => {
                let signature: p384::ecdsa::Signature = key
                    .try_sign(&bytes)
                    .map_err(|_| verification("P-384 signing failed"))?;
                signature.to_der().as_bytes().to_vec()
            }
            _ => unreachable!("constructor checks key and algorithm"),
        };
        let tbs = &self.certificate.certificate.tbs_certificate;
        Ok(SignerInfo {
            version: CmsVersion::V1,
            sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: tbs.issuer.clone(),
                serial_number: tbs.serial_number.clone(),
            }),
            digest_alg: self.algorithm.digest_algorithm(),
            signed_attrs: Some(attributes),
            signature_algorithm: self.algorithm.signature_algorithm(),
            signature: OctetString::new(signature).map_err(der_error)?,
            unsigned_attrs: None,
        })
    }
}

/// Bounds password-derived work and memory before invoking the cryptographic library.
fn check_key_derivation(key: &pkcs8::EncryptedPrivateKeyInfo<'_>) -> Result<()> {
    let parameters = key
        .encryption_algorithm
        .pbes2()
        .ok_or_else(|| unsupported("encrypted PKCS#8 keys require PBES2"))?;
    let excessive = match &parameters.kdf {
        pkcs8::pkcs5::pbes2::Kdf::Pbkdf2(params) => params.iteration_count > 10_000_000,
        pkcs8::pkcs5::pbes2::Kdf::Scrypt(params) => {
            let n = params.cost_parameter;
            let r = u64::from(params.block_size);
            let p = u64::from(params.parallelization);
            let memory = n
                .checked_add(p)
                .and_then(|sum| sum.checked_add(2))
                .and_then(|sum| sum.checked_mul(r))
                .and_then(|size| size.checked_mul(128));
            let work = n.checked_mul(r).and_then(|work| work.checked_mul(p));
            memory.is_none_or(|size| size > 256 * 1024 * 1024)
                || work.is_none_or(|work| work > 1 << 24)
        }
        _ => return Err(unsupported("unsupported PKCS#8 key derivation function")),
    };
    if excessive {
        return Err(crate::Error::new(
            crate::ErrorKind::Limit,
            "PKCS#8 key derivation cost limit exceeded",
        ));
    }
    Ok(())
}

/// Decodes a bounded secret document, retaining zeroization for its DER storage.
fn key_document(key: &[u8], limits: Limits) -> Result<SecretDocument> {
    limits.bytes(key.len() as u64)?;
    let document = if key.starts_with(b"-----BEGIN ") {
        let (label, document) = SecretDocument::from_pem(
            std::str::from_utf8(key).map_err(|_| invalid("PKCS#8 PEM is not UTF-8"))?,
        )
        .map_err(der_error)?;
        if !matches!(label, "PRIVATE KEY" | "ENCRYPTED PRIVATE KEY") {
            return Err(invalid("expected a PKCS#8 private key"));
        }
        document
    } else {
        SecretDocument::try_from(key).map_err(der_error)?
    };
    der_limits(document.as_bytes(), limits)?;
    Ok(document)
}

/// Creates one detached CMS envelope containing a signature from every supplied signer.
///
/// Signer certificates must be distinct and valid at `now`. Every signer covers the same exact
/// input. Certificates are embedded for identification; embedding them does not establish trust.
pub fn sign(input: &[u8], signers: &[&CmsSigner], now: SystemTime) -> Result<Vec<u8>> {
    if signers.is_empty() {
        return Err(invalid("CMS signing requires at least one signer"));
    }
    let mut identities = BTreeSet::new();
    let mut infos = Vec::new();
    let mut certificates = Vec::new();
    let mut digests = SetOfVec::new();
    for signer in signers {
        if !identities.insert(signer.certificate.fingerprint) {
            return Err(invalid("duplicate CMS signer certificate"));
        }
        infos.push(signer.sign_info(input, now)?);
        certificates.push(CertificateChoices::Certificate(
            signer.certificate.certificate.clone(),
        ));
        let digest = signer.algorithm.digest_algorithm();
        if !digests.iter().any(|existing| existing == &digest) {
            digests.insert(digest).map_err(der_error)?;
        }
    }
    let data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: digests,
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: DATA,
            econtent: None,
        },
        certificates: Some(CertificateSet(
            SetOfVec::try_from(certificates).map_err(der_error)?,
        )),
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(infos).map_err(der_error)?),
    };
    ContentInfo {
        content_type: SIGNED_DATA,
        content: Any::encode_from(&data).map_err(der_error)?,
    }
    .to_der()
    .map_err(der_error)
}

/// Supported secret-key representations.
enum PrivateKey {
    /// RSA key; boxed to avoid inflating the elliptic-curve variants.
    Rsa(Box<rsa::RsaPrivateKey>),
    /// NIST P-256 signing key.
    P256(p256::ecdsa::SigningKey),
    /// NIST P-384 signing key.
    P384(p384::ecdsa::SigningKey),
}

/// Parsed public keys after algorithm-parameter and strength checks.
enum PublicKey {
    /// RSA public key between 2048 and 8192 bits.
    Rsa(rsa::RsaPublicKey),
    /// NIST P-256 verification key.
    P256(p256::ecdsa::VerifyingKey),
    /// NIST P-384 verification key.
    P384(p384::ecdsa::VerifyingKey),
}

/// RFC 6211 algorithm protection, with exactly one signature algorithm and no MAC algorithm.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct AlgorithmProtection {
    /// Content and attribute digest algorithm.
    digest_algorithm: AlgorithmIdentifierOwned,
    /// Authenticated signature algorithm.
    #[asn1(context_specific = "1", tag_mode = "IMPLICIT", optional = "true")]
    signature_algorithm: Option<AlgorithmIdentifierOwned>,
    /// MAC algorithm, forbidden for SignedData.
    #[asn1(context_specific = "2", tag_mode = "IMPLICIT", optional = "true")]
    mac_algorithm: Option<AlgorithmIdentifierOwned>,
}

/// Validates CMS version numbers against the encoded certificate and signer choices.
fn validate_versions(data: &SignedData) -> Result<()> {
    let other = data.certificates.as_ref().is_some_and(|certs| {
        certs
            .0
            .iter()
            .any(|cert| matches!(cert, CertificateChoices::Other(_)))
    }) || data.crls.as_ref().is_some_and(|crls| {
        crls.0
            .iter()
            .any(|crl| matches!(crl, ::cms::revocation::RevocationInfoChoice::Other(_)))
    });
    let mut version = if other {
        CmsVersion::V5
    } else {
        CmsVersion::V1
    };
    for signer in data.signer_infos.0.iter() {
        let expected = match signer.sid {
            SignerIdentifier::IssuerAndSerialNumber(_) => CmsVersion::V1,
            SignerIdentifier::SubjectKeyIdentifier(_) => CmsVersion::V3,
        };
        if signer.version != expected {
            return Err(invalid(
                "CMS SignerInfo version does not match its identifier",
            ));
        }
        version = version.max(expected);
    }
    if data.version != version {
        return Err(invalid(
            "CMS SignedData version does not match its contents",
        ));
    }
    Ok(())
}

/// Matches the declared signer ID against an explicit certificate without using embedded trust.
fn matches_certificate(signer: &SignerInfo, certificate: &Certificate) -> Result<bool> {
    let tbs = &certificate.tbs_certificate;
    Ok(match &signer.sid {
        SignerIdentifier::IssuerAndSerialNumber(id) => {
            id.issuer == tbs.issuer && id.serial_number == tbs.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(id) => tbs
            .get::<SubjectKeyIdentifier>()
            .map_err(der_error)?
            .is_some_and(|(_, ski)| ski == *id),
    })
}

/// Checks signed attributes and verifies the signature over their canonical DER SET encoding.
fn verify_signer(
    data: &SignedData,
    signer: &SignerInfo,
    input: &[u8],
    certificate: &Certificate,
) -> Result<Algorithm> {
    let attributes = signer
        .signed_attrs
        .as_ref()
        .ok_or_else(|| invalid("CMS signer has no signed attributes"))?;
    let content_type = required_attribute(attributes, CONTENT_TYPE)?
        .decode_as::<ObjectIdentifier>()
        .map_err(der_error)?;
    if content_type != DATA {
        return Err(invalid("CMS signed content-type is not id-data"));
    }
    let protection = required_attribute(attributes, ALGORITHM_PROTECTION)?
        .decode_as::<AlgorithmProtection>()
        .map_err(der_error)?;
    if !same_algorithm(&protection.digest_algorithm, &signer.digest_alg)
        || protection.mac_algorithm.is_some()
        || !protection
            .signature_algorithm
            .as_ref()
            .is_some_and(|algorithm| same_algorithm(algorithm, &signer.signature_algorithm))
    {
        return Err(invalid("CMS protected algorithms do not match SignerInfo"));
    }
    if !data
        .digest_algorithms
        .iter()
        .any(|algorithm| same_algorithm(algorithm, &signer.digest_alg))
    {
        return Err(invalid("CMS signer digest is absent from digestAlgorithms"));
    }
    let key = public_key(certificate)?;
    let algorithm = signing_algorithm(&signer.digest_alg, &signer.signature_algorithm, &key)?;
    let digest = required_attribute(attributes, MESSAGE_DIGEST)?
        .decode_as::<OctetString>()
        .map_err(der_error)?;
    if digest.as_bytes() != algorithm.digest(input) {
        return Err(verification("CMS signed content digest mismatch"));
    }
    let bytes = attributes.to_der().map_err(der_error)?;
    verify_bytes(&key, algorithm, &bytes, signer.signature.as_bytes())?;
    Ok(algorithm)
}

/// Selects a supported digest/signature/key combination, rejecting forbidden parameters.
fn signing_algorithm(
    digest: &AlgorithmIdentifierOwned,
    signature: &AlgorithmIdentifierOwned,
    key: &PublicKey,
) -> Result<Algorithm> {
    if !null_or_absent(&digest.parameters) {
        return Err(invalid("unexpected CMS digest parameters"));
    }
    match key {
        PublicKey::Rsa(_) if null_or_absent(&signature.parameters) => {
            match (digest.oid, signature.oid) {
                (SHA256, RSA | RSA_SHA256) => return Ok(Algorithm::RsaSha256),
                (SHA512, RSA | RSA_SHA512) => return Ok(Algorithm::RsaSha512),
                _ => {}
            }
        }
        PublicKey::P256(_)
            if signature.parameters.is_none()
                && digest.oid == SHA256
                && signature.oid == ECDSA_SHA256 =>
        {
            return Ok(Algorithm::EcdsaP256Sha256);
        }
        PublicKey::P384(_)
            if signature.parameters.is_none()
                && digest.oid == SHA384
                && signature.oid == ECDSA_SHA384 =>
        {
            return Ok(Algorithm::EcdsaP384Sha384);
        }
        _ => {}
    }
    Err(unsupported(
        "unsupported CMS signature algorithm, digest, or key combination",
    ))
}

/// Applies a supported cryptographic verifier to the exact supplied signature input.
fn verify_bytes(
    key: &PublicKey,
    algorithm: Algorithm,
    input: &[u8],
    signature: &[u8],
) -> Result<()> {
    let valid = match (key, algorithm) {
        (PublicKey::Rsa(key), Algorithm::RsaSha256) => {
            rsa::pkcs1v15::Signature::try_from(signature)
                .ok()
                .is_some_and(|signature| {
                    rsa::pkcs1v15::VerifyingKey::<Sha256>::new(key.clone())
                        .verify(input, &signature)
                        .is_ok()
                })
        }
        (PublicKey::Rsa(key), Algorithm::RsaSha512) => {
            rsa::pkcs1v15::Signature::try_from(signature)
                .ok()
                .is_some_and(|signature| {
                    rsa::pkcs1v15::VerifyingKey::<Sha512>::new(key.clone())
                        .verify(input, &signature)
                        .is_ok()
                })
        }
        (PublicKey::P256(key), Algorithm::EcdsaP256Sha256) => {
            p256::ecdsa::Signature::from_der(signature)
                .ok()
                .is_some_and(|signature| key.verify(input, &signature).is_ok())
        }
        (PublicKey::P384(key), Algorithm::EcdsaP384Sha384) => {
            p384::ecdsa::Signature::from_der(signature)
                .ok()
                .is_some_and(|signature| key.verify(input, &signature).is_ok())
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(verification("CMS cryptographic signature mismatch"))
    }
}

/// Parses a certificate public key and enforces supported parameters and RSA bounds.
fn public_key(certificate: &Certificate) -> Result<PublicKey> {
    let spki = &certificate.tbs_certificate.subject_public_key_info;
    let bytes = spki.to_der().map_err(der_error)?;
    if spki.algorithm.oid == RSA && null_or_absent(&spki.algorithm.parameters) {
        let key = rsa::RsaPublicKey::from_public_key_der(&bytes)
            .map_err(|_| invalid("invalid RSA certificate public key"))?;
        if !(2048..=8192).contains(&key.n().bits()) {
            return Err(trust("RSA key strength must be between 2048 and 8192 bits"));
        }
        return Ok(PublicKey::Rsa(key));
    }
    if spki.algorithm.oid == EC {
        let curve = spki
            .algorithm
            .parameters
            .as_ref()
            .ok_or_else(|| invalid("EC public key omits its named curve"))?
            .decode_as::<ObjectIdentifier>()
            .map_err(der_error)?;
        if curve == P256 {
            return p256::ecdsa::VerifyingKey::from_public_key_der(&bytes)
                .map(PublicKey::P256)
                .map_err(|_| invalid("invalid P-256 certificate public key"));
        }
        if curve == P384 {
            return p384::ecdsa::VerifyingKey::from_public_key_der(&bytes)
                .map(PublicKey::P384)
                .map_err(|_| invalid("invalid P-384 certificate public key"));
        }
    }
    Err(unsupported(
        "unsupported CMS certificate public key algorithm",
    ))
}

/// Finds exactly one signed attribute with exactly one value.
fn required_attribute(attributes: &Attributes, oid: ObjectIdentifier) -> Result<&Any> {
    let mut matches = attributes.iter().filter(|attribute| attribute.oid == oid);
    let attribute = matches
        .next()
        .ok_or_else(|| invalid(format!("missing CMS signed attribute: {oid}")))?;
    if matches.next().is_some() || attribute.values.len() != 1 {
        return Err(invalid(format!(
            "duplicate or multivalued CMS signed attribute: {oid}"
        )));
    }
    attribute
        .values
        .get(0)
        .ok_or_else(|| invalid("empty CMS signed attribute"))
}

/// Creates a signed attribute with a single ASN.1 value.
fn attribute(oid: ObjectIdentifier, value: Any) -> Result<Attribute> {
    Ok(Attribute {
        oid,
        values: SetOfVec::try_from(vec![value]).map_err(der_error)?,
    })
}

/// Creates an algorithm identifier with explicit NULL or absent parameters.
fn identifier(oid: ObjectIdentifier, null: bool) -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned {
        oid,
        parameters: null.then(Any::null),
    }
}

/// Tests parameters permitted for digest algorithms and RSA PKCS#1 v1.5 identifiers.
fn null_or_absent(parameters: &Option<Any>) -> bool {
    parameters.as_ref().is_none_or(Any::is_null)
}

/// Compares protected algorithms, treating NULL and absence equally only where supported.
fn same_algorithm(a: &AlgorithmIdentifierOwned, b: &AlgorithmIdentifierOwned) -> bool {
    a == b
        || (a.oid == b.oid
            && matches!(
                a.oid,
                SHA256 | SHA384 | SHA512 | RSA | RSA_SHA256 | RSA_SHA512
            )
            && null_or_absent(&a.parameters)
            && null_or_absent(&b.parameters))
}

/// Decodes a public PEM value or retains the supplied DER bytes.
fn pem_or_der(bytes: &[u8], expected: &str) -> Result<Vec<u8>> {
    if bytes.starts_with(b"-----BEGIN ") {
        let (label, der) =
            der::pem::decode_vec(bytes).map_err(|_| invalid("invalid PEM encoding"))?;
        if label != expected {
            return Err(invalid(format!("expected PEM {expected}")));
        }
        Ok(der)
    } else {
        Ok(bytes.into())
    }
}

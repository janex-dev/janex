// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use sha2::{Digest, Sha256, Sha512};
use sm3::Sm3;
use xxhash_rust::xxh64::xxh64;

/// Static checksum algorithm contract used by concrete checksum structs.
pub trait Checksum<const SIZE: usize>:
    Sized + std::fmt::Debug + Clone + Copy + PartialEq + Eq
{
    /// The Janex `ChecksumAlgorithm` identifier stored in the binary format.
    const ALGORITHM_ID: u16;

    /// Creates the checksum wrapper from its fixed-size raw byte array.
    fn from_array(bytes: [u8; SIZE]) -> Self;

    /// Returns the checksum as its fixed-size raw byte array.
    fn as_array(&self) -> &[u8; SIZE];

    /// Computes the checksum over the provided bytes.
    fn compute(bytes: &[u8]) -> Self;

    /// Parses the checksum from a byte slice and validates its length.
    fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let expected = SIZE as u64;
        let actual = bytes.len() as u64;
        let bytes = <[u8; SIZE]>::try_from(bytes)
            .map_err(|_| Error::InvalidChecksumLength { expected, actual })?;
        Ok(Self::from_array(bytes))
    }

    /// Returns the checksum as an untyped byte slice.
    fn as_bytes(&self) -> &[u8] {
        self.as_array()
    }

    /// Wraps the concrete checksum value in `AnyChecksum`.
    fn to_any(self) -> AnyChecksum
    where
        Self: Into<AnyChecksum>,
    {
        self.into()
    }
}

/// Marker type for sections or resources without a checksum payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoChecksum;

impl Checksum<0> for NoChecksum {
    const ALGORITHM_ID: u16 = 0;

    fn from_array(_: [u8; 0]) -> Self {
        Self
    }

    fn as_array(&self) -> &[u8; 0] {
        static EMPTY: [u8; 0] = [];
        &EMPTY
    }

    fn compute(_: &[u8]) -> Self {
        Self
    }
}

/// XXH64 checksum payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Xxh64Checksum([u8; 8]);

impl Xxh64Checksum {
    /// Creates an XXH64 checksum from its raw bytes.
    pub const fn new(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }
}

impl Checksum<8> for Xxh64Checksum {
    const ALGORITHM_ID: u16 = 0x0101;

    fn from_array(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    fn as_array(&self) -> &[u8; 8] {
        &self.0
    }

    fn compute(bytes: &[u8]) -> Self {
        Self(xxh64(bytes, 0).to_le_bytes())
    }
}

/// SHA-256 checksum payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sha256Checksum([u8; 32]);

impl Sha256Checksum {
    /// Creates a SHA-256 checksum from its raw bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl Checksum<32> for Sha256Checksum {
    const ALGORITHM_ID: u16 = 0x8101;

    fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn as_array(&self) -> &[u8; 32] {
        &self.0
    }

    fn compute(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

/// SHA-512 checksum payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sha512Checksum([u8; 64]);

impl Sha512Checksum {
    /// Creates a SHA-512 checksum from its raw bytes.
    pub const fn new(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }
}

impl Default for Sha512Checksum {
    fn default() -> Self {
        Self([0; 64])
    }
}

impl Checksum<64> for Sha512Checksum {
    const ALGORITHM_ID: u16 = 0x8102;

    fn from_array(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    fn as_array(&self) -> &[u8; 64] {
        &self.0
    }

    fn compute(bytes: &[u8]) -> Self {
        Self(Sha512::digest(bytes).into())
    }
}

/// SM3 checksum payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sm3Checksum([u8; 32]);

impl Sm3Checksum {
    /// Creates an SM3 checksum from its raw bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl Checksum<32> for Sm3Checksum {
    const ALGORITHM_ID: u16 = 0x8301;

    fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn as_array(&self) -> &[u8; 32] {
        &self.0
    }

    fn compute(bytes: &[u8]) -> Self {
        Self(Sm3::digest(bytes).into())
    }
}

/// Runtime wrapper used by the Janex format, which carries the chosen checksum algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnyChecksum {
    /// No checksum bytes are stored.
    None(NoChecksum),
    /// XXH64 checksum bytes.
    XXH64(Xxh64Checksum),
    /// SHA-256 checksum bytes.
    SHA256(Sha256Checksum),
    /// SHA-512 checksum bytes.
    SHA512(Sha512Checksum),
    /// SM3 checksum bytes.
    SM3(Sm3Checksum),
}

impl AnyChecksum {
    /// Returns the raw Janex algorithm identifier used by the binary format.
    pub const fn algorithm_id(&self) -> u16 {
        match self {
            AnyChecksum::None(_) => NoChecksum::ALGORITHM_ID,
            AnyChecksum::XXH64(_) => Xxh64Checksum::ALGORITHM_ID,
            AnyChecksum::SHA256(_) => Sha256Checksum::ALGORITHM_ID,
            AnyChecksum::SHA512(_) => Sha512Checksum::ALGORITHM_ID,
            AnyChecksum::SM3(_) => Sm3Checksum::ALGORITHM_ID,
        }
    }

    /// Returns the checksum bytes without exposing format-specific tagging.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            AnyChecksum::None(checksum) => checksum.as_bytes(),
            AnyChecksum::XXH64(checksum) => checksum.as_bytes(),
            AnyChecksum::SHA256(checksum) => checksum.as_bytes(),
            AnyChecksum::SHA512(checksum) => checksum.as_bytes(),
            AnyChecksum::SM3(checksum) => checksum.as_bytes(),
        }
    }

    /// Parses a checksum from its raw algorithm tag and payload bytes.
    pub fn from_raw(algorithm: u16, checksum: &[u8]) -> Result<Self, Error> {
        match algorithm {
            NoChecksum::ALGORITHM_ID => Ok(Self::None(NoChecksum::from_bytes(checksum)?)),
            Xxh64Checksum::ALGORITHM_ID => Ok(Self::XXH64(Xxh64Checksum::from_bytes(checksum)?)),
            Sha256Checksum::ALGORITHM_ID => Ok(Self::SHA256(Sha256Checksum::from_bytes(checksum)?)),
            Sha512Checksum::ALGORITHM_ID => Ok(Self::SHA512(Sha512Checksum::from_bytes(checksum)?)),
            Sm3Checksum::ALGORITHM_ID => Ok(Self::SM3(Sm3Checksum::from_bytes(checksum)?)),
            _ => Err(Error::UnknownEnumValue {
                name: "checksum algorithm",
                value: algorithm as u64,
            }),
        }
    }

    /// Computes a checksum using the same algorithm variant as this value.
    pub fn compute_like(&self, bytes: &[u8]) -> Self {
        match self {
            AnyChecksum::None(_) => Self::None(NoChecksum::compute(bytes)),
            AnyChecksum::XXH64(_) => Self::XXH64(Xxh64Checksum::compute(bytes)),
            AnyChecksum::SHA256(_) => Self::SHA256(Sha256Checksum::compute(bytes)),
            AnyChecksum::SHA512(_) => Self::SHA512(Sha512Checksum::compute(bytes)),
            AnyChecksum::SM3(_) => Self::SM3(Sm3Checksum::compute(bytes)),
        }
    }
}

impl Default for AnyChecksum {
    fn default() -> Self {
        Self::None(NoChecksum)
    }
}

impl From<NoChecksum> for AnyChecksum {
    fn from(value: NoChecksum) -> Self {
        Self::None(value)
    }
}

impl From<Xxh64Checksum> for AnyChecksum {
    fn from(value: Xxh64Checksum) -> Self {
        Self::XXH64(value)
    }
}

impl From<Sha256Checksum> for AnyChecksum {
    fn from(value: Sha256Checksum) -> Self {
        Self::SHA256(value)
    }
}

impl From<Sha512Checksum> for AnyChecksum {
    fn from(value: Sha512Checksum) -> Self {
        Self::SHA512(value)
    }
}

impl From<Sm3Checksum> for AnyChecksum {
    fn from(value: Sm3Checksum) -> Self {
        Self::SM3(value)
    }
}

/// Detached OpenPGP signature bytes stored in `VerificationInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPgpSignature(Box<[u8]>);

impl OpenPgpSignature {
    /// Creates a detached OpenPGP signature wrapper from owned bytes.
    pub fn new(signature: Vec<u8>) -> Self {
        Self(signature.into_boxed_slice())
    }

    /// Returns the detached OpenPGP signature bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for OpenPgpSignature {
    fn from(value: Vec<u8>) -> Self {
        Self::new(value)
    }
}

impl From<Box<[u8]>> for OpenPgpSignature {
    fn from(value: Box<[u8]>) -> Self {
        Self(value)
    }
}

/// Detached CMS signature bytes stored in `VerificationInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmsSignature(Box<[u8]>);

impl CmsSignature {
    /// Creates a detached CMS signature wrapper from owned bytes.
    pub fn new(signature: Vec<u8>) -> Self {
        Self(signature.into_boxed_slice())
    }

    /// Returns the detached CMS signature bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for CmsSignature {
    fn from(value: Vec<u8>) -> Self {
        Self::new(value)
    }
}

impl From<Box<[u8]>> for CmsSignature {
    fn from(value: Box<[u8]>) -> Self {
        Self(value)
    }
}

/// Metadata verification strategies supported by the current implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationInfo {
    /// No metadata verification payload is stored.
    None,
    /// A checksum over the `FileMetadata` prefix.
    Checksum(AnyChecksum),
    /// Detached OpenPGP signature bytes.
    OpenPgp(OpenPgpSignature),
    /// Detached CMS signature bytes.
    Cms(CmsSignature),
}

/// Callback interface for detached-signature verification.
///
/// Janex stores only the detached signature bytes in `VerificationInfo`. For
/// OpenPGP and CMS, callers typically need external public keys, certificates,
/// or trust policy, so `JanexArchive::open_with_verifier` accepts one of these.
pub trait DetachedSignatureVerifier {
    /// Verifies a detached OpenPGP signature over the supplied metadata prefix.
    fn verify_openpgp(
        &self,
        signed_bytes: &[u8],
        signature: &OpenPgpSignature,
    ) -> Result<(), Error> {
        let _ = (signed_bytes, signature);
        Err(Error::VerificationFailed(
            "OpenPGP verification requires an external detached-signature verifier".to_string(),
        ))
    }

    /// Verifies a detached CMS signature over the supplied metadata prefix.
    fn verify_cms(&self, signed_bytes: &[u8], signature: &CmsSignature) -> Result<(), Error> {
        let _ = (signed_bytes, signature);
        Err(Error::VerificationFailed(
            "CMS verification requires an external detached-signature verifier".to_string(),
        ))
    }
}

/// Default verifier used by `open()`/`read_all()`, which rejects detached signatures.
#[derive(Debug, Clone, Copy, Default)]
pub struct RejectingDetachedSignatureVerifier;

impl DetachedSignatureVerifier for RejectingDetachedSignatureVerifier {}

impl VerificationInfo {
    /// Verifies the metadata prefix bytes according to this verification payload.
    pub(crate) fn verify<V: DetachedSignatureVerifier + ?Sized>(
        &self,
        signed_bytes: &[u8],
        verifier: &V,
    ) -> Result<(), Error> {
        match self {
            VerificationInfo::None => Ok(()),
            VerificationInfo::Checksum(checksum) => {
                verify_checksum(checksum, signed_bytes, "metadata")
            }
            VerificationInfo::OpenPgp(signature) => {
                verifier.verify_openpgp(signed_bytes, signature)
            }
            VerificationInfo::Cms(signature) => verifier.verify_cms(signed_bytes, signature),
        }
    }
}

/// Reads a Janex `Checksum` structure from the input stream.
pub(crate) fn read_checksum<R: DataReader>(reader: &mut R) -> Result<AnyChecksum, Error> {
    let algorithm = reader.read_u16_le()?;
    let reserved = reader.read_u8()?;
    if reserved != 0 {
        return Err(Error::InvalidValue("checksum reserved byte must be zero"));
    }

    let checksum = reader.read_bytes()?;
    AnyChecksum::from_raw(algorithm, checksum.as_ref())
}

/// Writes a Janex `Checksum` structure to the output stream.
pub(crate) fn write_checksum(
    writer: &mut VecDataWriter,
    checksum: &AnyChecksum,
) -> Result<(), Error> {
    writer.write_u16_le(checksum.algorithm_id());
    writer.write_u8(0);
    writer.write_bytes(checksum.as_bytes());
    Ok(())
}

/// Reads a `VerificationInfo` payload from `FileMetadata`.
pub(crate) fn read_verification_info<R: DataReader>(
    reader: &mut R,
) -> Result<VerificationInfo, Error> {
    let verification_type = reader.read_u8()?;
    let data = reader.read_bytes()?;
    match verification_type {
        0 => {
            if !data.is_empty() {
                return Err(Error::InvalidValue(
                    "VerificationType::None must not contain a payload",
                ));
            }
            Ok(VerificationInfo::None)
        }
        1 => {
            let mut payload_reader = ArrayDataReader::new(data.as_ref());
            let checksum = read_checksum(&mut payload_reader)?;
            if payload_reader.remaining() != 0 {
                return Err(Error::InvalidSectionLayout(
                    "verification checksum has trailing bytes".to_string(),
                ));
            }
            Ok(VerificationInfo::Checksum(checksum))
        }
        2 => {
            if data.is_empty() {
                return Err(Error::InvalidValue(
                    "VerificationType::OpenPGP must contain a detached signature payload",
                ));
            }
            Ok(VerificationInfo::OpenPgp(data.into()))
        }
        3 => {
            if data.is_empty() {
                return Err(Error::InvalidValue(
                    "VerificationType::CMS must contain a detached signature payload",
                ));
            }
            Ok(VerificationInfo::Cms(data.into()))
        }
        _ => Err(Error::UnknownEnumValue {
            name: "verification type",
            value: verification_type as u64,
        }),
    }
}

/// Encodes a `VerificationInfo` payload for storage in `FileMetadata`.
pub(crate) fn encode_verification_info(verification: &VerificationInfo) -> Result<Vec<u8>, Error> {
    let mut writer = VecDataWriter::new();
    match verification {
        VerificationInfo::None => {
            writer.write_u8(0);
            writer.write_bytes(&[]);
        }
        VerificationInfo::Checksum(checksum) => {
            writer.write_u8(1);
            let mut payload = VecDataWriter::new();
            write_checksum(&mut payload, checksum)?;
            writer.write_bytes(&payload.into_inner());
        }
        VerificationInfo::OpenPgp(signature) => {
            writer.write_u8(2);
            writer.write_bytes(signature.as_bytes());
        }
        VerificationInfo::Cms(signature) => {
            writer.write_u8(3);
            writer.write_bytes(signature.as_bytes());
        }
    }
    Ok(writer.into_inner())
}

/// Recomputes a checksum and compares it with the declared value.
pub(crate) fn verify_checksum(
    checksum: &AnyChecksum,
    bytes: &[u8],
    name: &'static str,
) -> Result<(), Error> {
    let expected = compute_checksum(checksum, bytes);
    if &expected != checksum {
        return Err(Error::VerificationFailed(format!(
            "{name} checksum mismatch"
        )));
    }
    Ok(())
}

/// Computes a checksum using the same algorithm variant as the provided template.
pub(crate) fn compute_checksum(template: &AnyChecksum, bytes: &[u8]) -> AnyChecksum {
    template.compute_like(bytes)
}

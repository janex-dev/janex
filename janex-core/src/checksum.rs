//  Copyright (c) 2026 Glavo
//  SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use sha2::{Digest, Sha256, Sha512};
use sm3::Sm3;
use xxhash_rust::xxh64::xxh64;

/// Static checksum algorithm contract used by concrete checksum structs.
pub trait Checksum<const SIZE: usize>:
    Sized + std::fmt::Debug + Clone + Copy + PartialEq + Eq
{
    const ALGORITHM_ID: u16;

    fn from_array(bytes: [u8; SIZE]) -> Self;

    fn as_array(&self) -> &[u8; SIZE];

    fn compute(bytes: &[u8]) -> Self;

    fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let expected = SIZE as u64;
        let actual = bytes.len() as u64;
        let bytes = <[u8; SIZE]>::try_from(bytes)
            .map_err(|_| Error::InvalidChecksumLength { expected, actual })?;
        Ok(Self::from_array(bytes))
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_array()
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
    None(NoChecksum),
    XXH64(Xxh64Checksum),
    SHA256(Sha256Checksum),
    SHA512(Sha512Checksum),
    SM3(Sm3Checksum),
}

impl AnyChecksum {
    /// Returns a checksum descriptor with no checksum payload.
    pub const fn none() -> Self {
        Self::None(NoChecksum)
    }

    pub const fn xxh64(bytes: [u8; 8]) -> Self {
        Self::XXH64(Xxh64Checksum::new(bytes))
    }

    pub const fn sha256(bytes: [u8; 32]) -> Self {
        Self::SHA256(Sha256Checksum::new(bytes))
    }

    pub const fn sha512(bytes: [u8; 64]) -> Self {
        Self::SHA512(Sha512Checksum::new(bytes))
    }

    pub const fn sm3(bytes: [u8; 32]) -> Self {
        Self::SM3(Sm3Checksum::new(bytes))
    }

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
        Self::none()
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

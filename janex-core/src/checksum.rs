//  Copyright (c) 2026 Glavo
//  SPDX-License-Identifier: MPL-2.0

use crate::error::Error;

/// A checksum payload stored alongside a section or resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    pub algorithm: ChecksumAlgorithm,
    pub checksum: Box<[u8]>,
}

impl Checksum {
    /// Returns a checksum descriptor with no checksum payload.
    pub fn none() -> Self {
        Self {
            algorithm: ChecksumAlgorithm::None,
            checksum: Box::new([]),
        }
    }
}

impl Default for Checksum {
    fn default() -> Self {
        Self::none()
    }
}

/// Supported checksum algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ChecksumAlgorithm {
    None = 0,
    XXH64 = 0x0101,
    SHA256 = 0x8101,
    SHA512 = 0x8102,
    SM3 = 0x8301,
}

impl ChecksumAlgorithm {
    pub const fn checksum_length(self) -> usize {
        match self {
            ChecksumAlgorithm::None => 0,
            ChecksumAlgorithm::XXH64 => 8,
            ChecksumAlgorithm::SHA256 => 32,
            ChecksumAlgorithm::SHA512 => 64,
            ChecksumAlgorithm::SM3 => 32,
        }
    }
}

impl TryFrom<u16> for ChecksumAlgorithm {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ChecksumAlgorithm::None),
            0x0101 => Ok(ChecksumAlgorithm::XXH64),
            0x8101 => Ok(ChecksumAlgorithm::SHA256),
            0x8102 => Ok(ChecksumAlgorithm::SHA512),
            0x8301 => Ok(ChecksumAlgorithm::SM3),
            _ => Err(Error::UnknownEnumValue {
                name: "checksum algorithm",
                value: value as u64,
            }),
        }
    }
}
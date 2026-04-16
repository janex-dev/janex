//  Copyright (c) 2026 Glavo
//  SPDX-License-Identifier: MPL-2.0

use crate::error::Error;

/// A checksum payload stored alongside a section or resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checksum {
    None,
    XXH64([u8; 8]),
    SHA256([u8; 32]),
    SHA512([u8; 64]),
    SM3([u8; 32]),
}

impl Checksum {
    /// Returns a checksum descriptor with no checksum payload.
    pub const fn none() -> Self {
        Self::None
    }

    /// Returns the raw Janex algorithm identifier used by the binary format.
    pub const fn algorithm_id(&self) -> u16 {
        match self {
            Checksum::None => 0,
            Checksum::XXH64(_) => 0x0101,
            Checksum::SHA256(_) => 0x8101,
            Checksum::SHA512(_) => 0x8102,
            Checksum::SM3(_) => 0x8301,
        }
    }

    /// Returns the checksum bytes without exposing format-specific tagging.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Checksum::None => &[],
            Checksum::XXH64(checksum) => checksum,
            Checksum::SHA256(checksum) => checksum,
            Checksum::SHA512(checksum) => checksum,
            Checksum::SM3(checksum) => checksum,
        }
    }

    /// Parses a checksum from its raw algorithm tag and payload bytes.
    pub fn from_raw(algorithm: u16, checksum: &[u8]) -> Result<Self, Error> {
        let checksum_length = checksum.len() as u64;

        match algorithm {
            0 => {
                if checksum.is_empty() {
                    Ok(Checksum::None)
                } else {
                    Err(Error::InvalidChecksumLength {
                        expected: 0,
                        actual: checksum_length,
                    })
                }
            }
            0x0101 => {
                let checksum =
                    <[u8; 8]>::try_from(checksum).map_err(|_| Error::InvalidChecksumLength {
                        expected: 8,
                        actual: checksum_length,
                    })?;
                Ok(Checksum::XXH64(checksum))
            }
            0x8101 => {
                let checksum =
                    <[u8; 32]>::try_from(checksum).map_err(|_| Error::InvalidChecksumLength {
                        expected: 32,
                        actual: checksum_length,
                    })?;
                Ok(Checksum::SHA256(checksum))
            }
            0x8102 => {
                let checksum =
                    <[u8; 64]>::try_from(checksum).map_err(|_| Error::InvalidChecksumLength {
                        expected: 64,
                        actual: checksum_length,
                    })?;
                Ok(Checksum::SHA512(checksum))
            }
            0x8301 => {
                let checksum =
                    <[u8; 32]>::try_from(checksum).map_err(|_| Error::InvalidChecksumLength {
                        expected: 32,
                        actual: checksum_length,
                    })?;
                Ok(Checksum::SM3(checksum))
            }
            _ => Err(Error::UnknownEnumValue {
                name: "checksum algorithm",
                value: algorithm as u64,
            }),
        }
    }
}

impl Default for Checksum {
    fn default() -> Self {
        Self::none()
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Standard digest encodings and streaming checksum verification.

use crate::{Error, ErrorKind, Result, error::invalid};
use sha2::{Digest, Sha256, Sha512};
use std::io::Read;

/// A supported checksum algorithm and its on-disk identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Algorithm {
    /// XXH3-64 with the default secret, seed zero, and a big-endian digest; not cryptographically secure.
    Xxh3_64 = 0x11,
    /// XXH3-128 with the default secret, seed zero, and a big-endian digest; not cryptographically secure.
    Xxh3_128 = 0x12,
    /// SHA-256 with a 32-byte digest.
    Sha256 = 0x21,
    /// SHA-512 with a 64-byte digest.
    Sha512 = 0x22,
    /// SM3 with a 32-byte digest.
    Sm3 = 0x31,
}

impl Algorithm {
    /// Returns the digest length in bytes.
    pub fn digest_length(self) -> usize {
        match self {
            Self::Xxh3_64 => 8,
            Self::Xxh3_128 => 16,
            Self::Sha256 | Self::Sm3 => 32,
            Self::Sha512 => 64,
        }
    }

    /// Returns whether this algorithm may authenticate section or external-region bytes.
    pub fn is_secure(self) -> bool {
        matches!(self, Self::Sha256 | Self::Sha512 | Self::Sm3)
    }

    /// Decodes a supported algorithm identifier; zero is invalid, other unknown IDs unsupported.
    pub fn from_id(id: u8) -> Result<Self> {
        match id {
            0 => Err(invalid("checksum algorithm zero is reserved")),
            0x11 => Ok(Self::Xxh3_64),
            0x12 => Ok(Self::Xxh3_128),
            0x21 => Ok(Self::Sha256),
            0x22 => Ok(Self::Sha512),
            0x31 => Ok(Self::Sm3),
            _ => Err(Error::new(
                ErrorKind::Unsupported,
                format!("checksum algorithm {id}"),
            )),
        }
    }
}

/// An algorithm identifier followed by its standard digest bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Checksum {
    /// The supported algorithm.
    algorithm: Algorithm,
    /// The exact standard digest representation.
    digest: Vec<u8>,
}

impl Checksum {
    /// Reads one complete checksum field, validating its algorithm and exact length.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (&id, digest) = bytes
            .split_first()
            .ok_or_else(|| invalid("empty checksum"))?;
        let algorithm = Algorithm::from_id(id)?;
        if digest.len() != algorithm.digest_length() {
            return Err(invalid("incorrect checksum length"));
        }
        Ok(Self {
            algorithm,
            digest: digest.to_vec(),
        })
    }

    /// Returns the algorithm.
    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    /// Borrows the digest bytes without the algorithm identifier.
    pub fn digest(&self) -> &[u8] {
        &self.digest
    }

    /// Encodes the complete checksum, suitable for a binary field or CBOR byte string.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = vec![self.algorithm as u8];
        bytes.extend_from_slice(&self.digest);
        bytes
    }

    /// Computes a checksum over all remaining bytes in a reader, consuming it through EOF.
    ///
    /// An I/O failure may have consumed a prefix of the input. No partial checksum is returned.
    pub fn compute(algorithm: Algorithm, mut reader: impl Read) -> Result<Self> {
        let digest = match algorithm {
            Algorithm::Xxh3_64 | Algorithm::Xxh3_128 => {
                let mut state = xxhash_rust::xxh3::Xxh3::new();
                read_chunks(&mut reader, |bytes| state.update(bytes))?;
                if algorithm == Algorithm::Xxh3_64 {
                    state.digest().to_be_bytes().to_vec()
                } else {
                    state.digest128().to_be_bytes().to_vec()
                }
            }
            Algorithm::Sha256 => hash::<Sha256>(&mut reader)?,
            Algorithm::Sha512 => hash::<Sha512>(&mut reader)?,
            Algorithm::Sm3 => hash::<sm3::Sm3>(&mut reader)?,
        };
        Ok(Self { algorithm, digest })
    }

    /// Verifies all remaining reader bytes, returning a verification error on mismatch.
    ///
    /// This checks content integrity and does not establish publisher identity or trust.
    pub fn verify(&self, reader: impl Read) -> Result<()> {
        if Self::compute(self.algorithm, reader)?.digest != self.digest {
            return Err(Error::new(ErrorKind::Verification, "checksum mismatch"));
        }
        Ok(())
    }
}

/// Hashes the remaining stream using a RustCrypto digest implementation.
fn hash<D: Digest + Default>(reader: &mut impl Read) -> Result<Vec<u8>> {
    let mut state = D::default();
    read_chunks(reader, |bytes| state.update(bytes))?;
    Ok(state.finalize().to_vec())
}

/// Passes bounded chunks to a consumer, retrying interrupted reads.
fn read_chunks(reader: &mut impl Read, mut consume: impl FnMut(&[u8])) -> Result<()> {
    let mut buffer = [0; 64 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => consume(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

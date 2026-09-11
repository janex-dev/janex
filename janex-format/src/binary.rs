// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Bounded binary decoding and shortest-form ULEB128 encoding.

use crate::{Error, ErrorKind, Result, error::invalid};
use std::io::Write;

/// Allocation and nesting limits for untrusted input.
///
/// These are reader policy, not file-format limits. Callers may raise them for larger
/// artifacts. Byte limits apply to individual buffered or decoded values.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum encoded or decoded bytes buffered for one value.
    pub max_bytes: u64,
    /// Maximum elements in one decoded collection.
    pub max_elements: u64,
    /// Maximum nesting depth for structured values.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024 * 1024,
            max_elements: 1_000_000,
            max_depth: 64,
        }
    }
}

impl Limits {
    /// Checks a byte length against the limit and the platform's address space.
    pub fn bytes(&self, length: u64) -> Result<usize> {
        if length > self.max_bytes {
            return Err(Error::new(ErrorKind::Limit, "byte limit exceeded"));
        }
        usize::try_from(length)
            .map_err(|_| Error::new(ErrorKind::Limit, "length does not fit usize"))
    }

    /// Checks a collection length against the element limit and address space.
    pub fn elements(&self, count: u64) -> Result<usize> {
        if count > self.max_elements {
            return Err(Error::new(ErrorKind::Limit, "element limit exceeded"));
        }
        usize::try_from(count).map_err(|_| Error::new(ErrorKind::Limit, "count does not fit usize"))
    }
}

/// A cursor borrowing one complete binary field.
///
/// Successful reads advance the cursor. A failed read may retain partial progress;
/// callers must discard the containing value rather than resume its decoding.
pub struct Decoder<'a> {
    /// The bounded input.
    bytes: &'a [u8],
    /// The next byte to read.
    position: usize,
    /// Resource limits for nested lengths.
    limits: Limits,
}

impl<'a> Decoder<'a> {
    /// Creates a decoder at the start of `bytes`, rejecting an oversized field.
    pub fn new(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        limits.bytes(bytes.len() as u64)?;
        Ok(Self {
            bytes,
            position: 0,
            limits,
        })
    }

    /// Returns the number of bytes consumed from this field.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Returns the limits used for this decoder.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Returns the number of unread bytes.
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Returns and consumes exactly `length` bytes, or reports truncation.
    pub fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(|| invalid("truncated binary field").at(self.position as u64))?;
        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }

    /// Reads an unsigned byte.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Reads a little-endian unsigned 32-bit integer.
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("fixed length"),
        ))
    }

    /// Reads a little-endian unsigned 64-bit integer.
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("fixed length"),
        ))
    }

    /// Reads a ULEB128 `u64`, accepting zero padding within the ten-byte limit.
    pub fn vuint(&mut self) -> Result<u64> {
        let start = self.position;
        let mut value = 0;
        for index in 0..10 {
            let byte = self.u8()?;
            if index == 9 && byte > 1 {
                return Err(invalid("ULEB128 exceeds u64").at(start as u64));
            }
            value |= u64::from(byte & 0x7f) << (index * 7);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid("unterminated ULEB128").at(start as u64))
    }

    /// Reads an element count and checks it against the element limit.
    pub fn count(&mut self) -> Result<usize> {
        let count = self.vuint()?;
        self.limits.elements(count)
    }

    /// Reads a length-prefixed byte field without allocating its contents.
    pub fn sized(&mut self) -> Result<&'a [u8]> {
        let length = self.vuint()?;
        let length = self.limits.bytes(length)?;
        self.take(length)
    }

    /// Reads a length-prefixed UTF-8 string, rejecting malformed UTF-8.
    pub fn string(&mut self) -> Result<&'a str> {
        let start = self.position;
        std::str::from_utf8(self.sized()?)
            .map_err(|_| invalid("invalid UTF-8 string").at(start as u64))
    }

    /// Checks that the containing field has no trailing bytes.
    pub fn finish(&self) -> Result<()> {
        if self.remaining() != 0 {
            return Err(invalid("trailing bytes").at(self.position as u64));
        }
        Ok(())
    }
}

/// Writes the shortest ULEB128 representation of `value`.
///
/// An I/O failure may leave a prefix in the destination.
pub fn write_vuint(mut writer: impl Write, mut value: u64) -> Result<()> {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        writer.write_all(&[byte | if value == 0 { 0 } else { 0x80 }])?;
        if value == 0 {
            return Ok(());
        }
    }
}

/// Writes the byte length followed by the exact contents of `bytes`.
///
/// An I/O failure may leave a prefix in the destination.
pub fn write_sized(mut writer: impl Write, bytes: &[u8]) -> Result<()> {
    write_vuint(&mut writer, bytes.len() as u64)?;
    writer.write_all(bytes)?;
    Ok(())
}

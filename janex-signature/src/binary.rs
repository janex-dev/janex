// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Bounded signature and key parsing.

use crate::{Error, ErrorKind, Result, error::invalid};

/// Allocation and nesting limits for untrusted input.
///
/// These are caller policy, not cryptographic format limits. Callers may raise them for larger
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
pub(crate) struct Decoder<'a> {
    /// The bounded input.
    bytes: &'a [u8],
    /// The next byte to read.
    position: usize,
    /// Resource limits for nested lengths.
    limits: Limits,
}

impl<'a> Decoder<'a> {
    /// Creates a decoder at the start of `bytes`, rejecting an oversized field.
    pub(crate) fn new(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        limits.bytes(bytes.len() as u64)?;
        Ok(Self {
            bytes,
            position: 0,
            limits,
        })
    }

    /// Returns the number of bytes consumed from this field.
    pub(crate) fn position(&self) -> usize {
        self.position
    }

    /// Returns the limits used for this decoder.
    pub(crate) fn limits(&self) -> Limits {
        self.limits
    }

    /// Returns the number of unread bytes.
    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Returns and consumes exactly `length` bytes, or reports truncation.
    pub(crate) fn take(&mut self, length: usize) -> Result<&'a [u8]> {
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
    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
}

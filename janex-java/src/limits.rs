// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Limits for Java metadata and bootstrap resources.

use crate::{Error, Result};

/// Allocation limits for untrusted input.
///
/// These are caller policy for Java metadata and argument transport. Callers may raise them for larger
/// artifacts. Byte limits apply to individual buffered or decoded values.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum encoded or decoded bytes buffered for one value.
    pub max_bytes: u64,
    /// Maximum elements in one decoded collection.
    pub max_elements: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024 * 1024,
            max_elements: 1_000_000,
        }
    }
}

impl Limits {
    /// Checks a byte length against the limit and the platform's address space.
    pub fn bytes(&self, length: u64) -> Result<usize> {
        if length > self.max_bytes {
            return Err(Error::Limit("byte limit exceeded".into()));
        }
        usize::try_from(length).map_err(|_| Error::Limit("length does not fit usize".into()))
    }

    /// Checks a collection length against the element limit and address space.
    pub fn elements(&self, count: u64) -> Result<usize> {
        if count > self.max_elements {
            return Err(Error::Limit("element limit exceeded".into()));
        }
        usize::try_from(count).map_err(|_| Error::Limit("count does not fit usize".into()))
    }
}

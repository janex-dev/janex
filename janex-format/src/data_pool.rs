// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Indexed, opaque byte sequences shared by resource paths and content transforms.

use crate::{
    Result,
    binary::{Decoder, Limits, write_sized, write_vuint},
    error::invalid,
};
use std::collections::BTreeMap;

/// An ordered collection of unique byte sequences with the empty sequence at index zero.
///
/// Existing indices remain stable when new entries are interned. The binary representation
/// is a vector of length-prefixed byte sequences; interpretation belongs to consumers.
#[derive(Clone, Debug)]
pub struct DataPool {
    /// Byte sequences in index order.
    values: Vec<Vec<u8>>,
    /// Reverse lookup used when constructing the pool.
    indices: BTreeMap<Vec<u8>, u64>,
}

impl Default for DataPool {
    fn default() -> Self {
        Self {
            values: vec![Vec::new()],
            indices: BTreeMap::from([(Vec::new(), 0)]),
        }
    }
}

impl DataPool {
    /// Creates a pool containing only the required empty sequence.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes exactly one pool, rejecting duplicate byte sequences and an invalid index zero.
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        let mut decoder = Decoder::new(bytes, limits)?;
        let count = decoder.count()?;
        if count == 0 {
            return Err(invalid("data pool must contain index zero"));
        }
        let mut pool = Self {
            values: Vec::new(),
            indices: BTreeMap::new(),
        };
        for index in 0..count {
            let value = decoder.sized()?;
            if index == 0 && !value.is_empty() {
                return Err(invalid("data pool index zero must be empty"));
            }
            if pool.indices.insert(value.into(), index as u64).is_some() {
                return Err(invalid("duplicate data pool entry"));
            }
            pool.values.push(value.into());
        }
        decoder.finish()?;
        Ok(pool)
    }

    /// Encodes the pool in its existing index order.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        write_vuint(&mut bytes, self.values.len() as u64)?;
        for value in &self.values {
            write_sized(&mut bytes, value)?;
        }
        Ok(bytes)
    }

    /// Returns the number of entries, including the empty sequence.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns false: every valid pool contains the empty sequence at index zero.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Borrows bytes by index, rejecting references outside the pool.
    pub fn get(&self, index: u64) -> Result<&[u8]> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.values.get(index))
            .map(Vec::as_slice)
            .ok_or_else(|| invalid("data pool index out of range"))
    }

    /// Returns an existing index without modifying the pool.
    pub fn find(&self, value: impl AsRef<[u8]>) -> Option<u64> {
        let value = value.as_ref();
        self.indices.get(value).copied()
    }

    /// Returns the existing index or appends a new byte sequence.
    pub fn intern(&mut self, value: impl AsRef<[u8]>) -> u64 {
        let value = value.as_ref();
        if let Some(index) = self.find(value) {
            return index;
        }
        let index = self.values.len() as u64;
        self.values.push(value.into());
        self.indices.insert(value.into(), index);
        index
    }

    /// Restores a previous length after abandoning tentative interning.
    pub(crate) fn truncate(&mut self, length: usize) {
        for value in self.values.drain(length.max(1)..) {
            self.indices.remove(&value);
        }
    }
}

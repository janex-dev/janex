// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Shared UTF-8 strings and resource-name representations.

use crate::{
    Result,
    binary::{Decoder, Limits, write_sized, write_vuint},
    error::invalid,
};
use std::collections::BTreeMap;

/// An ordered collection of unique UTF-8 strings with the empty string at index zero.
///
/// Existing indices remain stable when new strings are interned. The binary representation
/// is a vector of length-prefixed UTF-8 strings; no Unicode normalization is performed.
#[derive(Clone, Debug)]
pub struct StringPool {
    /// Strings in index order.
    values: Vec<String>,
    /// Reverse lookup used when constructing the pool.
    indices: BTreeMap<String, u64>,
}

impl Default for StringPool {
    fn default() -> Self {
        Self {
            values: vec![String::new()],
            indices: BTreeMap::from([(String::new(), 0)]),
        }
    }
}

impl StringPool {
    /// Creates a pool containing only the required empty string.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes exactly one pool, rejecting duplicate strings and an invalid index zero.
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        let mut decoder = Decoder::new(bytes, limits)?;
        let count = decoder.count()?;
        if count == 0 {
            return Err(invalid("string pool must contain index zero"));
        }
        let mut pool = Self {
            values: Vec::new(),
            indices: BTreeMap::new(),
        };
        for index in 0..count {
            let value = decoder.string()?;
            if index == 0 && !value.is_empty() {
                return Err(invalid("string pool index zero must be empty"));
            }
            if pool.indices.insert(value.into(), index as u64).is_some() {
                return Err(invalid("duplicate string pool entry"));
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
            write_sized(&mut bytes, value.as_bytes())?;
        }
        Ok(bytes)
    }

    /// Returns the number of strings, including the empty string.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns false: every valid pool contains the empty string at index zero.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Borrows a string by index, rejecting references outside the pool.
    pub fn get(&self, index: u64) -> Result<&str> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.values.get(index))
            .map(String::as_str)
            .ok_or_else(|| invalid("string pool index out of range"))
    }

    /// Returns an existing index without modifying the pool.
    pub fn find(&self, value: &str) -> Option<u64> {
        self.indices.get(value).copied()
    }

    /// Returns the existing index or appends a new string.
    pub fn intern(&mut self, value: &str) -> u64 {
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

    /// Reads and resolves a nonempty name in any of the three permitted representations.
    pub fn read_nonempty(&self, decoder: &mut Decoder<'_>) -> Result<String> {
        let index = decoder.vuint()?;
        let value = if index != 0 {
            let text = self.get(index)?;
            decoder.limits().bytes(text.len() as u64)?;
            text.to_owned()
        } else {
            let inline = decoder.string()?;
            if !inline.is_empty() {
                inline.to_owned()
            } else {
                let count = decoder.count()?;
                if count < 2 {
                    return Err(invalid(
                        "string concatenation requires at least two entries",
                    ));
                }
                let mut value = String::new();
                for _ in 0..count {
                    let item = self.get(decoder.vuint()?)?;
                    let length = value
                        .len()
                        .checked_add(item.len())
                        .ok_or_else(|| invalid("concatenated string length overflow"))?;
                    decoder.limits().bytes(length as u64)?;
                    value.push_str(item);
                }
                value
            }
        };
        decoder.limits().bytes(value.len() as u64)?;
        if value.is_empty() {
            return Err(invalid("resolved name must not be empty"));
        }
        Ok(value)
    }

    /// Writes a pooled name, sharing an existing class basename when possible.
    ///
    /// An existing full-string index is preferred. Otherwise an existing basename and
    /// the `.class` suffix may be concatenated; other names are interned as full strings.
    pub fn write_nonempty(&mut self, value: &str, bytes: &mut Vec<u8>) -> Result<()> {
        if value.is_empty() {
            return Err(invalid("name must not be empty"));
        }
        if let Some(index) = self.find(value) {
            return write_vuint(bytes, index);
        }
        if let Some(base) = value.strip_suffix(".class").filter(|base| !base.is_empty())
            && let Some(index) = self.find(base)
        {
            let suffix = self.intern(".class");
            bytes.extend_from_slice(&[0, 0, 2]);
            write_vuint(&mut *bytes, index)?;
            return write_vuint(bytes, suffix);
        }
        write_vuint(bytes, self.intern(value))
    }
}

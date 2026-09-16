// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Indexed, opaque byte sequences shared by resource paths and content transforms.

use crate::{
    Result,
    binary::{Decoder, Limits, write_sized, write_vuint},
    error::invalid,
};
use std::{collections::HashMap, ops::Deref};

/// A read-only collection of byte sequences, with the empty sequence at index zero.
/// Entries occupy a contiguous byte buffer and are accessed by index. Interpretation belongs
/// to consumers. Use [`DataPoolBuilder`] to intern additional entries before encoding.
#[derive(Clone, Debug)]
pub struct DataPool {
    /// Concatenated entry payloads without framing bytes.
    bytes: Vec<u8>,
    /// Entry boundaries, including the final end offset; each is bounded by the owned buffer.
    offsets: Vec<u64>,
}

impl Default for DataPool {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            offsets: vec![0, 0],
        }
    }
}

impl DataPool {
    /// Creates a pool containing only the required empty byte sequence.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes exactly one pool, rejecting invalid index zero and trailing bytes.
    /// Entries retain their encoded indices without checking uniqueness. The returned pool owns
    /// only payloads and entry offsets, with no reverse index or per-entry byte allocation.
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        let mut decoder = Decoder::new(bytes, limits)?;
        let count = decoder.count()?;
        if count == 0 {
            return Err(invalid("data pool must contain index zero"));
        }
        if count > decoder.remaining() {
            return Err(invalid("truncated data pool entries"));
        }
        let mut pool = Self {
            bytes: Vec::with_capacity(bytes.len()),
            offsets: Vec::with_capacity(count + 1),
        };
        pool.offsets.push(0);
        for index in 0..count {
            let value = decoder.sized()?;
            if index == 0 && !value.is_empty() {
                return Err(invalid("data pool index zero must be empty"));
            }
            pool.bytes.extend_from_slice(value);
            pool.offsets.push(pool.bytes.len() as u64);
        }
        decoder.finish()?;
        Ok(pool)
    }

    /// Encodes all entries in their existing index order.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        write_vuint(&mut bytes, self.len())?;
        for range in self.offsets.windows(2) {
            write_sized(
                &mut bytes,
                &self.bytes[range[0] as usize..range[1] as usize],
            )?;
        }
        Ok(bytes)
    }

    /// Borrows all entry payloads concatenated in index order, without framing bytes.
    pub fn payload(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the number of entries, including the empty sequence.
    pub fn len(&self) -> u64 {
        (self.offsets.len() - 1) as u64
    }

    /// Returns false: every valid pool contains index zero.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Borrows bytes by index, rejecting references outside the pool.
    pub fn get(&self, index: u64) -> Result<&[u8]> {
        let index = usize::try_from(index)
            .ok()
            .filter(|&index| index < self.offsets.len() - 1)
            .ok_or_else(|| invalid("data pool index out of range"))?;
        Ok(&self.bytes[self.offsets[index] as usize..self.offsets[index + 1] as usize])
    }
}

/// Builds a data pool using a reverse index for byte-exact deduplication.
/// Existing indices remain stable when entries are interned. [`Self::finish`] discards
/// the reverse index and transfers the accumulated payloads to a read-only pool.
#[derive(Clone, Debug)]
pub struct DataPoolBuilder {
    /// Accumulated entries in index order.
    pool: DataPool,
    /// Encoding-only reverse lookup.
    indices: HashMap<Vec<u8>, u64>,
}

impl Default for DataPoolBuilder {
    fn default() -> Self {
        Self::from(DataPool::new())
    }
}

impl From<DataPool> for DataPoolBuilder {
    /// Rebuilds the encoding index while retaining all existing indices and payloads.
    fn from(pool: DataPool) -> Self {
        let indices = pool
            .offsets
            .windows(2)
            .enumerate()
            .map(|(index, range)| {
                (
                    pool.bytes[range[0] as usize..range[1] as usize].to_vec(),
                    index as u64,
                )
            })
            .collect();
        Self { pool, indices }
    }
}

impl Deref for DataPoolBuilder {
    type Target = DataPool;

    /// Borrows the current entries without exposing mutable access to their bytes.
    fn deref(&self) -> &DataPool {
        &self.pool
    }
}

impl DataPoolBuilder {
    /// Creates a builder containing only the required empty sequence.
    pub fn new() -> Self {
        Self::default()
    }

    /// Discards the encoding index and returns the accumulated read-only pool without copying.
    pub fn finish(self) -> DataPool {
        self.pool
    }

    /// Returns an existing index without modifying the builder.
    pub fn find(&self, value: impl AsRef<[u8]>) -> Option<u64> {
        self.indices.get(value.as_ref()).copied()
    }

    /// Returns an existing index or appends a new byte sequence.
    pub fn intern(&mut self, value: impl AsRef<[u8]>) -> u64 {
        let value = value.as_ref();
        if let Some(index) = self.find(value) {
            return index;
        }
        let index = self.len();
        self.pool.bytes.extend_from_slice(value);
        self.pool.offsets.push(self.pool.bytes.len() as u64);
        self.indices.insert(value.into(), index);
        index
    }

    /// Removes entries appended since a previous length, preserving index zero.
    pub(crate) fn truncate(&mut self, length: u64) {
        let length =
            usize::try_from(length.max(1)).expect("pool checkpoint fits the address space");
        for range in self.pool.offsets[length..].windows(2) {
            self.indices
                .remove(&self.pool.bytes[range[0] as usize..range[1] as usize]);
        }
        self.pool.bytes.truncate(self.pool.offsets[length] as usize);
        self.pool.offsets.truncate(length + 1);
    }
}

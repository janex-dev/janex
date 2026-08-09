// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::janex::{ensure_fully_consumed, read_len_prefixed_vec, write_len_prefixed_slice};
use std::collections::BTreeMap;

/// A deduplicated pool of UTF-8 strings used by Janex sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringPool {
    /// The pool strings in index order.
    strings: Vec<String>,
    /// Reverse lookup from string content to pool index.
    indices: BTreeMap<String, u64>,
}

impl StringPool {
    /// Creates a string pool from an explicit list of strings.
    ///
    /// The pool must contain at least one element and index `0` must be the empty string.
    pub fn new(strings: Vec<String>) -> Result<Self, Error> {
        let mut pool = Self {
            strings: Vec::with_capacity(strings.len()),
            indices: BTreeMap::new(),
        };

        for string in strings {
            pool.push_existing(string)?;
        }

        if pool.strings.is_empty() {
            return Err(Error::InvalidValue(
                "string pool must contain at least one string",
            ));
        }

        if !pool.strings[0].is_empty() {
            return Err(Error::InvalidValue(
                "string pool index 0 must be an empty string",
            ));
        }

        Ok(pool)
    }

    /// Creates a valid empty-rooted pool whose first entry is `""`.
    pub fn with_empty_root() -> Self {
        let mut indices = BTreeMap::new();
        indices.insert(String::new(), 0);
        Self {
            strings: vec![String::new()],
            indices,
        }
    }

    /// Returns the number of strings stored in the pool.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Returns whether the pool has no entries.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// Returns the string at the given pool index.
    pub fn get(&self, index: u64) -> Option<&str> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.strings.get(index))
            .map(String::as_str)
    }

    /// Inserts a string if it is not already present and returns its index.
    pub fn push(&mut self, value: impl Into<String>) -> u64 {
        let value = value.into();
        if let Some(index) = self.indices.get(&value) {
            return *index;
        }

        let index = self.strings.len() as u64;
        self.indices.insert(value.clone(), index);
        self.strings.push(value);
        index
    }

    /// Iterates over the pool in index order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &str> {
        self.strings.iter().map(String::as_str)
    }

    /// Inserts a string that is known to be unique already, used while reconstructing a `StringPool` from encoded data.
    fn push_existing(&mut self, value: String) -> Result<(), Error> {
        if self.indices.contains_key(&value) {
            return Err(Error::InvalidValue(
                "string pool contains duplicated strings",
            ));
        }

        let index = self.strings.len() as u64;
        self.indices.insert(value.clone(), index);
        self.strings.push(value);
        Ok(())
    }
}

impl Default for StringPool {
    fn default() -> Self {
        Self::with_empty_root()
    }
}

/// A self-describing string-pool payload stored as a blob in a `DataPool`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringPoolData {
    /// Strings stored in pool-index order.
    pub strings: StringPool,
}

impl StringPoolData {
    /// The payload magic number (`"STRPOOL\0"`).
    pub const MAGIC_NUMBER: u64 = 0x004c_4f4f_5052_5453;
    /// The supported `StringPoolData` schema version.
    pub const VERSION: u32 = 1;
    /// The encoding tag for standard UTF-8 strings.
    pub const UTF8_ENCODING: u8 = 0;

    /// Creates string-pool data from a validated pool.
    pub fn new(strings: StringPool) -> Self {
        Self { strings }
    }

    /// Decodes self-describing string-pool data from its logical blob bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        decode_string_pool_data(bytes)
    }

    /// Encodes this string pool into its logical, uncompressed blob representation.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        encode_string_pool_data(self)
    }
}

impl Default for StringPoolData {
    fn default() -> Self {
        Self::new(StringPool::default())
    }
}

/// Parses a `StringPoolData` blob from its logical bytes.
fn decode_string_pool_data(bytes: &[u8]) -> Result<StringPoolData, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = DataReader::read_u64_le(&mut reader)?;
    if magic != StringPoolData::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: StringPoolData::MAGIC_NUMBER,
            actual: magic,
        });
    }

    let version = DataReader::read_u32_le(&mut reader)?;
    if version != StringPoolData::VERSION {
        return Err(Error::UnsupportedFeature(
            "unsupported string pool data version",
        ));
    }

    let encoding = DataReader::read_u8(&mut reader)?;
    if encoding != StringPoolData::UTF8_ENCODING {
        return Err(Error::UnknownEnumValue {
            name: "string pool encoding",
            value: encoding as u64,
        });
    }

    let strings = read_len_prefixed_vec(&mut reader, DataReader::read_string)?;
    ensure_fully_consumed(&reader, "string pool data")?;
    Ok(StringPoolData::new(StringPool::new(strings)?))
}

/// Encodes a `StringPoolData` value into its logical blob representation.
fn encode_string_pool_data(data: &StringPoolData) -> Result<Vec<u8>, Error> {
    let mut writer = VecDataWriter::new();
    writer.write_u64_le(StringPoolData::MAGIC_NUMBER);
    writer.write_u32_le(StringPoolData::VERSION);
    writer.write_u8(StringPoolData::UTF8_ENCODING);
    let strings: Vec<&str> = data.strings.iter().collect();
    write_len_prefixed_slice(&mut writer, &strings, |writer, string| {
        writer.write_string(string);
        Ok(())
    })?;
    Ok(writer.into_inner())
}

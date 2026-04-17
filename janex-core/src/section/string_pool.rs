// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::janex::{
    CompressInfo, StringPoolSection, ensure_fully_consumed, read_compressed_blob, read_usize,
    write_compressed_blob,
};
use std::collections::BTreeMap;

/// A deduplicated pool of UTF-8 strings used by Janex sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringPool {
    strings: Vec<String>,
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
        self.strings.get(index as usize).map(String::as_str)
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

pub(crate) fn parse_string_pool_section(bytes: &[u8]) -> Result<StringPoolSection, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = DataReader::read_u64_le(&mut reader)?;
    if magic != StringPoolSection::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: StringPoolSection::MAGIC_NUMBER,
            actual: magic,
        });
    }

    let count = read_usize(DataReader::read_vuint(&mut reader)?)?;
    let mut sizes = Vec::with_capacity(count);
    for _ in 0..count {
        sizes.push(read_usize(DataReader::read_vuint(&mut reader)?)?);
    }

    let (compression, data) = read_compressed_blob(&mut reader)?;
    ensure_fully_consumed(&reader, "string pool section")?;

    let expected_total: usize = sizes.iter().sum();
    if expected_total != data.len() {
        return Err(Error::InvalidSectionLayout(
            "string pool byte count does not match the declared sizes".to_string(),
        ));
    }

    let mut strings = Vec::with_capacity(count);
    let mut offset = 0usize;
    for size in sizes {
        let end = offset.checked_add(size).ok_or_else(|| {
            Error::InvalidSectionLayout("string pool offset overflow".to_string())
        })?;
        strings.push(String::from_utf8(data[offset..end].to_vec())?);
        offset = end;
    }

    Ok(StringPoolSection {
        compression,
        strings: StringPool::new(strings)?,
    })
}

pub(crate) fn encode_string_pool_section(
    writer: &mut VecDataWriter,
    section: &StringPoolSection,
) -> Result<(), Error> {
    writer.write_u64_le(StringPoolSection::MAGIC_NUMBER);
    writer.write_vuint(section.strings.len() as u64);

    let strings: Vec<&str> = section.strings.iter().collect();
    for string in &strings {
        writer.write_vuint(string.len() as u64);
    }

    let uncompressed = strings.concat().into_bytes();
    write_compressed_blob(writer, &section.compression, &uncompressed)
}

impl StringPoolSection {
    pub const MAGIC_NUMBER: u64 = 0x004c_4f4f_5052_5453;

    /// Creates a string-pool section using no compression.
    pub fn new(strings: StringPool) -> Self {
        Self {
            compression: CompressInfo::none(),
            strings,
        }
    }
}

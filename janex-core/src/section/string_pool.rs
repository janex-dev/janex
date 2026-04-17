// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::janex::{
    ensure_fully_consumed, read_compressed_blob, read_usize, write_compressed_blob,
    StringPoolSection,
};
use crate::string_pool::StringPool;

pub(crate) fn parse(bytes: &[u8]) -> Result<StringPoolSection, Error> {
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

pub(crate) fn encode(writer: &mut VecDataWriter, section: &StringPoolSection) -> Result<(), Error> {
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
            compression: crate::janex::CompressInfo::none(),
            strings,
        }
    }
}

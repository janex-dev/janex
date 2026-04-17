// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::janex::DataPoolSection;

/// Parses a `DataPool` section from its encoded bytes.
pub(crate) fn parse(bytes: &[u8]) -> Result<DataPoolSection, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = DataReader::read_u64_le(&mut reader)?;
    if magic != DataPoolSection::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: DataPoolSection::MAGIC_NUMBER,
            actual: magic,
        });
    }
    let remaining = DataReader::remaining(&reader);
    Ok(DataPoolSection {
        bytes: DataReader::read_u8_array(&mut reader, remaining)?,
    })
}

/// Encodes a `DataPool` section into its on-disk representation.
pub(crate) fn encode(writer: &mut VecDataWriter, section: &DataPoolSection) {
    writer.write_u64_le(DataPoolSection::MAGIC_NUMBER);
    writer.write_all(&section.bytes);
}

impl DataPoolSection {
    /// The `DataPool` section magic number (`"DATAPOOL"`).
    pub const MAGIC_NUMBER: u64 = 0x4c4f_4f50_4154_4144;
}

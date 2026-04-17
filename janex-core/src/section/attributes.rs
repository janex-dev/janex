// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::janex::{
    Attribute, AttributesSection, SectionType, ensure_fully_consumed, read_len_prefixed_vec,
    write_len_prefixed_slice,
};

/// Parses an `Attributes` section from its encoded bytes.
pub(crate) fn parse(bytes: &[u8]) -> Result<AttributesSection, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = DataReader::read_u64_le(&mut reader)?;
    if magic != AttributesSection::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: AttributesSection::MAGIC_NUMBER,
            actual: magic,
        });
    }

    let attributes = read_len_prefixed_vec(&mut reader, read_attribute)?;
    ensure_fully_consumed(&reader, "attributes section")?;
    Ok(AttributesSection { attributes })
}

/// Encodes an `Attributes` section into its on-disk representation.
pub(crate) fn encode(
    writer: &mut VecDataWriter,
    section: &AttributesSection,
) -> Result<(), Error> {
    writer.write_u64_le(AttributesSection::MAGIC_NUMBER);
    write_len_prefixed_slice(writer, &section.attributes, write_attribute)
}

impl AttributesSection {
    /// The `Attributes` section magic number (`"ATTRIBS."`).
    pub const MAGIC_NUMBER: u64 = SectionType::ATTRIBUTES_RAW;
}

/// Reads one name/value attribute entry.
fn read_attribute<R: DataReader>(reader: &mut R) -> Result<Attribute, Error> {
    Ok(Attribute {
        name: reader.read_string()?,
        value: reader.read_bytes()?,
    })
}

/// Writes one name/value attribute entry.
fn write_attribute(writer: &mut VecDataWriter, attribute: &Attribute) -> Result<(), Error> {
    writer.write_string(&attribute.name);
    writer.write_bytes(&attribute.value);
    Ok(())
}

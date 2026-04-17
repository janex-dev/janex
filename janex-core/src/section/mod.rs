// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::VecDataWriter;
use crate::janex::{SectionContent, SectionType};
use self::string_pool::{encode_string_pool_section, parse_string_pool_section};

mod data_pool;
mod padding;
mod resource_groups;
mod root_config_group;
pub mod string_pool;

#[cfg(test)]
pub(crate) use resource_groups::validate_resource_path;

pub(crate) fn parse_section_content(
    section_type: SectionType,
    bytes: &[u8],
) -> Result<SectionContent, Error> {
    match section_type {
        SectionType::Padding => Ok(SectionContent::Padding(padding::parse(bytes))),
        SectionType::RootConfigGroup => Ok(SectionContent::RootConfigGroup(
            root_config_group::parse(bytes)?,
        )),
        SectionType::ResourceGroups => Ok(SectionContent::ResourceGroups(
            resource_groups::parse(bytes)?,
        )),
        SectionType::StringPool => Ok(SectionContent::StringPool(parse_string_pool_section(
            bytes,
        )?)),
        SectionType::DataPool => Ok(SectionContent::DataPool(data_pool::parse(bytes)?)),
        SectionType::ExternalHeader
        | SectionType::ExternalTail
        | SectionType::FileMetadata
        | SectionType::Attributes => Err(unsupported_section_error()),
    }
}

pub(crate) fn encode_section_content(section: &SectionContent) -> Result<Vec<u8>, Error> {
    let mut writer = VecDataWriter::new();
    match section {
        SectionContent::Padding(bytes) => padding::encode(&mut writer, bytes),
        SectionContent::RootConfigGroup(section) => root_config_group::encode(&mut writer, section)?,
        SectionContent::ResourceGroups(section) => resource_groups::encode(&mut writer, section)?,
        SectionContent::StringPool(section) => encode_string_pool_section(&mut writer, section)?,
        SectionContent::DataPool(section) => data_pool::encode(&mut writer, section),
    }
    Ok(writer.into_inner())
}

impl SectionContent {
    pub(crate) fn section_type(&self) -> SectionType {
        match self {
            SectionContent::Padding(_) => SectionType::Padding,
            SectionContent::RootConfigGroup(_) => SectionType::RootConfigGroup,
            SectionContent::ResourceGroups(_) => SectionType::ResourceGroups,
            SectionContent::StringPool(_) => SectionType::StringPool,
            SectionContent::DataPool(_) => SectionType::DataPool,
        }
    }
}

fn unsupported_section_error() -> Error {
    Error::UnsupportedFeature(
        "external header/tail, attributes, and nested metadata sections are not implemented",
    )
}

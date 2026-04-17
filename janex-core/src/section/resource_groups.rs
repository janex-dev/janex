// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::checksum::{read_checksum, write_checksum};
use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::janex::{
    Resource, ResourceField, ResourceGroup, ResourceGroupsSection, ResourcePath, Timestamp,
    ensure_fully_consumed, read_compress_info, read_compressed_blob, read_len_prefixed_vec,
    read_tagged_field_u32, read_usize, write_compress_info, write_compressed_blob,
    write_len_prefixed_slice, write_payload, write_tagged_field_u32,
};
use crate::string_pool::StringPool;
use std::collections::HashSet;

pub(crate) fn parse(bytes: &[u8]) -> Result<ResourceGroupsSection, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = DataReader::read_u64_le(&mut reader)?;
    if magic != ResourceGroupsSection::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: ResourceGroupsSection::MAGIC_NUMBER,
            actual: magic,
        });
    }
    let groups = read_len_prefixed_vec(&mut reader, read_resource_group)?;
    ensure_fully_consumed(&reader, "resource groups section")?;
    Ok(ResourceGroupsSection { groups })
}

pub(crate) fn encode(
    writer: &mut VecDataWriter,
    section: &ResourceGroupsSection,
) -> Result<(), Error> {
    writer.write_u64_le(ResourceGroupsSection::MAGIC_NUMBER);
    write_len_prefixed_slice(writer, &section.groups, write_resource_group)
}

impl ResourceGroupsSection {
    pub const MAGIC_NUMBER: u64 = 0x0053_5052_4753_4552;
}

impl ResourceGroup {
    pub const MAGIC_NUMBER: u32 = 0x4753_4552;

    pub(crate) fn validate(
        &self,
        string_pool: Option<&StringPool>,
        data_pool_len: Option<u64>,
    ) -> Result<(), Error> {
        let mut paths = HashSet::new();
        for resource in &self.resources {
            let path = resource.path().resolve(string_pool)?;
            if !paths.insert(path.clone()) {
                return Err(Error::InvalidSectionLayout(format!(
                    "resource group '{}' contains duplicate path '{}'",
                    self.name, path
                )));
            }

            if let Resource::File {
                compress_info,
                content_offset,
                ..
            } = resource
            {
                let data_pool_len = data_pool_len.ok_or_else(|| {
                    Error::InvalidReference(format!(
                        "resource group '{}' contains files but no data pool section is present",
                        self.name
                    ))
                })?;
                let end = content_offset
                    .checked_add(compress_info.compressed_size)
                    .ok_or_else(|| {
                        Error::InvalidReference("resource content offset overflow".to_string())
                    })?;
                if end > data_pool_len {
                    return Err(Error::InvalidReference(format!(
                        "resource '{}' points outside the data pool",
                        path
                    )));
                }
            }
        }

        Ok(())
    }
}

impl Resource {
    const TAG_FILE: u32 = 0x0053_4552;
    const TAG_DIRECTORY: u32 = 0x0052_4944;
    const TAG_SYMBOLIC_LINK: u32 = 0x4c4d_5953;

    fn path(&self) -> &ResourcePath {
        match self {
            Resource::File { path, .. }
            | Resource::Directory { path, .. }
            | Resource::SymbolicLink { path, .. } => path,
        }
    }
}

impl ResourcePath {
    fn resolve(&self, string_pool: Option<&StringPool>) -> Result<String, Error> {
        match self {
            ResourcePath::String(path) => {
                validate_resource_path(path)?;
                Ok(path.clone())
            }
            ResourcePath::Ref {
                directory_index,
                file_name_index,
            } => {
                let string_pool = string_pool.ok_or_else(|| {
                    Error::InvalidReference(
                        "resource path uses string-pool references but no string pool exists"
                            .to_string(),
                    )
                })?;
                let directory = string_pool.get(*directory_index).ok_or_else(|| {
                    Error::InvalidReference(format!(
                        "invalid string pool index {} in resource path",
                        directory_index
                    ))
                })?;
                let file_name = string_pool.get(*file_name_index).ok_or_else(|| {
                    Error::InvalidReference(format!(
                        "invalid string pool index {} in resource path",
                        file_name_index
                    ))
                })?;
                let path = if directory.is_empty() {
                    file_name.to_string()
                } else {
                    format!("{directory}/{file_name}")
                };
                validate_resource_path(&path)?;
                Ok(path)
            }
        }
    }
}

impl Timestamp {
    /// Validates the timestamp against the format's `[0, 1_000_000_000)` nanosecond range.
    pub fn validate(&self) -> Result<(), Error> {
        if self.nanos >= 1_000_000_000 {
            return Err(Error::InvalidValue(
                "timestamp nanos must be in the range [0, 1_000_000_000)",
            ));
        }
        Ok(())
    }
}

fn read_resource_group<R: DataReader>(reader: &mut R) -> Result<ResourceGroup, Error> {
    let magic = reader.read_u32_le()?;
    if magic != ResourceGroup::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: ResourceGroup::MAGIC_NUMBER as u64,
            actual: magic as u64,
        });
    }

    let name = reader.read_string()?;
    let fields = read_len_prefixed_vec(reader, read_tagged_field_u32)?;
    let resources_count = read_usize(reader.read_vuint()?)?;
    let (resources_compression, resources_data) = read_compressed_blob(reader)?;
    let mut resources_reader = ArrayDataReader::new(&resources_data);
    let mut resources = Vec::with_capacity(resources_count);
    for _ in 0..resources_count {
        resources.push(read_resource(&mut resources_reader)?);
    }
    ensure_fully_consumed(&resources_reader, "resource group payload")?;

    Ok(ResourceGroup {
        name,
        fields,
        resources_compression,
        resources,
    })
}

fn write_resource_group(writer: &mut VecDataWriter, group: &ResourceGroup) -> Result<(), Error> {
    writer.write_u32_le(ResourceGroup::MAGIC_NUMBER);
    writer.write_string(&group.name);
    write_len_prefixed_slice(writer, &group.fields, write_tagged_field_u32)?;
    writer.write_vuint(group.resources.len() as u64);

    let mut payload = VecDataWriter::new();
    for resource in &group.resources {
        write_resource(&mut payload, resource)?;
    }
    write_compressed_blob(writer, &group.resources_compression, &payload.into_inner())?;
    Ok(())
}

fn read_resource<R: DataReader>(reader: &mut R) -> Result<Resource, Error> {
    let tag = reader.read_u32_le()?;
    match tag {
        Resource::TAG_FILE => Ok(Resource::File {
            path: read_resource_path(reader)?,
            compress_info: read_compress_info(reader)?,
            content_offset: reader.read_vuint()?,
            fields: read_len_prefixed_vec(reader, read_resource_field)?,
        }),
        Resource::TAG_DIRECTORY => Ok(Resource::Directory {
            path: read_resource_path(reader)?,
            fields: read_len_prefixed_vec(reader, read_resource_field)?,
        }),
        Resource::TAG_SYMBOLIC_LINK => Ok(Resource::SymbolicLink {
            path: read_resource_path(reader)?,
            target: read_resource_path(reader)?,
            fields: read_len_prefixed_vec(reader, read_resource_field)?,
        }),
        _ => Err(Error::UnknownEnumValue {
            name: "resource type",
            value: tag as u64,
        }),
    }
}

fn write_resource(writer: &mut VecDataWriter, resource: &Resource) -> Result<(), Error> {
    match resource {
        Resource::File {
            path,
            compress_info,
            content_offset,
            fields,
        } => {
            writer.write_u32_le(Resource::TAG_FILE);
            write_resource_path(writer, path)?;
            write_compress_info(writer, compress_info)?;
            writer.write_vuint(*content_offset);
            write_len_prefixed_slice(writer, fields, write_resource_field)?;
        }
        Resource::Directory { path, fields } => {
            writer.write_u32_le(Resource::TAG_DIRECTORY);
            write_resource_path(writer, path)?;
            write_len_prefixed_slice(writer, fields, write_resource_field)?;
        }
        Resource::SymbolicLink {
            path,
            target,
            fields,
        } => {
            writer.write_u32_le(Resource::TAG_SYMBOLIC_LINK);
            write_resource_path(writer, path)?;
            write_resource_path(writer, target)?;
            write_len_prefixed_slice(writer, fields, write_resource_field)?;
        }
    }
    Ok(())
}

fn read_resource_path<R: DataReader>(reader: &mut R) -> Result<ResourcePath, Error> {
    let length = reader.read_vuint()?;
    if length == 0 {
        Ok(ResourcePath::Ref {
            directory_index: reader.read_vuint()?,
            file_name_index: reader.read_vuint()?,
        })
    } else {
        let body = reader.read_u8_array(read_usize(length)?)?;
        let path = String::from_utf8(body.into_vec())?;
        validate_resource_path(&path)?;
        Ok(ResourcePath::String(path))
    }
}

fn write_resource_path(writer: &mut VecDataWriter, path: &ResourcePath) -> Result<(), Error> {
    match path {
        ResourcePath::String(path) => {
            validate_resource_path(path)?;
            writer.write_vuint(path.len() as u64);
            writer.write_all(path.as_bytes());
        }
        ResourcePath::Ref {
            directory_index,
            file_name_index,
        } => {
            writer.write_vuint(0);
            writer.write_vuint(*directory_index);
            writer.write_vuint(*file_name_index);
        }
    }
    Ok(())
}

fn read_resource_field<R: DataReader>(reader: &mut R) -> Result<ResourceField, Error> {
    let tag = reader.read_u8()?;
    match tag {
        0x01 => {
            let payload = reader.read_bytes()?;
            let mut payload_reader = ArrayDataReader::new(payload.as_ref());
            let checksum = read_checksum(&mut payload_reader)?;
            ensure_fully_consumed(&payload_reader, "resource checksum field")?;
            Ok(ResourceField::Checksum(checksum))
        }
        0x02 => Ok(ResourceField::Comment(reader.read_string()?)),
        0x03 => Ok(ResourceField::FileCreateTime(read_timestamp_payload(
            reader.read_bytes()?.as_ref(),
            "resource creation timestamp",
        )?)),
        0x04 => Ok(ResourceField::FileModifyTime(read_timestamp_payload(
            reader.read_bytes()?.as_ref(),
            "resource modification timestamp",
        )?)),
        0x05 => Ok(ResourceField::FileAccessTime(read_timestamp_payload(
            reader.read_bytes()?.as_ref(),
            "resource access timestamp",
        )?)),
        0x06 => {
            let payload = reader.read_bytes()?;
            let mut payload_reader = ArrayDataReader::new(payload.as_ref());
            let permissions = DataReader::read_u16_le(&mut payload_reader)?;
            ensure_fully_consumed(&payload_reader, "POSIX permission field")?;
            Ok(ResourceField::PosixFilePermissions(permissions))
        }
        0x7f => {
            let payload = reader.read_bytes()?;
            let mut payload_reader = ArrayDataReader::new(payload.as_ref());
            let name = DataReader::read_string(&mut payload_reader)?;
            let content = DataReader::read_bytes(&mut payload_reader)?;
            ensure_fully_consumed(&payload_reader, "custom resource field")?;
            Ok(ResourceField::Custom { name, content })
        }
        _ => Err(Error::UnknownEnumValue {
            name: "resource field",
            value: tag as u64,
        }),
    }
}

fn write_resource_field(writer: &mut VecDataWriter, field: &ResourceField) -> Result<(), Error> {
    match field {
        ResourceField::Checksum(checksum) => {
            writer.write_u8(0x01);
            write_payload(writer, |payload| write_checksum(payload, checksum))?;
        }
        ResourceField::Comment(comment) => {
            writer.write_u8(0x02);
            writer.write_string(comment);
        }
        ResourceField::FileCreateTime(timestamp) => {
            writer.write_u8(0x03);
            write_payload(writer, |payload| write_timestamp(payload, timestamp))?;
        }
        ResourceField::FileModifyTime(timestamp) => {
            writer.write_u8(0x04);
            write_payload(writer, |payload| write_timestamp(payload, timestamp))?;
        }
        ResourceField::FileAccessTime(timestamp) => {
            writer.write_u8(0x05);
            write_payload(writer, |payload| write_timestamp(payload, timestamp))?;
        }
        ResourceField::PosixFilePermissions(permissions) => {
            writer.write_u8(0x06);
            write_payload(writer, |payload| {
                payload.write_u16_le(*permissions);
                Ok(())
            })?;
        }
        ResourceField::Custom { name, content } => {
            writer.write_u8(0x7f);
            write_payload(writer, |payload| {
                payload.write_string(name);
                payload.write_bytes(content);
                Ok(())
            })?;
        }
    }
    Ok(())
}

fn read_timestamp_payload(bytes: &[u8], name: &'static str) -> Result<Timestamp, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let timestamp = read_timestamp(&mut reader)?;
    ensure_fully_consumed(&reader, name)?;
    Ok(timestamp)
}

fn read_timestamp<R: DataReader>(reader: &mut R) -> Result<Timestamp, Error> {
    let timestamp = Timestamp {
        epoch_second: reader.read_i64_le()?,
        nanos: reader.read_u32_le()?,
    };
    timestamp.validate()?;
    Ok(timestamp)
}

fn write_timestamp(writer: &mut VecDataWriter, timestamp: &Timestamp) -> Result<(), Error> {
    timestamp.validate()?;
    writer.write_i64_le(timestamp.epoch_second);
    writer.write_u32_le(timestamp.nanos);
    Ok(())
}

pub(crate) fn validate_resource_path(path: &str) -> Result<(), Error> {
    if path.is_empty() || path.starts_with('/') || path.ends_with('/') {
        return Err(Error::InvalidValue(
            "resource path must not be empty or start/end with '/'",
        ));
    }

    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(Error::InvalidValue(
                "resource path segments must not be empty, '.' or '..'",
            ));
        }
    }

    Ok(())
}

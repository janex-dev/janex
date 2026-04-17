// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::janex::{
    ConfigField, ConfigGroup, JavaAgent, ResourceGroupReference, RootConfigGroupSection,
    ensure_fully_consumed, read_checksum, read_len_prefixed_vec, write_checksum,
    write_len_prefixed_slice, write_payload,
};
use std::collections::HashSet;

const DEFAULT_MAVEN_REPOSITORY: &str = "https://repo1.maven.org/maven2";

pub(crate) fn parse(bytes: &[u8]) -> Result<RootConfigGroupSection, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = DataReader::read_u64_le(&mut reader)?;
    if magic != RootConfigGroupSection::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: RootConfigGroupSection::MAGIC_NUMBER,
            actual: magic,
        });
    }
    let root_group = read_config_group(&mut reader)?;
    ensure_fully_consumed(&reader, "root config group section")?;
    Ok(RootConfigGroupSection { root_group })
}

pub(crate) fn encode(
    writer: &mut VecDataWriter,
    section: &RootConfigGroupSection,
) -> Result<(), Error> {
    writer.write_u64_le(RootConfigGroupSection::MAGIC_NUMBER);
    write_config_group(writer, &section.root_group)
}

impl RootConfigGroupSection {
    pub const MAGIC_NUMBER: u64 = 0x5055_4f52_4747_4643;
}

impl ConfigGroup {
    pub const MAGIC_NUMBER: u32 = 0x5052_4743;

    pub(crate) fn validate(&self, local_group_names: &HashSet<String>) -> Result<(), Error> {
        for field in &self.fields {
            match field {
                ConfigField::Condition(value)
                | ConfigField::MainClass(value)
                | ConfigField::MainModule(value) => {
                    if value.is_empty() {
                        return Err(Error::InvalidValue(
                            "configuration strings must not be empty",
                        ));
                    }
                }
                ConfigField::ModulePath(items) | ConfigField::ClassPath(items) => {
                    for item in items {
                        item.validate(local_group_names)?;
                    }
                }
                ConfigField::Agents(items) => {
                    for item in items {
                        item.reference.validate(local_group_names)?;
                    }
                }
                ConfigField::JvmOptions(options) => {
                    if options.iter().any(String::is_empty) {
                        return Err(Error::InvalidValue("JVM options must not be empty"));
                    }
                }
                ConfigField::SubGroups(groups) => {
                    for group in groups {
                        group.validate(local_group_names)?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl ResourceGroupReference {
    const TAG_LOCAL: u32 = 0x0043_4f4c;
    const TAG_MAVEN: u32 = 0x0056_4147;

    fn validate(&self, local_group_names: &HashSet<String>) -> Result<(), Error> {
        match self {
            ResourceGroupReference::Local { group_name } => {
                if !local_group_names.is_empty() && !local_group_names.contains(group_name) {
                    return Err(Error::InvalidReference(format!(
                        "unknown local resource group '{}'",
                        group_name
                    )));
                }
            }
            ResourceGroupReference::Maven {
                gav,
                repository,
                checksum: _,
            } => {
                if gav.is_empty() || repository.is_empty() {
                    return Err(Error::InvalidValue("Maven references must not be empty"));
                }
            }
        }
        Ok(())
    }
}

fn read_config_group<R: DataReader>(reader: &mut R) -> Result<ConfigGroup, Error> {
    let magic = reader.read_u32_le()?;
    if magic != ConfigGroup::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: ConfigGroup::MAGIC_NUMBER as u64,
            actual: magic as u64,
        });
    }

    Ok(ConfigGroup {
        fields: read_len_prefixed_vec(reader, read_config_field)?,
    })
}

fn write_config_group(writer: &mut VecDataWriter, group: &ConfigGroup) -> Result<(), Error> {
    writer.write_u32_le(ConfigGroup::MAGIC_NUMBER);
    write_len_prefixed_slice(writer, &group.fields, write_config_field)
}

fn read_config_field<R: DataReader>(reader: &mut R) -> Result<ConfigField, Error> {
    let field_type = reader.read_u32_le()?;
    Ok(match field_type {
        0x444e_4f43 => ConfigField::Condition(reader.read_string()?),
        0x534c_434d => ConfigField::MainClass(reader.read_string()?),
        0x444f_4d4d => ConfigField::MainModule(reader.read_string()?),
        0x5044_4f4d => {
            let payload = reader.read_bytes()?;
            let mut payload_reader = ArrayDataReader::new(payload.as_ref());
            let items = read_len_prefixed_vec(&mut payload_reader, read_resource_group_reference)?;
            ensure_fully_consumed(&payload_reader, "module path config field")?;
            ConfigField::ModulePath(items)
        }
        0x5053_4c43 => {
            let payload = reader.read_bytes()?;
            let mut payload_reader = ArrayDataReader::new(payload.as_ref());
            let items = read_len_prefixed_vec(&mut payload_reader, read_resource_group_reference)?;
            ensure_fully_consumed(&payload_reader, "class path config field")?;
            ConfigField::ClassPath(items)
        }
        0x544e_4741 => {
            let payload = reader.read_bytes()?;
            let mut payload_reader = ArrayDataReader::new(payload.as_ref());
            let items = read_len_prefixed_vec(&mut payload_reader, read_java_agent)?;
            ensure_fully_consumed(&payload_reader, "agents config field")?;
            ConfigField::Agents(items)
        }
        0x5450_4f4a => {
            let payload = reader.read_bytes()?;
            let mut payload_reader = ArrayDataReader::new(payload.as_ref());
            let options = read_len_prefixed_vec(&mut payload_reader, |reader| {
                DataReader::read_string(reader)
            })?;
            ensure_fully_consumed(&payload_reader, "JVM options config field")?;
            ConfigField::JvmOptions(options)
        }
        0x5052_4753 => {
            let payload = reader.read_bytes()?;
            let mut payload_reader = ArrayDataReader::new(payload.as_ref());
            let groups = read_len_prefixed_vec(&mut payload_reader, read_config_group)?;
            ensure_fully_consumed(&payload_reader, "subgroup config field")?;
            ConfigField::SubGroups(groups)
        }
        _ => {
            return Err(Error::UnknownEnumValue {
                name: "config field",
                value: field_type as u64,
            });
        }
    })
}

fn write_config_field(writer: &mut VecDataWriter, field: &ConfigField) -> Result<(), Error> {
    match field {
        ConfigField::Condition(value) => {
            writer.write_u32_le(0x444e_4f43);
            writer.write_string(value);
        }
        ConfigField::MainClass(value) => {
            writer.write_u32_le(0x534c_434d);
            writer.write_string(value);
        }
        ConfigField::MainModule(value) => {
            writer.write_u32_le(0x444f_4d4d);
            writer.write_string(value);
        }
        ConfigField::ModulePath(items) => {
            writer.write_u32_le(0x5044_4f4d);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, items, write_resource_group_reference)
            })?;
        }
        ConfigField::ClassPath(items) => {
            writer.write_u32_le(0x5053_4c43);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, items, write_resource_group_reference)
            })?;
        }
        ConfigField::Agents(items) => {
            writer.write_u32_le(0x544e_4741);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, items, write_java_agent)
            })?;
        }
        ConfigField::JvmOptions(options) => {
            writer.write_u32_le(0x5450_4f4a);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, options, |writer, value| {
                    writer.write_string(value);
                    Ok(())
                })
            })?;
        }
        ConfigField::SubGroups(groups) => {
            writer.write_u32_le(0x5052_4753);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, groups, write_config_group)
            })?;
        }
    }
    Ok(())
}

fn read_resource_group_reference<R: DataReader>(
    reader: &mut R,
) -> Result<ResourceGroupReference, Error> {
    let tag = reader.read_u32_le()?;
    match tag {
        ResourceGroupReference::TAG_LOCAL => Ok(ResourceGroupReference::Local {
            group_name: reader.read_string()?,
        }),
        ResourceGroupReference::TAG_MAVEN => {
            let gav = reader.read_string()?;
            let repository = reader.read_string()?;
            Ok(ResourceGroupReference::Maven {
                gav,
                repository: if repository.is_empty() {
                    DEFAULT_MAVEN_REPOSITORY.to_string()
                } else {
                    repository
                },
                checksum: read_checksum(reader)?,
            })
        }
        _ => Err(Error::UnknownEnumValue {
            name: "resource group reference type",
            value: tag as u64,
        }),
    }
}

fn write_resource_group_reference(
    writer: &mut VecDataWriter,
    reference: &ResourceGroupReference,
) -> Result<(), Error> {
    match reference {
        ResourceGroupReference::Local { group_name } => {
            writer.write_u32_le(ResourceGroupReference::TAG_LOCAL);
            writer.write_string(group_name);
        }
        ResourceGroupReference::Maven {
            gav,
            repository,
            checksum,
        } => {
            writer.write_u32_le(ResourceGroupReference::TAG_MAVEN);
            writer.write_string(gav);
            writer.write_string(repository);
            write_checksum(writer, checksum)?;
        }
    }
    Ok(())
}

fn read_java_agent<R: DataReader>(reader: &mut R) -> Result<JavaAgent, Error> {
    Ok(JavaAgent {
        reference: read_resource_group_reference(reader)?,
        option: reader.read_string()?,
    })
}

fn write_java_agent(writer: &mut VecDataWriter, agent: &JavaAgent) -> Result<(), Error> {
    write_resource_group_reference(writer, &agent.reference)?;
    writer.write_string(&agent.option);
    Ok(())
}

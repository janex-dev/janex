// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::byteorder::LittleEndian;
use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::string_pool::StringPool;
use sha2::{Digest, Sha256, Sha512};
use sm3::Sm3;
use std::collections::HashSet;
use xxhash_rust::xxh64::xxh64;

const DEFAULT_MAVEN_REPOSITORY: &str = "https://repo1.maven.org/maven2";
const CURRENT_MAJOR_VERSION: u32 = 0;
const CURRENT_MINOR_VERSION: u32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JanexFile {
    pub major_version: u32,
    pub minor_version: u32,
    pub flags: u64,
    pub fields: Vec<TaggedField<u32>>,
    pub verification: VerificationInfo,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: u64,
    pub options: Vec<TaggedField<u32>>,
    pub checksum: Checksum,
    pub content: SectionContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionContent {
    Padding(Box<[u8]>),
    RootConfigGroup(RootConfigGroupSection),
    ResourceGroups(ResourceGroupsSection),
    StringPool(StringPoolSection),
    DataPool(DataPoolSection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootConfigGroupSection {
    pub root_group: ConfigGroup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceGroupsSection {
    pub groups: Vec<ResourceGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringPoolSection {
    pub compression: CompressInfo,
    pub strings: StringPool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPoolSection {
    pub bytes: Box<[u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceGroup {
    pub name: String,
    pub fields: Vec<TaggedField<u32>>,
    pub resources_compression: CompressInfo,
    pub resources: Vec<Resource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigGroup {
    pub fields: Vec<ConfigField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigField {
    Condition(String),
    MainClass(String),
    MainModule(String),
    ModulePath(Vec<ResourceGroupReference>),
    ClassPath(Vec<ResourceGroupReference>),
    Agents(Vec<JavaAgent>),
    JvmOptions(Vec<String>),
    SubGroups(Vec<ConfigGroup>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceGroupReference {
    Local {
        group_name: String,
    },
    Maven {
        gav: String,
        repository: String,
        checksum: Checksum,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaAgent {
    pub reference: ResourceGroupReference,
    pub option: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    File {
        path: ResourcePath,
        compress_info: CompressInfo,
        content_offset: u64,
        fields: Vec<ResourceField>,
    },
    Directory {
        path: ResourcePath,
        fields: Vec<ResourceField>,
    },
    SymbolicLink {
        path: ResourcePath,
        target: ResourcePath,
        fields: Vec<ResourceField>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourcePath {
    String(String),
    Ref {
        directory_index: u64,
        file_name_index: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceField {
    Checksum(Checksum),
    Comment(String),
    FileCreateTime(Timestamp),
    FileModifyTime(Timestamp),
    FileAccessTime(Timestamp),
    PosixFilePermissions(u16),
    Custom { name: String, content: Box<[u8]> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub epoch_second: i64,
    pub nanos: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedField<T> {
    pub tag: T,
    pub payload: Box<[u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressInfo {
    pub method: CompressMethod,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub options: Box<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressMethod {
    None = 0,
    Composite = 1,
    Classfile = 2,
    Zstd = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    pub algorithm: ChecksumAlgorithm,
    pub checksum: Box<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ChecksumAlgorithm {
    None = 0,
    Xxh64 = 0x0101,
    Sha256 = 0x8101,
    Sha512 = 0x8102,
    Sm3 = 0x8301,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationInfo {
    None,
    Checksum(Checksum),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum SectionType {
    Padding = 0x0047_4e49_4444_4150,
    ExternalHeader = 0x4441_4548_4c54_5845,
    ExternalTail = 0x4c49_4154_4c54_5845,
    FileMetadata = 0x4154_4144_4154_454d,
    Attributes = 0x2e53_4249_5254_5441,
    DataPool = 0x4c4f_4f50_4154_4144,
    RootConfigGroup = 0x5055_4f52_4747_4643,
    ResourceGroups = 0x0053_5052_4753_4552,
    StringPool = 0x004c_4f4f_5052_5453,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SectionInfoRecord {
    section_type: SectionType,
    id: u64,
    options: Vec<TaggedField<u32>>,
    length: u64,
    checksum: Checksum,
}

#[derive(Debug)]
struct ParsedMetadata {
    major_version: u32,
    minor_version: u32,
    flags: u64,
    section_table: Vec<SectionInfoRecord>,
    fields: Vec<TaggedField<u32>>,
    verification: VerificationInfo,
    metadata_length: u64,
    file_length: u64,
    verification_offset: usize,
}

impl JanexFile {
    pub const MAGIC_NUMBER: u64 = 0x5050_4158_454e_414a;
    pub const FILE_METADATA_MAGIC_NUMBER: u64 = 0x4154_4144_4154_454d;
    pub const END_MARK: u64 = 0x444e_4558_454e_414a;

    pub fn new(sections: Vec<Section>) -> Self {
        Self {
            major_version: CURRENT_MAJOR_VERSION,
            minor_version: CURRENT_MINOR_VERSION,
            flags: 0,
            fields: Vec::new(),
            verification: VerificationInfo::None,
            sections,
        }
    }

    pub fn read(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 24 {
            return Err(Error::UnexpectedEndOfFile);
        }

        let footer_offset = bytes.len() - 24;
        let mut footer_reader = ArrayDataReader::new(&bytes[footer_offset..]);
        let end_mark = read_le_u64(&mut footer_reader)?;
        if end_mark != Self::END_MARK {
            return Err(Error::InvalidMagicNumber {
                expected: Self::END_MARK,
                actual: end_mark,
            });
        }

        let metadata_length = read_usize(read_le_u64(&mut footer_reader)?)?;
        let file_length = read_usize(read_le_u64(&mut footer_reader)?)?;
        if file_length > bytes.len() {
            return Err(Error::InvalidSectionLayout(
                "file_length is larger than the input size".to_string(),
            ));
        }

        let file_start = bytes.len() - file_length;
        let file_end = file_start + file_length;
        let metadata_start = file_end
            .checked_sub(metadata_length)
            .ok_or_else(|| Error::InvalidSectionLayout("metadata_length underflow".to_string()))?;
        if metadata_start < file_start + 8 {
            return Err(Error::InvalidSectionLayout(
                "metadata section overlaps the file header".to_string(),
            ));
        }

        let mut file_reader = ArrayDataReader::new(&bytes[file_start..metadata_start]);
        let magic = read_le_u64(&mut file_reader)?;
        if magic != Self::MAGIC_NUMBER {
            return Err(Error::InvalidMagicNumber {
                expected: Self::MAGIC_NUMBER,
                actual: magic,
            });
        }

        let metadata_bytes = &bytes[metadata_start..file_end];
        let parsed_metadata = read_metadata(metadata_bytes)?;
        if parsed_metadata.metadata_length != metadata_length as u64 {
            return Err(Error::InvalidSectionLayout(
                "metadata_length does not match the footer".to_string(),
            ));
        }
        if parsed_metadata.file_length != file_length as u64 {
            return Err(Error::InvalidSectionLayout(
                "file_length does not match the footer".to_string(),
            ));
        }
        if parsed_metadata.major_version != CURRENT_MAJOR_VERSION
            || parsed_metadata.minor_version != CURRENT_MINOR_VERSION
        {
            return Err(Error::UnsupportedFeature("unsupported Janex file version"));
        }
        parsed_metadata.verify(metadata_bytes)?;

        let mut sections = Vec::with_capacity(parsed_metadata.section_table.len());
        for section_info in &parsed_metadata.section_table {
            let section_bytes =
                read_le_u8_array(&mut file_reader, read_usize(section_info.length)?)?;
            verify_checksum(&section_info.checksum, section_bytes.as_ref(), "section")?;
            let content = parse_section_content(section_info.section_type, section_bytes.as_ref())?;
            sections.push(Section {
                id: section_info.id,
                options: section_info.options.clone(),
                checksum: section_info.checksum.clone(),
                content,
            });
        }

        if remaining_le(&file_reader) != 0 {
            return Err(Error::InvalidSectionLayout(
                "section table does not consume the full file body".to_string(),
            ));
        }

        let file = Self {
            major_version: parsed_metadata.major_version,
            minor_version: parsed_metadata.minor_version,
            flags: parsed_metadata.flags,
            fields: parsed_metadata.fields,
            verification: parsed_metadata.verification,
            sections,
        };
        file.validate()?;
        Ok(file)
    }

    pub fn write(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;

        let mut section_infos = Vec::with_capacity(self.sections.len());
        let mut encoded_sections = Vec::with_capacity(self.sections.len());
        for section in &self.sections {
            let bytes = encode_section_content(&section.content)?;
            let checksum = compute_checksum(section.checksum.algorithm, &bytes)?;
            section_infos.push(SectionInfoRecord {
                section_type: section.content.section_type(),
                id: section.id,
                options: section.options.clone(),
                length: bytes.len() as u64,
                checksum,
            });
            encoded_sections.push(bytes);
        }

        let mut metadata_prefix = VecDataWriter::<LittleEndian>::new();
        metadata_prefix.write_u64(Self::FILE_METADATA_MAGIC_NUMBER);
        metadata_prefix.write_u32(self.major_version);
        metadata_prefix.write_u32(self.minor_version);
        metadata_prefix.write_u64(self.flags);
        write_len_prefixed_slice(
            &mut metadata_prefix,
            &section_infos,
            write_section_info_record,
        )?;
        write_len_prefixed_slice(&mut metadata_prefix, &self.fields, write_tagged_field_u32)?;
        let metadata_prefix = metadata_prefix.into_inner();

        let verification = match &self.verification {
            VerificationInfo::None => VerificationInfo::None,
            VerificationInfo::Checksum(checksum) => {
                VerificationInfo::Checksum(compute_checksum(checksum.algorithm, &metadata_prefix)?)
            }
        };
        let verification_bytes = encode_verification_info(&verification)?;
        let metadata_length = metadata_prefix.len() + verification_bytes.len() + 24;
        let sections_length: usize = encoded_sections.iter().map(Vec::len).sum();
        let file_length = 8 + sections_length + metadata_length;

        let mut writer = VecDataWriter::<LittleEndian>::with_capacity(file_length);
        writer.write_u64(Self::MAGIC_NUMBER);
        for section in &encoded_sections {
            writer.write_all(section);
        }
        writer.write_all(&metadata_prefix);
        writer.write_all(&verification_bytes);
        writer.write_u64(Self::END_MARK);
        writer.write_u64(metadata_length as u64);
        writer.write_u64(file_length as u64);
        Ok(writer.into_inner())
    }

    fn validate(&self) -> Result<(), Error> {
        let mut root_config_group_count = 0usize;
        let mut resource_groups_count = 0usize;
        let mut string_pool_count = 0usize;
        let mut data_pool_count = 0usize;
        let mut seen_section_keys = HashSet::with_capacity(self.sections.len());
        let mut seen_resource_group_names = HashSet::new();
        let mut local_group_names = HashSet::new();
        let mut string_pool_position = None;
        let mut resource_groups_position = None;
        let mut data_pool_len = None;
        let mut string_pool_ref = None;

        for (idx, section) in self.sections.iter().enumerate() {
            let key = (section.content.section_type() as u64, section.id);
            if !seen_section_keys.insert(key) {
                return Err(Error::InvalidSectionLayout(format!(
                    "duplicate section id {} for section type 0x{:016x}",
                    section.id,
                    section.content.section_type() as u64
                )));
            }

            match &section.content {
                SectionContent::Padding(_) => {}
                SectionContent::RootConfigGroup(_) => {
                    root_config_group_count += 1;
                }
                SectionContent::ResourceGroups(section) => {
                    resource_groups_count += 1;
                    resource_groups_position = Some(idx);
                    for group in &section.groups {
                        if !seen_resource_group_names.insert(group.name.clone()) {
                            return Err(Error::InvalidSectionLayout(format!(
                                "duplicate resource group name '{}'",
                                group.name
                            )));
                        }
                        local_group_names.insert(group.name.clone());
                    }
                }
                SectionContent::StringPool(section) => {
                    string_pool_count += 1;
                    string_pool_position = Some(idx);
                    string_pool_ref = Some(&section.strings);
                    if section.strings.is_empty() || section.strings.get(0) != Some("") {
                        return Err(Error::InvalidSectionLayout(
                            "string pool index 0 must be an empty string".to_string(),
                        ));
                    }
                }
                SectionContent::DataPool(section) => {
                    data_pool_count += 1;
                    data_pool_len = Some(section.bytes.len() as u64);
                }
            }
        }

        if root_config_group_count > 1
            || resource_groups_count > 1
            || string_pool_count > 1
            || data_pool_count > 1
        {
            return Err(Error::InvalidSectionLayout(
                "Janex files may contain at most one root config group, resource groups, string pool, and data pool section".to_string(),
            ));
        }

        if let (Some(string_pool_position), Some(resource_groups_position)) =
            (string_pool_position, resource_groups_position)
        {
            if string_pool_position >= resource_groups_position {
                return Err(Error::InvalidSectionLayout(
                    "the string pool section must appear before the resource groups section"
                        .to_string(),
                ));
            }
        }

        for section in &self.sections {
            match &section.content {
                SectionContent::RootConfigGroup(section) => {
                    section.root_group.validate(&local_group_names)?;
                }
                SectionContent::ResourceGroups(resource_groups) => {
                    for group in &resource_groups.groups {
                        group.validate(string_pool_ref, data_pool_len)?;
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }
}

impl Default for JanexFile {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl SectionContent {
    fn section_type(&self) -> SectionType {
        match self {
            SectionContent::Padding(_) => SectionType::Padding,
            SectionContent::RootConfigGroup(_) => SectionType::RootConfigGroup,
            SectionContent::ResourceGroups(_) => SectionType::ResourceGroups,
            SectionContent::StringPool(_) => SectionType::StringPool,
            SectionContent::DataPool(_) => SectionType::DataPool,
        }
    }
}

impl RootConfigGroupSection {
    pub const MAGIC_NUMBER: u64 = 0x5055_4f52_4747_4643;
}

impl ResourceGroupsSection {
    pub const MAGIC_NUMBER: u64 = 0x0053_5052_4753_4552;
}

impl StringPoolSection {
    pub const MAGIC_NUMBER: u64 = 0x004c_4f4f_5052_5453;

    pub fn new(strings: StringPool) -> Self {
        Self {
            compression: CompressInfo::none(),
            strings,
        }
    }
}

impl DataPoolSection {
    pub const MAGIC_NUMBER: u64 = 0x4c4f_4f50_4154_4144;
}

impl ResourceGroup {
    pub const MAGIC_NUMBER: u32 = 0x4753_4552;
}

impl ConfigGroup {
    pub const MAGIC_NUMBER: u32 = 0x5052_4743;
}

impl ResourceGroupReference {
    const TAG_LOCAL: u32 = 0x0043_4f4c;
    const TAG_MAVEN: u32 = 0x0056_4147;
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

impl Timestamp {
    pub fn validate(&self) -> Result<(), Error> {
        if self.nanos >= 1_000_000_000 {
            return Err(Error::InvalidValue(
                "timestamp nanos must be in the range [0, 1_000_000_000)",
            ));
        }
        Ok(())
    }
}

impl TaggedField<u32> {
    pub fn new(tag: u32, payload: Vec<u8>) -> Self {
        Self {
            tag,
            payload: payload.into_boxed_slice(),
        }
    }
}

impl TaggedField<u8> {
    pub fn new(tag: u8, payload: Vec<u8>) -> Self {
        Self {
            tag,
            payload: payload.into_boxed_slice(),
        }
    }
}

impl CompressInfo {
    pub fn none() -> Self {
        Self {
            method: CompressMethod::None,
            uncompressed_size: 0,
            compressed_size: 0,
            options: Box::new([]),
        }
    }
}

impl Default for CompressInfo {
    fn default() -> Self {
        Self::none()
    }
}

impl Checksum {
    pub fn none() -> Self {
        Self {
            algorithm: ChecksumAlgorithm::None,
            checksum: Box::new([]),
        }
    }
}

impl Default for Checksum {
    fn default() -> Self {
        Self::none()
    }
}

impl ChecksumAlgorithm {
    fn expected_len(self) -> usize {
        match self {
            ChecksumAlgorithm::None => 0,
            ChecksumAlgorithm::Xxh64 => 8,
            ChecksumAlgorithm::Sha256 => 32,
            ChecksumAlgorithm::Sha512 => 64,
            ChecksumAlgorithm::Sm3 => 32,
        }
    }
}

impl TryFrom<u8> for CompressMethod {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(CompressMethod::None),
            1 => Ok(CompressMethod::Composite),
            2 => Ok(CompressMethod::Classfile),
            3 => Ok(CompressMethod::Zstd),
            _ => Err(Error::UnknownEnumValue {
                name: "compression method",
                value: value as u64,
            }),
        }
    }
}

impl TryFrom<u16> for ChecksumAlgorithm {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ChecksumAlgorithm::None),
            0x0101 => Ok(ChecksumAlgorithm::Xxh64),
            0x8101 => Ok(ChecksumAlgorithm::Sha256),
            0x8102 => Ok(ChecksumAlgorithm::Sha512),
            0x8301 => Ok(ChecksumAlgorithm::Sm3),
            _ => Err(Error::UnknownEnumValue {
                name: "checksum algorithm",
                value: value as u64,
            }),
        }
    }
}

impl TryFrom<u64> for SectionType {
    type Error = Error;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0x0047_4e49_4444_4150 => Ok(SectionType::Padding),
            0x4441_4548_4c54_5845 => Ok(SectionType::ExternalHeader),
            0x4c49_4154_4c54_5845 => Ok(SectionType::ExternalTail),
            0x4154_4144_4154_454d => Ok(SectionType::FileMetadata),
            0x2e53_4249_5254_5441 => Ok(SectionType::Attributes),
            0x4c4f_4f50_4154_4144 => Ok(SectionType::DataPool),
            0x5055_4f52_4747_4643 => Ok(SectionType::RootConfigGroup),
            0x0053_5052_4753_4552 => Ok(SectionType::ResourceGroups),
            0x004c_4f4f_5052_5453 => Ok(SectionType::StringPool),
            _ => Err(Error::UnknownEnumValue {
                name: "section type",
                value,
            }),
        }
    }
}

impl ResourceGroup {
    fn validate(
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

impl ConfigGroup {
    fn validate(&self, local_group_names: &HashSet<String>) -> Result<(), Error> {
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
                checksum,
            } => {
                if gav.is_empty() || repository.is_empty() {
                    return Err(Error::InvalidValue("Maven references must not be empty"));
                }
                validate_checksum_shape(checksum)?;
            }
        }
        Ok(())
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

impl ParsedMetadata {
    fn verify(&self, metadata_bytes: &[u8]) -> Result<(), Error> {
        match &self.verification {
            VerificationInfo::None => Ok(()),
            VerificationInfo::Checksum(checksum) => verify_checksum(
                checksum,
                &metadata_bytes[..self.verification_offset],
                "metadata",
            ),
        }
    }
}

fn read_metadata(bytes: &[u8]) -> Result<ParsedMetadata, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = read_le_u64(&mut reader)?;
    if magic != JanexFile::FILE_METADATA_MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: JanexFile::FILE_METADATA_MAGIC_NUMBER,
            actual: magic,
        });
    }

    let major_version = read_le_u32(&mut reader)?;
    let minor_version = read_le_u32(&mut reader)?;
    let flags = read_le_u64(&mut reader)?;
    let section_table = read_len_prefixed_vec(&mut reader, read_section_info_record)?;
    let fields = read_len_prefixed_vec(&mut reader, read_tagged_field_u32)?;
    let verification_offset = bytes.len() - remaining_le(&reader);
    let verification = read_verification_info(&mut reader)?;
    let end_mark = read_le_u64(&mut reader)?;
    if end_mark != JanexFile::END_MARK {
        return Err(Error::InvalidMagicNumber {
            expected: JanexFile::END_MARK,
            actual: end_mark,
        });
    }

    let metadata_length = read_le_u64(&mut reader)?;
    let file_length = read_le_u64(&mut reader)?;
    if remaining_le(&reader) != 0 {
        return Err(Error::InvalidSectionLayout(
            "metadata has trailing bytes".to_string(),
        ));
    }

    Ok(ParsedMetadata {
        major_version,
        minor_version,
        flags,
        section_table,
        fields,
        verification,
        metadata_length,
        file_length,
        verification_offset,
    })
}

fn parse_section_content(section_type: SectionType, bytes: &[u8]) -> Result<SectionContent, Error> {
    match section_type {
        SectionType::Padding => Ok(SectionContent::Padding(bytes.into())),
        SectionType::RootConfigGroup => Ok(SectionContent::RootConfigGroup(
            parse_root_config_group_section(bytes)?,
        )),
        SectionType::ResourceGroups => Ok(SectionContent::ResourceGroups(
            parse_resource_groups_section(bytes)?,
        )),
        SectionType::StringPool => Ok(SectionContent::StringPool(parse_string_pool_section(
            bytes,
        )?)),
        SectionType::DataPool => Ok(SectionContent::DataPool(parse_data_pool_section(bytes)?)),
        SectionType::ExternalHeader
        | SectionType::ExternalTail
        | SectionType::FileMetadata
        | SectionType::Attributes => Err(Error::UnsupportedFeature(
            "external header/tail, attributes, and nested metadata sections are not implemented",
        )),
    }
}

fn encode_section_content(section: &SectionContent) -> Result<Vec<u8>, Error> {
    let mut writer = VecDataWriter::<LittleEndian>::new();
    match section {
        SectionContent::Padding(bytes) => writer.write_all(bytes),
        SectionContent::RootConfigGroup(section) => {
            writer.write_u64(RootConfigGroupSection::MAGIC_NUMBER);
            write_config_group(&mut writer, &section.root_group)?;
        }
        SectionContent::ResourceGroups(section) => {
            writer.write_u64(ResourceGroupsSection::MAGIC_NUMBER);
            write_len_prefixed_slice(&mut writer, &section.groups, write_resource_group)?;
        }
        SectionContent::StringPool(section) => {
            writer.write_u64(StringPoolSection::MAGIC_NUMBER);
            writer.write_vuint(section.strings.len() as u64);
            let strings: Vec<&str> = section.strings.iter().collect();
            for string in &strings {
                writer.write_vuint(string.len() as u64);
            }
            let uncompressed = strings.concat().into_bytes();
            write_compressed_blob(&mut writer, &section.compression, &uncompressed)?;
        }
        SectionContent::DataPool(section) => {
            writer.write_u64(DataPoolSection::MAGIC_NUMBER);
            writer.write_all(&section.bytes);
        }
    }
    Ok(writer.into_inner())
}

fn parse_root_config_group_section(bytes: &[u8]) -> Result<RootConfigGroupSection, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = read_le_u64(&mut reader)?;
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

fn parse_resource_groups_section(bytes: &[u8]) -> Result<ResourceGroupsSection, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = read_le_u64(&mut reader)?;
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

fn parse_string_pool_section(bytes: &[u8]) -> Result<StringPoolSection, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = read_le_u64(&mut reader)?;
    if magic != StringPoolSection::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: StringPoolSection::MAGIC_NUMBER,
            actual: magic,
        });
    }

    let count = read_usize(read_le_vuint(&mut reader)?)?;
    let mut sizes = Vec::with_capacity(count);
    for _ in 0..count {
        sizes.push(read_usize(read_le_vuint(&mut reader)?)?);
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

fn parse_data_pool_section(bytes: &[u8]) -> Result<DataPoolSection, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = read_le_u64(&mut reader)?;
    if magic != DataPoolSection::MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: DataPoolSection::MAGIC_NUMBER,
            actual: magic,
        });
    }
    let remaining = remaining_le(&reader);
    Ok(DataPoolSection {
        bytes: read_le_u8_array(&mut reader, remaining)?,
    })
}

fn read_section_info_record<R: DataReader<LittleEndian>>(
    reader: &mut R,
) -> Result<SectionInfoRecord, Error> {
    Ok(SectionInfoRecord {
        section_type: SectionType::try_from(reader.read_u64()?)?,
        id: reader.read_vuint()?,
        options: read_len_prefixed_vec(reader, read_tagged_field_u32)?,
        length: reader.read_vuint()?,
        checksum: read_checksum(reader)?,
    })
}

fn write_section_info_record(
    writer: &mut VecDataWriter<LittleEndian>,
    record: &SectionInfoRecord,
) -> Result<(), Error> {
    writer.write_u64(record.section_type as u64);
    writer.write_vuint(record.id);
    write_len_prefixed_slice(writer, &record.options, write_tagged_field_u32)?;
    writer.write_vuint(record.length);
    write_checksum(writer, &record.checksum)?;
    Ok(())
}

fn read_verification_info<R: DataReader<LittleEndian>>(
    reader: &mut R,
) -> Result<VerificationInfo, Error> {
    let verification_type = reader.read_u8()?;
    let data = reader.read_bytes()?;
    match verification_type {
        0 => {
            if !data.is_empty() {
                return Err(Error::InvalidValue(
                    "VerificationType::None must not contain a payload",
                ));
            }
            Ok(VerificationInfo::None)
        }
        1 => {
            let mut payload_reader = ArrayDataReader::new(data.as_ref());
            let checksum = read_checksum(&mut payload_reader)?;
            ensure_fully_consumed(&payload_reader, "verification checksum")?;
            Ok(VerificationInfo::Checksum(checksum))
        }
        2 => Err(Error::UnsupportedFeature("OpenPGP verification")),
        3 => Err(Error::UnsupportedFeature("CMS verification")),
        _ => Err(Error::UnknownEnumValue {
            name: "verification type",
            value: verification_type as u64,
        }),
    }
}

fn encode_verification_info(verification: &VerificationInfo) -> Result<Vec<u8>, Error> {
    let mut writer = VecDataWriter::<LittleEndian>::new();
    match verification {
        VerificationInfo::None => {
            writer.write_u8(0);
            writer.write_bytes(&[]);
        }
        VerificationInfo::Checksum(checksum) => {
            writer.write_u8(1);
            write_payload(&mut writer, |payload| write_checksum(payload, checksum))?;
        }
    }
    Ok(writer.into_inner())
}

fn read_config_group<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<ConfigGroup, Error> {
    let magic = reader.read_u32()?;
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

fn write_config_group(
    writer: &mut VecDataWriter<LittleEndian>,
    group: &ConfigGroup,
) -> Result<(), Error> {
    writer.write_u32(ConfigGroup::MAGIC_NUMBER);
    write_len_prefixed_slice(writer, &group.fields, write_config_field)
}

fn read_config_field<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<ConfigField, Error> {
    let field_type = reader.read_u32()?;
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
            let options = read_len_prefixed_vec(&mut payload_reader, read_le_string)?;
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

fn write_config_field(
    writer: &mut VecDataWriter<LittleEndian>,
    field: &ConfigField,
) -> Result<(), Error> {
    match field {
        ConfigField::Condition(value) => {
            writer.write_u32(0x444e_4f43);
            writer.write_string(value);
        }
        ConfigField::MainClass(value) => {
            writer.write_u32(0x534c_434d);
            writer.write_string(value);
        }
        ConfigField::MainModule(value) => {
            writer.write_u32(0x444f_4d4d);
            writer.write_string(value);
        }
        ConfigField::ModulePath(items) => {
            writer.write_u32(0x5044_4f4d);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, items, write_resource_group_reference)
            })?;
        }
        ConfigField::ClassPath(items) => {
            writer.write_u32(0x5053_4c43);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, items, write_resource_group_reference)
            })?;
        }
        ConfigField::Agents(items) => {
            writer.write_u32(0x544e_4741);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, items, write_java_agent)
            })?;
        }
        ConfigField::JvmOptions(options) => {
            writer.write_u32(0x5450_4f4a);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, options, |writer, value| {
                    writer.write_string(value);
                    Ok(())
                })
            })?;
        }
        ConfigField::SubGroups(groups) => {
            writer.write_u32(0x5052_4753);
            write_payload(writer, |payload| {
                write_len_prefixed_slice(payload, groups, write_config_group)
            })?;
        }
    }
    Ok(())
}

fn read_resource_group_reference<R: DataReader<LittleEndian>>(
    reader: &mut R,
) -> Result<ResourceGroupReference, Error> {
    let tag = reader.read_u32()?;
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
    writer: &mut VecDataWriter<LittleEndian>,
    reference: &ResourceGroupReference,
) -> Result<(), Error> {
    match reference {
        ResourceGroupReference::Local { group_name } => {
            writer.write_u32(ResourceGroupReference::TAG_LOCAL);
            writer.write_string(group_name);
        }
        ResourceGroupReference::Maven {
            gav,
            repository,
            checksum,
        } => {
            writer.write_u32(ResourceGroupReference::TAG_MAVEN);
            writer.write_string(gav);
            writer.write_string(repository);
            write_checksum(writer, checksum)?;
        }
    }
    Ok(())
}

fn read_java_agent<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<JavaAgent, Error> {
    Ok(JavaAgent {
        reference: read_resource_group_reference(reader)?,
        option: reader.read_string()?,
    })
}

fn write_java_agent(
    writer: &mut VecDataWriter<LittleEndian>,
    agent: &JavaAgent,
) -> Result<(), Error> {
    write_resource_group_reference(writer, &agent.reference)?;
    writer.write_string(&agent.option);
    Ok(())
}

fn read_resource_group<R: DataReader<LittleEndian>>(
    reader: &mut R,
) -> Result<ResourceGroup, Error> {
    let magic = reader.read_u32()?;
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

fn write_resource_group(
    writer: &mut VecDataWriter<LittleEndian>,
    group: &ResourceGroup,
) -> Result<(), Error> {
    writer.write_u32(ResourceGroup::MAGIC_NUMBER);
    writer.write_string(&group.name);
    write_len_prefixed_slice(writer, &group.fields, write_tagged_field_u32)?;
    writer.write_vuint(group.resources.len() as u64);

    let mut payload = VecDataWriter::<LittleEndian>::new();
    for resource in &group.resources {
        write_resource(&mut payload, resource)?;
    }
    write_compressed_blob(writer, &group.resources_compression, &payload.into_inner())?;
    Ok(())
}

fn read_resource<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<Resource, Error> {
    let tag = reader.read_u32()?;
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

fn write_resource(
    writer: &mut VecDataWriter<LittleEndian>,
    resource: &Resource,
) -> Result<(), Error> {
    match resource {
        Resource::File {
            path,
            compress_info,
            content_offset,
            fields,
        } => {
            writer.write_u32(Resource::TAG_FILE);
            write_resource_path(writer, path)?;
            write_compress_info(writer, compress_info)?;
            writer.write_vuint(*content_offset);
            write_len_prefixed_slice(writer, fields, write_resource_field)?;
        }
        Resource::Directory { path, fields } => {
            writer.write_u32(Resource::TAG_DIRECTORY);
            write_resource_path(writer, path)?;
            write_len_prefixed_slice(writer, fields, write_resource_field)?;
        }
        Resource::SymbolicLink {
            path,
            target,
            fields,
        } => {
            writer.write_u32(Resource::TAG_SYMBOLIC_LINK);
            write_resource_path(writer, path)?;
            write_resource_path(writer, target)?;
            write_len_prefixed_slice(writer, fields, write_resource_field)?;
        }
    }
    Ok(())
}

fn read_resource_path<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<ResourcePath, Error> {
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

fn write_resource_path(
    writer: &mut VecDataWriter<LittleEndian>,
    path: &ResourcePath,
) -> Result<(), Error> {
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

fn read_resource_field<R: DataReader<LittleEndian>>(
    reader: &mut R,
) -> Result<ResourceField, Error> {
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
            let permissions = read_le_u16(&mut payload_reader)?;
            ensure_fully_consumed(&payload_reader, "POSIX permission field")?;
            Ok(ResourceField::PosixFilePermissions(permissions))
        }
        0x7f => {
            let payload = reader.read_bytes()?;
            let mut payload_reader = ArrayDataReader::new(payload.as_ref());
            let name = read_le_string(&mut payload_reader)?;
            let content = read_le_bytes(&mut payload_reader)?;
            ensure_fully_consumed(&payload_reader, "custom resource field")?;
            Ok(ResourceField::Custom { name, content })
        }
        _ => Err(Error::UnknownEnumValue {
            name: "resource field",
            value: tag as u64,
        }),
    }
}

fn write_resource_field(
    writer: &mut VecDataWriter<LittleEndian>,
    field: &ResourceField,
) -> Result<(), Error> {
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
                payload.write_u16(*permissions);
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

fn read_timestamp<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<Timestamp, Error> {
    let timestamp = Timestamp {
        epoch_second: read_le_i64(reader)?,
        nanos: read_le_u32(reader)?,
    };
    timestamp.validate()?;
    Ok(timestamp)
}

fn write_timestamp(
    writer: &mut VecDataWriter<LittleEndian>,
    timestamp: &Timestamp,
) -> Result<(), Error> {
    timestamp.validate()?;
    writer.write_i64(timestamp.epoch_second);
    writer.write_u32(timestamp.nanos);
    Ok(())
}

fn read_compress_info<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<CompressInfo, Error> {
    Ok(CompressInfo {
        method: CompressMethod::try_from(reader.read_u8()?)?,
        uncompressed_size: reader.read_vuint()?,
        compressed_size: reader.read_vuint()?,
        options: reader.read_bytes()?,
    })
}

fn write_compress_info(
    writer: &mut VecDataWriter<LittleEndian>,
    info: &CompressInfo,
) -> Result<(), Error> {
    writer.write_u8(info.method as u8);
    writer.write_vuint(info.uncompressed_size);
    writer.write_vuint(info.compressed_size);
    writer.write_bytes(&info.options);
    Ok(())
}

fn read_compressed_blob<R: DataReader<LittleEndian>>(
    reader: &mut R,
) -> Result<(CompressInfo, Vec<u8>), Error> {
    let info = read_compress_info(reader)?;
    let compressed_size = read_usize(info.compressed_size)?;
    let compressed = reader.read_u8_array(compressed_size)?;
    let data = decompress_bytes(&info, compressed.as_ref())?;
    if data.len() as u64 != info.uncompressed_size {
        return Err(Error::CompressionError(
            "uncompressed size does not match the declared size".to_string(),
        ));
    }
    Ok((info, data))
}

fn write_compressed_blob(
    writer: &mut VecDataWriter<LittleEndian>,
    info: &CompressInfo,
    uncompressed: &[u8],
) -> Result<(), Error> {
    let compressed = compress_bytes(info, uncompressed)?;
    let actual_info = CompressInfo {
        method: info.method,
        uncompressed_size: uncompressed.len() as u64,
        compressed_size: compressed.len() as u64,
        options: info.options.clone(),
    };
    write_compress_info(writer, &actual_info)?;
    writer.write_all(&compressed);
    Ok(())
}

fn read_checksum<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<Checksum, Error> {
    let algorithm = ChecksumAlgorithm::try_from(reader.read_u16()?)?;
    let reserved = reader.read_u8()?;
    if reserved != 0 {
        return Err(Error::InvalidValue("checksum reserved byte must be zero"));
    }

    let checksum = Checksum {
        algorithm,
        checksum: reader.read_bytes()?,
    };
    validate_checksum_shape(&checksum)?;
    Ok(checksum)
}

fn write_checksum(
    writer: &mut VecDataWriter<LittleEndian>,
    checksum: &Checksum,
) -> Result<(), Error> {
    validate_checksum_shape(checksum)?;
    writer.write_u16(checksum.algorithm as u16);
    writer.write_u8(0);
    writer.write_bytes(&checksum.checksum);
    Ok(())
}

fn validate_checksum_shape(checksum: &Checksum) -> Result<(), Error> {
    if checksum.checksum.len() != checksum.algorithm.expected_len() {
        return Err(Error::InvalidValue(
            "checksum payload length does not match its algorithm",
        ));
    }
    Ok(())
}

fn verify_checksum(checksum: &Checksum, bytes: &[u8], name: &'static str) -> Result<(), Error> {
    let expected = compute_checksum(checksum.algorithm, bytes)?;
    if expected.checksum != checksum.checksum {
        return Err(Error::VerificationFailed(format!(
            "{name} checksum mismatch"
        )));
    }
    Ok(())
}

fn compute_checksum(algorithm: ChecksumAlgorithm, bytes: &[u8]) -> Result<Checksum, Error> {
    let checksum = match algorithm {
        ChecksumAlgorithm::None => Vec::new(),
        ChecksumAlgorithm::Xxh64 => xxh64(bytes, 0).to_le_bytes().to_vec(),
        ChecksumAlgorithm::Sha256 => Sha256::digest(bytes).to_vec(),
        ChecksumAlgorithm::Sha512 => Sha512::digest(bytes).to_vec(),
        ChecksumAlgorithm::Sm3 => Sm3::digest(bytes).to_vec(),
    };
    Ok(Checksum {
        algorithm,
        checksum: checksum.into_boxed_slice(),
    })
}

fn compress_bytes(info: &CompressInfo, data: &[u8]) -> Result<Vec<u8>, Error> {
    match info.method {
        CompressMethod::None => Ok(data.to_vec()),
        CompressMethod::Zstd => zstd::stream::encode_all(data, 0).map_err(Error::from),
        CompressMethod::Composite => {
            let layers = parse_composite_layers(&info.options)?;
            let mut current = data.to_vec();
            for layer in &layers {
                current = compress_bytes(layer, &current)?;
            }
            Ok(current)
        }
        CompressMethod::Classfile => Err(Error::UnsupportedFeature(
            "classfile compression for arbitrary payloads",
        )),
    }
}

fn decompress_bytes(info: &CompressInfo, data: &[u8]) -> Result<Vec<u8>, Error> {
    match info.method {
        CompressMethod::None => Ok(data.to_vec()),
        CompressMethod::Zstd => zstd::stream::decode_all(data).map_err(Error::from),
        CompressMethod::Composite => {
            let layers = parse_composite_layers(&info.options)?;
            let mut current = data.to_vec();
            for layer in layers.iter().rev() {
                current = decompress_bytes(layer, &current)?;
            }
            Ok(current)
        }
        CompressMethod::Classfile => Err(Error::UnsupportedFeature(
            "classfile compression for arbitrary payloads",
        )),
    }
}

fn parse_composite_layers(bytes: &[u8]) -> Result<Vec<CompressInfo>, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let layers = read_len_prefixed_vec(&mut reader, read_compress_info)?;
    ensure_fully_consumed(&reader, "composite compression options")?;
    Ok(layers)
}

fn read_tagged_field_u32<R: DataReader<LittleEndian>>(
    reader: &mut R,
) -> Result<TaggedField<u32>, Error> {
    Ok(TaggedField {
        tag: reader.read_u32()?,
        payload: reader.read_bytes()?,
    })
}

fn write_tagged_field_u32(
    writer: &mut VecDataWriter<LittleEndian>,
    field: &TaggedField<u32>,
) -> Result<(), Error> {
    writer.write_u32(field.tag);
    writer.write_bytes(&field.payload);
    Ok(())
}

fn read_len_prefixed_vec<R, T, F>(reader: &mut R, mut read_item: F) -> Result<Vec<T>, Error>
where
    R: DataReader<LittleEndian>,
    F: FnMut(&mut R) -> Result<T, Error>,
{
    let count = read_usize(read_le_vuint(reader)?)?;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        items.push(read_item(reader)?);
    }
    Ok(items)
}

fn write_len_prefixed_slice<T, F>(
    writer: &mut VecDataWriter<LittleEndian>,
    items: &[T],
    mut write_item: F,
) -> Result<(), Error>
where
    F: FnMut(&mut VecDataWriter<LittleEndian>, &T) -> Result<(), Error>,
{
    writer.write_vuint(items.len() as u64);
    for item in items {
        write_item(writer, item)?;
    }
    Ok(())
}

fn write_payload<F>(writer: &mut VecDataWriter<LittleEndian>, encode: F) -> Result<(), Error>
where
    F: FnOnce(&mut VecDataWriter<LittleEndian>) -> Result<(), Error>,
{
    let mut payload = VecDataWriter::<LittleEndian>::new();
    encode(&mut payload)?;
    writer.write_bytes(&payload.into_inner());
    Ok(())
}

fn validate_resource_path(path: &str) -> Result<(), Error> {
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

fn ensure_fully_consumed(
    reader: &impl DataReader<LittleEndian>,
    context: &'static str,
) -> Result<(), Error> {
    if remaining_le(reader) != 0 {
        return Err(Error::InvalidSectionLayout(format!(
            "{context} has trailing bytes"
        )));
    }
    Ok(())
}

fn read_le_u16<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<u16, Error> {
    reader.read_u16()
}

fn read_le_u32<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<u32, Error> {
    reader.read_u32()
}

fn read_le_u64<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<u64, Error> {
    reader.read_u64()
}

fn read_le_i64<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<i64, Error> {
    reader.read_i64()
}

fn read_le_vuint<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<u64, Error> {
    reader.read_vuint()
}

fn read_le_u8_array<R: DataReader<LittleEndian>>(
    reader: &mut R,
    size: usize,
) -> Result<Box<[u8]>, Error> {
    reader.read_u8_array(size)
}

fn remaining_le<R: DataReader<LittleEndian> + ?Sized>(reader: &R) -> usize {
    reader.remaining()
}

fn read_le_bytes<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<Box<[u8]>, Error> {
    reader.read_bytes()
}

fn read_le_string<R: DataReader<LittleEndian>>(reader: &mut R) -> Result<String, Error> {
    reader.read_string()
}

fn read_usize(value: u64) -> Result<usize, Error> {
    usize::try_from(value)
        .map_err(|_| Error::InvalidSectionLayout("value does not fit in usize".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vuint_roundtrip() -> Result<(), Error> {
        let values = [
            0,
            1,
            0x7f,
            0x80,
            0x3fff,
            0x4000,
            u32::MAX as u64,
            u64::MAX >> 1,
        ];
        let mut writer = VecDataWriter::<LittleEndian>::new();
        for value in values {
            writer.write_vuint(value);
        }

        let bytes = writer.into_inner();
        let mut reader = ArrayDataReader::new(&bytes);
        for value in values {
            assert_eq!(read_le_vuint(&mut reader)?, value);
        }
        assert_eq!(remaining_le(&reader), 0);
        Ok(())
    }

    #[test]
    fn janex_roundtrip() -> Result<(), Error> {
        let mut string_pool = StringPool::with_empty_root();
        let dir_index = string_pool.push("com/example");
        let file_index = string_pool.push("App.class");

        let file = JanexFile {
            major_version: CURRENT_MAJOR_VERSION,
            minor_version: CURRENT_MINOR_VERSION,
            flags: 0,
            fields: vec![TaggedField::<u32>::new(0xfeed_beef, b"meta".to_vec())],
            verification: VerificationInfo::Checksum(Checksum {
                algorithm: ChecksumAlgorithm::Sha256,
                checksum: Box::new([]),
            }),
            sections: vec![
                Section {
                    id: 0,
                    options: Vec::new(),
                    checksum: Checksum {
                        algorithm: ChecksumAlgorithm::Sha256,
                        checksum: Box::new([]),
                    },
                    content: SectionContent::StringPool(StringPoolSection {
                        compression: CompressInfo::none(),
                        strings: string_pool,
                    }),
                },
                Section {
                    id: 0,
                    options: Vec::new(),
                    checksum: Checksum {
                        algorithm: ChecksumAlgorithm::Sha256,
                        checksum: Box::new([]),
                    },
                    content: SectionContent::RootConfigGroup(RootConfigGroupSection {
                        root_group: ConfigGroup {
                            fields: vec![
                                ConfigField::MainClass("com.example.Main".to_string()),
                                ConfigField::ClassPath(vec![ResourceGroupReference::Local {
                                    group_name: "app".to_string(),
                                }]),
                                ConfigField::JvmOptions(vec!["-Xmx512m".to_string()]),
                            ],
                        },
                    }),
                },
                Section {
                    id: 0,
                    options: Vec::new(),
                    checksum: Checksum {
                        algorithm: ChecksumAlgorithm::Sha256,
                        checksum: Box::new([]),
                    },
                    content: SectionContent::ResourceGroups(ResourceGroupsSection {
                        groups: vec![ResourceGroup {
                            name: "app".to_string(),
                            fields: Vec::new(),
                            resources_compression: CompressInfo::none(),
                            resources: vec![
                                Resource::Directory {
                                    path: ResourcePath::String("com".to_string()),
                                    fields: Vec::new(),
                                },
                                Resource::File {
                                    path: ResourcePath::Ref {
                                        directory_index: dir_index,
                                        file_name_index: file_index,
                                    },
                                    compress_info: CompressInfo {
                                        method: CompressMethod::Classfile,
                                        uncompressed_size: 5,
                                        compressed_size: 5,
                                        options: Box::new([]),
                                    },
                                    content_offset: 0,
                                    fields: vec![ResourceField::Comment("class".to_string())],
                                },
                            ],
                        }],
                    }),
                },
                Section {
                    id: 0,
                    options: Vec::new(),
                    checksum: Checksum {
                        algorithm: ChecksumAlgorithm::Sha256,
                        checksum: Box::new([]),
                    },
                    content: SectionContent::DataPool(DataPoolSection {
                        bytes: b"hello".to_vec().into_boxed_slice(),
                    }),
                },
            ],
        };

        let encoded = file.write()?;
        let decoded = JanexFile::read(&encoded)?;
        assert_eq!(decoded.write()?, encoded);
        assert_eq!(decoded.sections.len(), 4);
        Ok(())
    }

    #[test]
    fn reject_invalid_path() {
        assert!(validate_resource_path("foo//bar").is_err());
        assert!(validate_resource_path("../bar").is_err());
        assert!(validate_resource_path("/bar").is_err());
    }
}

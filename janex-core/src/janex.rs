// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::checksum::{
    AnyChecksum, compute_checksum, encode_verification_info, read_checksum, read_verification_info,
    verify_checksum, write_checksum,
};
pub use crate::checksum::{
    CmsSignature, DetachedSignatureVerifier, OpenPgpSignature, RejectingDetachedSignatureVerifier,
    VerificationInfo,
};
use crate::error::Error;
use crate::io::{ArrayDataReader, DataReader, DataWriter, VecDataWriter};
use crate::section::{encode_section_content, parse_section_content};
use crate::string_pool::StringPool;
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};

const CURRENT_MAJOR_VERSION: u32 = 0;
const CURRENT_MINOR_VERSION: u32 = 0;

/// An in-memory representation of a Janex file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JanexFile {
    /// The Janex file-format major version.
    pub major_version: u32,
    /// The Janex file-format minor version.
    pub minor_version: u32,
    /// Reserved file-level flags.
    pub flags: u64,
    /// Opaque metadata fields reserved for forward-compatible extensions.
    pub fields: Vec<TaggedField<u32>>,
    /// Integrity information for the metadata section itself.
    pub verification: VerificationInfo,
    sections: Vec<Section>,
}

/// Builds a `JanexFile` from typed sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JanexBuilder {
    /// The Janex file-format major version.
    pub major_version: u32,
    /// The Janex file-format minor version.
    pub minor_version: u32,
    /// Reserved file-level flags.
    pub flags: u64,
    /// Opaque metadata fields reserved for forward-compatible extensions.
    pub fields: Vec<TaggedField<u32>>,
    /// Integrity information for the metadata section itself.
    pub verification: VerificationInfo,
    sections: Vec<Section>,
}

/// Metadata and typed payload for one section being added to a `JanexBuilder`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionBuilder<T> {
    /// The section identifier, unique within its section type.
    pub id: u64,
    /// Section-scoped extension fields.
    pub options: Vec<TaggedField<u32>>,
    /// The checksum policy for the encoded section payload.
    pub checksum: AnyChecksum,
    /// The typed content carried by this section.
    pub content: T,
}

/// The supported Janex section bodies implemented by `janex-core`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SectionContent {
    /// Arbitrary padding bytes between typed sections.
    Padding(Box<[u8]>),
    /// The launcher root configuration tree.
    RootConfigGroup(RootConfigGroupSection),
    /// Embedded resource-group metadata.
    ResourceGroups(ResourceGroupsSection),
    /// Shared strings referenced by resources and classfile compression.
    StringPool(StringPoolSection),
    /// The raw bytes referenced by file resources.
    DataPool(DataPoolSection),
}

/// A section entry recorded in `FileMetadata.section_table`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    /// The section identifier, unique within its section type.
    id: u64,
    /// Section-scoped extension fields.
    options: Vec<TaggedField<u32>>,
    /// The checksum policy for the encoded section payload.
    checksum: AnyChecksum,
    /// The typed content carried by this section.
    content: SectionContent,
}

/// The `RootConfigGroup` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootConfigGroupSection {
    /// The root configuration group evaluated by the launcher.
    pub root_group: ConfigGroup,
}

/// The `ResourceGroups` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceGroupsSection {
    /// All embedded resource groups.
    pub groups: Vec<ResourceGroup>,
}

/// The `StringPool` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringPoolSection {
    /// Compression settings for the concatenated string bytes.
    pub compression: CompressInfo,
    /// Strings stored in pool-index order.
    pub strings: StringPool,
}

/// The `DataPool` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPoolSection {
    /// Concatenated raw bytes referenced by file resources.
    pub bytes: Box<[u8]>,
}

/// A logical resource container, typically mirroring one source JAR or module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceGroup {
    /// The group name referenced from configuration entries.
    pub name: String,
    /// Opaque extension fields reserved for future use.
    pub fields: Vec<TaggedField<u32>>,
    /// Compression settings for the encoded resource metadata array.
    pub resources_compression: CompressInfo,
    /// All resources declared by the group.
    pub resources: Vec<Resource>,
}

/// A configuration group inside the launcher configuration tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigGroup {
    /// Ordered fields applied by the launcher.
    pub fields: Vec<ConfigField>,
}

/// Typed configuration items supported by the current Janex format implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigField {
    /// CEL condition guarding the current group.
    Condition(String),
    /// Fully qualified main-class name.
    MainClass(String),
    /// Main module name used with `--module`.
    MainModule(String),
    /// Resource groups placed on the JVM module path.
    ModulePath(Vec<ResourceGroupReference>),
    /// Resource groups placed on the JVM class path.
    ClassPath(Vec<ResourceGroupReference>),
    /// Java agents loaded before application startup.
    Agents(Vec<JavaAgent>),
    /// Additional JVM arguments.
    JvmOptions(Vec<String>),
    /// Nested configuration groups.
    SubGroups(Vec<ConfigGroup>),
}

/// A reference to either an embedded resource group or a remote Maven artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceGroupReference {
    /// A reference to a local `ResourceGroup`.
    Local { group_name: String },
    /// A reference to a Maven artifact downloaded at launch time.
    Maven {
        gav: String,
        repository: String,
        checksum: AnyChecksum,
    },
}

/// A Java agent entry with its optional agent argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaAgent {
    /// The resource group or Maven artifact containing the agent JAR.
    pub reference: ResourceGroupReference,
    /// The option string appended to `-javaagent:...=`.
    pub option: String,
}

/// A resource entry inside a `ResourceGroup`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    /// A regular file whose bytes live in the `DataPool`.
    File {
        path: ResourcePath,
        compress_info: CompressInfo,
        content_offset: u64,
        fields: Vec<ResourceField>,
    },
    /// A directory marker.
    Directory {
        path: ResourcePath,
        fields: Vec<ResourceField>,
    },
    /// A symbolic-link entry.
    SymbolicLink {
        path: ResourcePath,
        target: ResourcePath,
        fields: Vec<ResourceField>,
    },
}

/// A resource path encoded either inline or by string-pool references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourcePath {
    /// Stores the full UTF-8 path inline.
    String(String),
    /// Stores directory and file-name components via string-pool indices.
    Ref {
        directory_index: u64,
        file_name_index: u64,
    },
}

/// Optional metadata attached to a `Resource`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceField {
    Checksum(AnyChecksum),
    Comment(String),
    FileCreateTime(Timestamp),
    FileModifyTime(Timestamp),
    FileAccessTime(Timestamp),
    PosixFilePermissions(u16),
    Custom { name: String, content: Box<[u8]> },
}

/// A nanosecond-precision timestamp relative to the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub epoch_second: i64,
    pub nanos: u32,
}

/// A tagged payload preserved as raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedField<T> {
    pub tag: T,
    pub payload: Box<[u8]>,
}

/// Compression metadata for an encoded Janex payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressInfo {
    pub method: CompressMethod,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub options: Box<[u8]>,
}

/// Compression algorithms referenced by `CompressInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressMethod {
    None = 0,
    Composite = 1,
    Classfile = 2,
    Zstd = 3,
}

/// Well-known Janex section type tags.
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

/// The eagerly loaded metadata of a Janex file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JanexMetadata {
    /// The Janex file-format major version.
    pub major_version: u32,
    /// The Janex file-format minor version.
    pub minor_version: u32,
    /// Reserved file-level flags.
    pub flags: u64,
    /// Opaque metadata fields reserved for forward-compatible extensions.
    pub fields: Vec<TaggedField<u32>>,
    /// Integrity information for the metadata section itself.
    pub verification: VerificationInfo,
    /// Indexed metadata for each non-metadata section stored in the file body.
    pub sections: Vec<SectionMetadata>,
}

/// The file offset and metadata of a single section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionMetadata {
    /// The section type tag.
    pub section_type: SectionType,
    /// The section identifier, unique within its section type.
    pub id: u64,
    /// Section-scoped extension fields.
    pub options: Vec<TaggedField<u32>>,
    /// The encoded section length in bytes.
    pub length: u64,
    /// The checksum policy for the encoded section payload.
    pub checksum: AnyChecksum,
    /// Absolute file offset of the section payload, including its section magic when present.
    pub offset: u64,
}

/// A file-backed Janex reader that keeps only `FileMetadata` in memory.
#[derive(Debug)]
pub struct JanexArchive<R> {
    reader: R,
    metadata: JanexMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SectionInfoRecord {
    section_type: SectionType,
    id: u64,
    options: Vec<TaggedField<u32>>,
    length: u64,
    checksum: AnyChecksum,
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

    fn new(sections: Vec<Section>) -> Self {
        Self {
            major_version: CURRENT_MAJOR_VERSION,
            minor_version: CURRENT_MINOR_VERSION,
            flags: 0,
            fields: Vec::new(),
            verification: VerificationInfo::None,
            sections,
        }
    }

    /// Starts building a Janex file using typed section inputs.
    pub fn builder() -> JanexBuilder {
        JanexBuilder::new()
    }

    /// Eagerly parses a Janex file from raw bytes.
    ///
    /// This convenience API now delegates to `JanexArchive` and decodes every
    /// section after the metadata has been validated.
    pub fn read_all(bytes: &[u8]) -> Result<Self, Error> {
        JanexArchive::open(std::io::Cursor::new(bytes))?.decode_all()
    }

    /// Eagerly parses a Janex file using a detached-signature verifier when needed.
    pub fn read_all_with_verifier<V: DetachedSignatureVerifier + ?Sized>(
        bytes: &[u8],
        verifier: &V,
    ) -> Result<Self, Error> {
        JanexArchive::open_with_verifier(std::io::Cursor::new(bytes), verifier)?.decode_all()
    }

    /// Returns the number of sections stored in the file body.
    pub fn sections_len(&self) -> usize {
        self.sections.len()
    }

    /// Encodes the current file into the Janex binary format.
    ///
    /// Section payloads are serialized first so the metadata section can record
    /// their final lengths and derived checksums.
    pub fn write(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;

        let mut section_infos = Vec::with_capacity(self.sections.len());
        let mut encoded_sections = Vec::with_capacity(self.sections.len());
        for section in &self.sections {
            // Section metadata depends on the final encoded byte length and checksum.
            let bytes = encode_section_content(&section.content)?;
            let checksum = compute_checksum(&section.checksum, &bytes);
            section_infos.push(SectionInfoRecord {
                section_type: section.content.section_type(),
                id: section.id,
                options: section.options.clone(),
                length: bytes.len() as u64,
                checksum,
            });
            encoded_sections.push(bytes);
        }

        let mut metadata_prefix = VecDataWriter::new();
        metadata_prefix.write_u64_le(Self::FILE_METADATA_MAGIC_NUMBER);
        metadata_prefix.write_u32_le(self.major_version);
        metadata_prefix.write_u32_le(self.minor_version);
        metadata_prefix.write_u64_le(self.flags);
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
                VerificationInfo::Checksum(compute_checksum(checksum, &metadata_prefix))
            }
            VerificationInfo::OpenPgp(signature) => VerificationInfo::OpenPgp(signature.clone()),
            VerificationInfo::Cms(signature) => VerificationInfo::Cms(signature.clone()),
        };
        let verification_bytes = encode_verification_info(&verification)?;
        let metadata_length = metadata_prefix.len() + verification_bytes.len() + 24;
        let sections_length: usize = encoded_sections.iter().map(Vec::len).sum();
        let file_length = 8 + sections_length + metadata_length;

        let mut writer = VecDataWriter::with_capacity(file_length);
        writer.write_u64_le(Self::MAGIC_NUMBER);
        for section in &encoded_sections {
            writer.write_all(section);
        }
        writer.write_all(&metadata_prefix);
        writer.write_all(&verification_bytes);
        writer.write_u64_le(Self::END_MARK);
        writer.write_u64_le(metadata_length as u64);
        writer.write_u64_le(file_length as u64);
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
            && string_pool_position >= resource_groups_position
        {
            return Err(Error::InvalidSectionLayout(
                "the string pool section must appear before the resource groups section"
                    .to_string(),
            ));
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

impl JanexBuilder {
    /// Creates an empty builder using the current Janex format version.
    pub fn new() -> Self {
        Self {
            major_version: CURRENT_MAJOR_VERSION,
            minor_version: CURRENT_MINOR_VERSION,
            flags: 0,
            fields: Vec::new(),
            verification: VerificationInfo::None,
            sections: Vec::new(),
        }
    }

    /// Adds a padding section without replacing any existing section.
    pub fn push_padding<S>(&mut self, section: S) -> &mut Self
    where
        S: Into<SectionBuilder<Box<[u8]>>>,
    {
        self.sections
            .push(section.into().into_section(SectionContent::Padding));
        self
    }

    /// Sets the root configuration section, replacing any existing one in place.
    pub fn with_root_config_group<S>(&mut self, section: S) -> &mut Self
    where
        S: Into<SectionBuilder<RootConfigGroupSection>>,
    {
        self.replace_unique_section(section.into().into_section(SectionContent::RootConfigGroup));
        self
    }

    /// Sets the resource-groups section, replacing any existing one in place.
    pub fn with_resource_groups<S>(&mut self, section: S) -> &mut Self
    where
        S: Into<SectionBuilder<ResourceGroupsSection>>,
    {
        self.replace_unique_section(section.into().into_section(SectionContent::ResourceGroups));
        self
    }

    /// Sets the string-pool section, replacing any existing one in place.
    pub fn with_string_pool<S>(&mut self, section: S) -> &mut Self
    where
        S: Into<SectionBuilder<StringPoolSection>>,
    {
        self.replace_unique_section(section.into().into_section(SectionContent::StringPool));
        self
    }

    /// Sets the data-pool section, replacing any existing one in place.
    pub fn with_data_pool<S>(&mut self, section: S) -> &mut Self
    where
        S: Into<SectionBuilder<DataPoolSection>>,
    {
        self.replace_unique_section(section.into().into_section(SectionContent::DataPool));
        self
    }

    /// Validates the collected sections and returns a writable `JanexFile`.
    pub fn build(self) -> Result<JanexFile, Error> {
        let file = JanexFile {
            major_version: self.major_version,
            minor_version: self.minor_version,
            flags: self.flags,
            fields: self.fields,
            verification: self.verification,
            sections: self.sections,
        };
        file.validate()?;
        Ok(file)
    }

    fn replace_unique_section(&mut self, section: Section) {
        let section_type = section.content.section_type();
        if let Some(existing) = self
            .sections
            .iter_mut()
            .find(|existing| existing.content.section_type() == section_type)
        {
            *existing = section;
        } else {
            self.sections.push(section);
        }
    }
}

impl Default for JanexBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SectionBuilder<T> {
    /// Creates section metadata using default id, options, and no checksum.
    pub fn new(content: T) -> Self {
        Self {
            id: 0,
            options: Vec::new(),
            checksum: AnyChecksum::default(),
            content,
        }
    }

    /// Sets the section identifier.
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    /// Replaces the section options.
    pub fn with_options(mut self, options: Vec<TaggedField<u32>>) -> Self {
        self.options = options;
        self
    }

    /// Sets the checksum algorithm template used when encoding the section.
    pub fn with_checksum(mut self, checksum: AnyChecksum) -> Self {
        self.checksum = checksum;
        self
    }

    fn into_section(self, content: impl FnOnce(T) -> SectionContent) -> Section {
        Section {
            id: self.id,
            options: self.options,
            checksum: self.checksum,
            content: content(self.content),
        }
    }
}

impl<T> From<T> for SectionBuilder<T> {
    fn from(content: T) -> Self {
        Self::new(content)
    }
}

impl From<Vec<u8>> for SectionBuilder<Box<[u8]>> {
    fn from(content: Vec<u8>) -> Self {
        Self::new(content.into_boxed_slice())
    }
}

impl JanexMetadata {
    /// Validates metadata-level invariants that do not require decoding section bodies.
    pub fn validate(&self) -> Result<(), Error> {
        let mut root_config_group_count = 0usize;
        let mut resource_groups_count = 0usize;
        let mut string_pool_count = 0usize;
        let mut data_pool_count = 0usize;
        let mut seen_section_keys = HashSet::with_capacity(self.sections.len());
        let mut string_pool_position = None;
        let mut resource_groups_position = None;

        for (idx, section) in self.sections.iter().enumerate() {
            let key = (section.section_type as u64, section.id);
            if !seen_section_keys.insert(key) {
                return Err(Error::InvalidSectionLayout(format!(
                    "duplicate section id {} for section type 0x{:016x}",
                    section.id, section.section_type as u64
                )));
            }

            match section.section_type {
                SectionType::Padding => {}
                SectionType::RootConfigGroup => {
                    root_config_group_count += 1;
                }
                SectionType::ResourceGroups => {
                    resource_groups_count += 1;
                    resource_groups_position = Some(idx);
                }
                SectionType::StringPool => {
                    string_pool_count += 1;
                    string_pool_position = Some(idx);
                }
                SectionType::DataPool => {
                    data_pool_count += 1;
                }
                SectionType::ExternalHeader
                | SectionType::ExternalTail
                | SectionType::FileMetadata
                | SectionType::Attributes => {
                    return Err(Error::UnsupportedFeature(
                        "external header/tail, attributes, and nested metadata sections are not implemented",
                    ));
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
            && string_pool_position >= resource_groups_position
        {
            return Err(Error::InvalidSectionLayout(
                "the string pool section must appear before the resource groups section"
                    .to_string(),
            ));
        }

        Ok(())
    }
}

impl SectionMetadata {
    /// Returns the exclusive end offset of the encoded section payload.
    pub fn end_offset(&self) -> Result<u64, Error> {
        self.offset
            .checked_add(self.length)
            .ok_or_else(|| Error::InvalidSectionLayout("section offset overflow".to_string()))
    }
}

impl<R: Read + Seek> JanexArchive<R> {
    /// Opens a Janex file from a seekable reader and loads only `FileMetadata`.
    pub fn open(reader: R) -> Result<Self, Error> {
        Self::open_with_verifier(reader, &RejectingDetachedSignatureVerifier)
    }

    /// Opens a Janex file using a detached-signature verifier when needed.
    pub fn open_with_verifier<V: DetachedSignatureVerifier + ?Sized>(
        mut reader: R,
        verifier: &V,
    ) -> Result<Self, Error> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        if file_size < 24 {
            return Err(Error::UnexpectedEndOfFile);
        }

        let footer_bytes = read_exact_at(&mut reader, file_size - 24, 24)?;
        let mut footer_reader = ArrayDataReader::new(&footer_bytes);
        let end_mark = DataReader::read_u64_le(&mut footer_reader)?;
        if end_mark != JanexFile::END_MARK {
            return Err(Error::InvalidMagicNumber {
                expected: JanexFile::END_MARK,
                actual: end_mark,
            });
        }

        let metadata_length = read_usize(DataReader::read_u64_le(&mut footer_reader)?)?;
        let file_length = DataReader::read_u64_le(&mut footer_reader)?;
        if file_length > file_size {
            return Err(Error::InvalidSectionLayout(
                "file_length is larger than the input size".to_string(),
            ));
        }

        let file_start = file_size - file_length;
        let file_end = file_start + file_length;
        let metadata_start = file_end
            .checked_sub(metadata_length as u64)
            .ok_or_else(|| Error::InvalidSectionLayout("metadata_length underflow".to_string()))?;
        if metadata_start < file_start + 8 {
            return Err(Error::InvalidSectionLayout(
                "metadata section overlaps the file header".to_string(),
            ));
        }

        let file_header = read_exact_at(&mut reader, file_start, 8)?;
        let mut file_reader = ArrayDataReader::new(&file_header);
        let magic = DataReader::read_u64_le(&mut file_reader)?;
        if magic != JanexFile::MAGIC_NUMBER {
            return Err(Error::InvalidMagicNumber {
                expected: JanexFile::MAGIC_NUMBER,
                actual: magic,
            });
        }

        let metadata_bytes = read_exact_at(&mut reader, metadata_start, metadata_length)?;
        let parsed_metadata = read_metadata(&metadata_bytes)?;
        if parsed_metadata.metadata_length != metadata_length as u64 {
            return Err(Error::InvalidSectionLayout(
                "metadata_length does not match the footer".to_string(),
            ));
        }
        if parsed_metadata.file_length != file_length {
            return Err(Error::InvalidSectionLayout(
                "file_length does not match the footer".to_string(),
            ));
        }
        if parsed_metadata.major_version != CURRENT_MAJOR_VERSION
            || parsed_metadata.minor_version != CURRENT_MINOR_VERSION
        {
            return Err(Error::UnsupportedFeature("unsupported Janex file version"));
        }
        parsed_metadata.verify(&metadata_bytes, verifier)?;

        let mut next_section_offset = file_start + 8;
        let mut sections = Vec::with_capacity(parsed_metadata.section_table.len());
        for record in &parsed_metadata.section_table {
            sections.push(SectionMetadata {
                section_type: record.section_type,
                id: record.id,
                options: record.options.clone(),
                length: record.length,
                checksum: record.checksum,
                offset: next_section_offset,
            });
            next_section_offset =
                next_section_offset
                    .checked_add(record.length)
                    .ok_or_else(|| {
                        Error::InvalidSectionLayout("section offset overflow".to_string())
                    })?;
        }

        if next_section_offset != metadata_start {
            return Err(Error::InvalidSectionLayout(
                "section table does not consume the full file body".to_string(),
            ));
        }

        let metadata = JanexMetadata {
            major_version: parsed_metadata.major_version,
            minor_version: parsed_metadata.minor_version,
            flags: parsed_metadata.flags,
            fields: parsed_metadata.fields,
            verification: parsed_metadata.verification,
            sections,
        };
        metadata.validate()?;

        Ok(Self { reader, metadata })
    }

    /// Returns the eagerly loaded `FileMetadata` view.
    pub fn metadata(&self) -> &JanexMetadata {
        &self.metadata
    }

    /// Returns metadata for all indexed sections.
    pub fn sections(&self) -> &[SectionMetadata] {
        &self.metadata.sections
    }

    /// Returns metadata for a section by index.
    pub fn section(&self, index: usize) -> Option<&SectionMetadata> {
        self.metadata.sections.get(index)
    }

    /// Consumes the archive and returns the underlying reader.
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Reads and verifies the raw bytes of a section on demand.
    pub fn read_section_bytes(&mut self, index: usize) -> Result<Box<[u8]>, Error> {
        let section = self
            .metadata
            .sections
            .get(index)
            .ok_or_else(|| Error::InvalidReference(format!("invalid section index {index}")))?
            .clone();
        let bytes = read_exact_at(
            &mut self.reader,
            section.offset,
            read_usize(section.length)?,
        )?;
        verify_checksum(&section.checksum, &bytes, "section")?;
        Ok(bytes)
    }

    /// Decodes every indexed section into an owned `JanexFile`.
    pub fn decode_all(&mut self) -> Result<JanexFile, Error> {
        let mut sections = Vec::with_capacity(self.metadata.sections.len());
        for index in 0..self.metadata.sections.len() {
            let metadata = self.metadata.sections[index].clone();
            sections.push(Section {
                id: metadata.id,
                options: metadata.options,
                checksum: metadata.checksum,
                content: self.decode_section(index)?,
            });
        }

        let file = JanexFile {
            major_version: self.metadata.major_version,
            minor_version: self.metadata.minor_version,
            flags: self.metadata.flags,
            fields: self.metadata.fields.clone(),
            verification: self.metadata.verification.clone(),
            sections,
        };
        file.validate()?;
        Ok(file)
    }

    /// Reads the first `RootConfigGroup` section, if present.
    pub fn read_root_config_group(&mut self) -> Result<Option<RootConfigGroupSection>, Error> {
        match self.read_first_section_of_type(SectionType::RootConfigGroup)? {
            Some(SectionContent::RootConfigGroup(section)) => Ok(Some(section)),
            None => Ok(None),
            Some(_) => unreachable!(),
        }
    }

    /// Reads the first `ResourceGroups` section, if present.
    pub fn read_resource_groups(&mut self) -> Result<Option<ResourceGroupsSection>, Error> {
        match self.read_first_section_of_type(SectionType::ResourceGroups)? {
            Some(SectionContent::ResourceGroups(section)) => Ok(Some(section)),
            None => Ok(None),
            Some(_) => unreachable!(),
        }
    }

    /// Reads the first `StringPool` section, if present.
    pub fn read_string_pool(&mut self) -> Result<Option<StringPoolSection>, Error> {
        match self.read_first_section_of_type(SectionType::StringPool)? {
            Some(SectionContent::StringPool(section)) => Ok(Some(section)),
            None => Ok(None),
            Some(_) => unreachable!(),
        }
    }

    /// Reads the first `DataPool` section, if present.
    pub fn read_data_pool(&mut self) -> Result<Option<DataPoolSection>, Error> {
        match self.read_first_section_of_type(SectionType::DataPool)? {
            Some(SectionContent::DataPool(section)) => Ok(Some(section)),
            None => Ok(None),
            Some(_) => unreachable!(),
        }
    }

    fn read_first_section_of_type(
        &mut self,
        section_type: SectionType,
    ) -> Result<Option<SectionContent>, Error> {
        if let Some(index) = self
            .metadata
            .sections
            .iter()
            .position(|section| section.section_type == section_type)
        {
            return self.decode_section(index).map(Some);
        }
        Ok(None)
    }

    fn decode_section(&mut self, index: usize) -> Result<SectionContent, Error> {
        let section_type = self
            .metadata
            .sections
            .get(index)
            .ok_or_else(|| Error::InvalidReference(format!("invalid section index {index}")))?
            .section_type;
        let bytes = self.read_section_bytes(index)?;
        parse_section_content(section_type, &bytes)
    }
}

impl TaggedField<u32> {
    /// Creates a 32-bit tagged payload from raw bytes.
    pub fn new(tag: u32, payload: Vec<u8>) -> Self {
        Self {
            tag,
            payload: payload.into_boxed_slice(),
        }
    }
}

impl TaggedField<u8> {
    /// Creates an 8-bit tagged payload from raw bytes.
    pub fn new(tag: u8, payload: Vec<u8>) -> Self {
        Self {
            tag,
            payload: payload.into_boxed_slice(),
        }
    }
}

impl CompressInfo {
    /// Returns an empty no-compression descriptor.
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

impl ParsedMetadata {
    fn verify<V: DetachedSignatureVerifier + ?Sized>(
        &self,
        metadata_bytes: &[u8],
        verifier: &V,
    ) -> Result<(), Error> {
        self.verification
            .verify(&metadata_bytes[..self.verification_offset], verifier)
    }
}

fn read_metadata(bytes: &[u8]) -> Result<ParsedMetadata, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let magic = DataReader::read_u64_le(&mut reader)?;
    if magic != JanexFile::FILE_METADATA_MAGIC_NUMBER {
        return Err(Error::InvalidMagicNumber {
            expected: JanexFile::FILE_METADATA_MAGIC_NUMBER,
            actual: magic,
        });
    }

    let major_version = DataReader::read_u32_le(&mut reader)?;
    let minor_version = DataReader::read_u32_le(&mut reader)?;
    let flags = DataReader::read_u64_le(&mut reader)?;
    let section_table = read_len_prefixed_vec(&mut reader, read_section_info_record)?;
    let fields = read_len_prefixed_vec(&mut reader, read_tagged_field_u32)?;
    let verification_offset = bytes.len() - DataReader::remaining(&reader);
    let verification = read_verification_info(&mut reader)?;
    let end_mark = DataReader::read_u64_le(&mut reader)?;
    if end_mark != JanexFile::END_MARK {
        return Err(Error::InvalidMagicNumber {
            expected: JanexFile::END_MARK,
            actual: end_mark,
        });
    }

    let metadata_length = DataReader::read_u64_le(&mut reader)?;
    let file_length = DataReader::read_u64_le(&mut reader)?;
    if DataReader::remaining(&reader) != 0 {
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

fn read_section_info_record<R: DataReader>(reader: &mut R) -> Result<SectionInfoRecord, Error> {
    Ok(SectionInfoRecord {
        section_type: SectionType::try_from(reader.read_u64_le()?)?,
        id: reader.read_vuint()?,
        options: read_len_prefixed_vec(reader, read_tagged_field_u32)?,
        length: reader.read_vuint()?,
        checksum: read_checksum(reader)?,
    })
}

fn write_section_info_record(
    writer: &mut VecDataWriter,
    record: &SectionInfoRecord,
) -> Result<(), Error> {
    writer.write_u64_le(record.section_type as u64);
    writer.write_vuint(record.id);
    write_len_prefixed_slice(writer, &record.options, write_tagged_field_u32)?;
    writer.write_vuint(record.length);
    write_checksum(writer, &record.checksum)?;
    Ok(())
}

pub(crate) fn read_compress_info<R: DataReader>(reader: &mut R) -> Result<CompressInfo, Error> {
    Ok(CompressInfo {
        method: CompressMethod::try_from(reader.read_u8()?)?,
        uncompressed_size: reader.read_vuint()?,
        compressed_size: reader.read_vuint()?,
        options: reader.read_bytes()?,
    })
}

pub(crate) fn write_compress_info(
    writer: &mut VecDataWriter,
    info: &CompressInfo,
) -> Result<(), Error> {
    writer.write_u8(info.method as u8);
    writer.write_vuint(info.uncompressed_size);
    writer.write_vuint(info.compressed_size);
    writer.write_bytes(&info.options);
    Ok(())
}

pub(crate) fn read_compressed_blob<R: DataReader>(
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

pub(crate) fn write_compressed_blob(
    writer: &mut VecDataWriter,
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

fn compress_bytes(info: &CompressInfo, data: &[u8]) -> Result<Vec<u8>, Error> {
    match info.method {
        CompressMethod::None => Ok(data.to_vec()),
        CompressMethod::Zstd => zstd::stream::encode_all(data, 0).map_err(Error::from),
        CompressMethod::Composite => {
            let layers = parse_composite_layers(&info.options)?;
            let mut current = data.to_vec();
            // Composite compression applies the declared layers in order.
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
            // Decompression runs the composite stack in reverse order.
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

pub(crate) fn read_tagged_field_u32<R: DataReader>(
    reader: &mut R,
) -> Result<TaggedField<u32>, Error> {
    Ok(TaggedField {
        tag: reader.read_u32_le()?,
        payload: reader.read_bytes()?,
    })
}

pub(crate) fn write_tagged_field_u32(
    writer: &mut VecDataWriter,
    field: &TaggedField<u32>,
) -> Result<(), Error> {
    writer.write_u32_le(field.tag);
    writer.write_bytes(&field.payload);
    Ok(())
}

pub(crate) fn read_len_prefixed_vec<R, T, F>(
    reader: &mut R,
    mut read_item: F,
) -> Result<Vec<T>, Error>
where
    R: DataReader,
    F: FnMut(&mut R) -> Result<T, Error>,
{
    let count = read_usize(reader.read_vuint()?)?;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        items.push(read_item(reader)?);
    }
    Ok(items)
}

pub(crate) fn write_len_prefixed_slice<T, F>(
    writer: &mut VecDataWriter,
    items: &[T],
    mut write_item: F,
) -> Result<(), Error>
where
    F: FnMut(&mut VecDataWriter, &T) -> Result<(), Error>,
{
    writer.write_vuint(items.len() as u64);
    for item in items {
        write_item(writer, item)?;
    }
    Ok(())
}

pub(crate) fn write_payload<F>(writer: &mut VecDataWriter, encode: F) -> Result<(), Error>
where
    F: FnOnce(&mut VecDataWriter) -> Result<(), Error>,
{
    let mut payload = VecDataWriter::new();
    encode(&mut payload)?;
    writer.write_bytes(&payload.into_inner());
    Ok(())
}

fn read_exact_at<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    size: usize,
) -> Result<Box<[u8]>, Error> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0u8; size];
    reader.read_exact(&mut bytes)?;
    Ok(bytes.into_boxed_slice())
}

pub(crate) fn ensure_fully_consumed(
    reader: &impl DataReader,
    context: &'static str,
) -> Result<(), Error> {
    if reader.remaining() != 0 {
        return Err(Error::InvalidSectionLayout(format!(
            "{context} has trailing bytes"
        )));
    }
    Ok(())
}

pub(crate) fn read_usize(value: u64) -> Result<usize, Error> {
    usize::try_from(value)
        .map_err(|_| Error::InvalidSectionLayout("value does not fit in usize".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::{
        Checksum, CmsSignature, DetachedSignatureVerifier, OpenPgpSignature, Sha256Checksum,
    };
    use std::io::Cursor;

    fn sample_file(verification: VerificationInfo) -> Result<JanexFile, Error> {
        let mut string_pool = StringPool::with_empty_root();
        let dir_index = string_pool.push("com/example");
        let file_index = string_pool.push("App.class");

        let section_checksum = Sha256Checksum::new([0; 32]).to_any();
        let mut builder = JanexFile::builder();
        builder.fields = vec![TaggedField::<u32>::new(0xfeed_beef, b"meta".to_vec())];
        builder.verification = verification;
        builder
            .with_string_pool(
                SectionBuilder::new(StringPoolSection {
                    compression: CompressInfo::none(),
                    strings: string_pool,
                })
                .with_checksum(section_checksum),
            )
            .with_root_config_group(
                SectionBuilder::new(RootConfigGroupSection {
                    root_group: ConfigGroup {
                        fields: vec![
                            ConfigField::MainClass("com.example.Main".to_string()),
                            ConfigField::ClassPath(vec![ResourceGroupReference::Local {
                                group_name: "app".to_string(),
                            }]),
                            ConfigField::JvmOptions(vec!["-Xmx512m".to_string()]),
                        ],
                    },
                })
                .with_checksum(section_checksum),
            )
            .with_resource_groups(
                SectionBuilder::new(ResourceGroupsSection {
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
                })
                .with_checksum(section_checksum),
            )
            .with_data_pool(
                SectionBuilder::new(DataPoolSection {
                    bytes: b"hello".to_vec().into_boxed_slice(),
                })
                .with_checksum(section_checksum),
            );

        builder.build()
    }

    struct AcceptingDetachedVerifier;

    impl DetachedSignatureVerifier for AcceptingDetachedVerifier {
        fn verify_openpgp(
            &self,
            signed_bytes: &[u8],
            signature: &OpenPgpSignature,
        ) -> Result<(), Error> {
            assert!(!signed_bytes.is_empty());
            assert_eq!(signature.as_bytes(), b"pgp-signature");
            Ok(())
        }

        fn verify_cms(&self, signed_bytes: &[u8], signature: &CmsSignature) -> Result<(), Error> {
            assert!(!signed_bytes.is_empty());
            assert_eq!(signature.as_bytes(), b"cms-signature");
            Ok(())
        }
    }

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
        let mut writer = VecDataWriter::new();
        for value in values {
            writer.write_vuint(value);
        }

        let bytes = writer.into_inner();
        let mut reader = ArrayDataReader::new(&bytes);
        for value in values {
            assert_eq!(DataReader::read_vuint(&mut reader)?, value);
        }
        assert_eq!(DataReader::remaining(&reader), 0);
        Ok(())
    }

    #[test]
    fn janex_roundtrip() -> Result<(), Error> {
        let file = sample_file(VerificationInfo::Checksum(
            Sha256Checksum::new([0; 32]).to_any(),
        ))?;

        let encoded = file.write()?;

        let mut archive = JanexArchive::open(Cursor::new(encoded.clone()))?;
        assert_eq!(archive.sections().len(), 4);
        assert_eq!(archive.metadata().major_version, CURRENT_MAJOR_VERSION);

        let root_group = archive.read_root_config_group()?.unwrap();
        assert_eq!(root_group.root_group.fields.len(), 3);

        let string_pool = archive.read_string_pool()?.unwrap();
        assert_eq!(string_pool.strings.get(2), Some("App.class"));

        let resource_groups = archive.read_resource_groups()?.unwrap();
        assert_eq!(resource_groups.groups.len(), 1);

        let data_pool = archive.read_data_pool()?.unwrap();
        assert_eq!(data_pool.bytes.as_ref(), b"hello");

        let eager = archive.decode_all()?;
        assert_eq!(eager.write()?, encoded);
        Ok(())
    }

    #[test]
    fn openpgp_verification_uses_external_verifier() -> Result<(), Error> {
        let file = sample_file(VerificationInfo::OpenPgp(b"pgp-signature".to_vec().into()))?;
        let encoded = file.write()?;

        let error = JanexFile::read_all(&encoded).unwrap_err();
        match error {
            Error::VerificationFailed(message) => {
                assert!(message.contains("OpenPGP verification requires"));
            }
            other => panic!("unexpected error: {other}"),
        }

        let decoded = JanexFile::read_all_with_verifier(&encoded, &AcceptingDetachedVerifier)?;
        assert_eq!(decoded.write()?, encoded);
        Ok(())
    }

    #[test]
    fn cms_verification_uses_external_verifier() -> Result<(), Error> {
        let file = sample_file(VerificationInfo::Cms(b"cms-signature".to_vec().into()))?;
        let encoded = file.write()?;

        let error = JanexArchive::open(Cursor::new(encoded.clone())).unwrap_err();
        match error {
            Error::VerificationFailed(message) => {
                assert!(message.contains("CMS verification requires"));
            }
            other => panic!("unexpected error: {other}"),
        }

        let mut archive = JanexArchive::open_with_verifier(
            Cursor::new(encoded.clone()),
            &AcceptingDetachedVerifier,
        )?;
        assert_eq!(archive.read_data_pool()?.unwrap().bytes.as_ref(), b"hello");
        assert_eq!(archive.decode_all()?.write()?, encoded);
        Ok(())
    }

    #[test]
    fn reject_invalid_path() {
        assert!(crate::section::validate_resource_path("foo//bar").is_err());
        assert!(crate::section::validate_resource_path("../bar").is_err());
        assert!(crate::section::validate_resource_path("/bar").is_err());
    }
}

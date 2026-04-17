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
    /// All non-metadata sections stored in file order.
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
    /// Collected section payloads that will become the file body.
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
    /// Optional metadata attributes.
    Attributes(AttributesSection),
    /// Raw bytes stored before the `JanexFile` structure.
    ExternalHeader(Box<[u8]>),
    /// Raw bytes stored after the `JanexFile` structure.
    ExternalTail(Box<[u8]>),
    /// The launcher root configuration tree.
    RootConfigGroup(RootConfigGroupSection),
    /// Embedded resource-group metadata.
    ResourceGroups(ResourceGroupsSection),
    /// Shared strings referenced by resources and classfile compression.
    StringPool(StringPoolSection),
    /// The raw bytes referenced by file resources.
    DataPool(DataPoolSection),
    /// An unrecognized section preserved as raw bytes.
    Unknown(UnknownSection),
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

/// The `Attributes` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributesSection {
    /// The attributes carried by the file.
    pub attributes: Vec<Attribute>,
}

/// One name/value attribute inside `AttributesSection`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// The attribute name.
    pub name: String,
    /// The raw attribute value bytes.
    pub value: Box<[u8]>,
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
    /// An unrecognized configuration field preserved as raw bytes.
    Unknown {
        /// The unrecognized 4-byte field type tag.
        field_type: u32,
        /// The raw length-prefixed field payload.
        payload: Box<[u8]>,
    },
}

/// A reference to either an embedded resource group or a remote Maven artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceGroupReference {
    /// A reference to a local `ResourceGroup`.
    Local {
        /// The unique `ResourceGroup.name` referenced by this entry.
        group_name: String,
    },
    /// A reference to a Maven artifact downloaded at launch time.
    Maven {
        /// The Maven coordinates in `groupId:artifactId:version` form.
        gav: String,
        /// The Maven repository base URL used to download the artifact.
        repository: String,
        /// The expected checksum of the downloaded artifact.
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
        /// The path of the file within the resource group.
        path: ResourcePath,
        /// The compression metadata for the file content stored in the `DataPool`.
        compress_info: CompressInfo,
        /// The byte offset of the compressed file bytes inside the `DataPool`.
        content_offset: u64,
        /// Optional metadata fields attached to the file entry.
        fields: Vec<ResourceField>,
    },
    /// A directory marker.
    Directory {
        /// The path of the directory within the resource group.
        path: ResourcePath,
        /// Optional metadata fields attached to the directory entry.
        fields: Vec<ResourceField>,
    },
    /// A symbolic-link entry.
    SymbolicLink {
        /// The path of the symbolic link within the resource group.
        path: ResourcePath,
        /// The target path referenced by the symbolic link.
        target: ResourcePath,
        /// Optional metadata fields attached to the symbolic-link entry.
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
        /// The index of the directory component in the shared `StringPool`.
        directory_index: u64,
        /// The index of the file-name component in the shared `StringPool`.
        file_name_index: u64,
    },
}

/// Optional metadata attached to a `Resource`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceField {
    /// Checksum of the uncompressed resource content.
    Checksum(AnyChecksum),
    /// UTF-8 comment string attached to the resource.
    Comment(String),
    /// Resource creation timestamp.
    FileCreateTime(Timestamp),
    /// Resource last-modification timestamp.
    FileModifyTime(Timestamp),
    /// Resource last-access timestamp.
    FileAccessTime(Timestamp),
    /// POSIX file permission bits.
    PosixFilePermissions(u16),
    /// Application-defined custom metadata.
    Custom {
        /// The custom field name.
        name: String,
        /// The raw custom field content bytes.
        content: Box<[u8]>,
    },
    /// An unrecognized resource field preserved as raw bytes.
    Unknown {
        /// The unrecognized 1-byte field identifier.
        id: u8,
        /// The raw length-prefixed field payload.
        payload: Box<[u8]>,
    },
}

/// A nanosecond-precision timestamp relative to the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    /// The number of seconds elapsed since the Unix epoch.
    pub epoch_second: i64,
    /// The sub-second nanosecond component in the range `[0, 1_000_000_000)`.
    pub nanos: u32,
}

/// A tagged payload preserved as raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedField<T> {
    /// The integer tag identifying the payload type.
    pub tag: T,
    /// The raw payload bytes following the tag.
    pub payload: Box<[u8]>,
}

/// Compression metadata for an encoded Janex payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressInfo {
    /// The compression method applied to the payload.
    pub method: CompressMethod,
    /// The byte length of the uncompressed payload.
    pub uncompressed_size: u64,
    /// The byte length of the compressed payload.
    pub compressed_size: u64,
    /// Method-specific compression options.
    pub options: Box<[u8]>,
}

/// Compression algorithms referenced by `CompressInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressMethod {
    /// No compression.
    None = 0,
    /// Multiple compression layers applied in sequence.
    Composite = 1,
    /// Janex's class-file-aware compression transform.
    Classfile = 2,
    /// Zstandard compression.
    Zstd = 3,
}

/// Well-known Janex section type tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionType {
    /// Arbitrary padding bytes.
    Padding,
    /// Bytes stored before the `JanexFile` structure.
    ExternalHeader,
    /// Bytes stored after the `JanexFile` structure.
    ExternalTail,
    /// The `FileMetadata` section.
    FileMetadata,
    /// The `Attributes` section.
    Attributes,
    /// The `DataPool` section.
    DataPool,
    /// The `RootConfigGroup` section.
    RootConfigGroup,
    /// The `ResourceGroups` section.
    ResourceGroups,
    /// The `StringPool` section.
    StringPool,
    /// An unrecognized section type tag.
    Unknown(u64),
}

/// An unrecognized section body preserved as raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownSection {
    /// The unrecognized section type tag.
    pub section_type: SectionType,
    /// The raw encoded section bytes.
    pub bytes: Box<[u8]>,
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
    /// The underlying seekable reader from which sections are loaded on demand.
    reader: R,
    /// The eagerly loaded `FileMetadata` view.
    metadata: JanexMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SectionInfoRecord {
    /// The section type stored in `FileMetadata.section_table`.
    section_type: SectionType,
    /// The section identifier unique within the section type.
    id: u64,
    /// Section-scoped extension fields.
    options: Vec<TaggedField<u32>>,
    /// The encoded length of the section payload.
    length: u64,
    /// The checksum recorded for the encoded section payload.
    checksum: AnyChecksum,
}

#[derive(Debug)]
struct ParsedMetadata {
    /// The parsed file-format major version.
    major_version: u32,
    /// The parsed file-format minor version.
    minor_version: u32,
    /// The parsed file-level flags.
    flags: u64,
    /// The parsed `section_table` entries from `FileMetadata`.
    section_table: Vec<SectionInfoRecord>,
    /// The parsed metadata extension fields.
    fields: Vec<TaggedField<u32>>,
    /// The parsed verification payload.
    verification: VerificationInfo,
    /// The `metadata_length` footer value.
    metadata_length: u64,
    /// The `file_length` footer value.
    file_length: u64,
    /// The byte offset at which the verification payload starts inside the metadata bytes.
    verification_offset: usize,
}

impl JanexFile {
    /// The Janex file header magic stored at the logical start of the file.
    pub const MAGIC_NUMBER: u64 = 0x5050_4158_454e_414a;
    /// The section type magic stored at the start of the encoded `FileMetadata` payload.
    pub const FILE_METADATA_MAGIC_NUMBER: u64 = 0x4154_4144_4154_454d;
    /// The footer marker used to locate `FileMetadata` from the end of the file.
    pub const END_MARK: u64 = 0x444e_4558_454e_414a;

    /// Creates an empty in-memory Janex file using the current format version defaults.
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

    /// Returns the first attributes section if present.
    pub fn attributes(&self) -> Option<&AttributesSection> {
        self.sections.iter().find_map(|section| match &section.content {
            SectionContent::Attributes(section) => Some(section),
            _ => None,
        })
    }

    /// Returns the external header bytes if present.
    pub fn external_header(&self) -> Option<&[u8]> {
        self.sections.iter().find_map(|section| match &section.content {
            SectionContent::ExternalHeader(section) => Some(section.as_ref()),
            _ => None,
        })
    }

    /// Returns the external tail bytes if present.
    pub fn external_tail(&self) -> Option<&[u8]> {
        self.sections.iter().find_map(|section| match &section.content {
            SectionContent::ExternalTail(section) => Some(section.as_ref()),
            _ => None,
        })
    }

    /// Returns the root configuration section if present.
    pub fn root_config_group(&self) -> Option<&RootConfigGroupSection> {
        self.sections.iter().find_map(|section| match &section.content {
            SectionContent::RootConfigGroup(section) => Some(section),
            _ => None,
        })
    }

    /// Returns the embedded resource-groups section if present.
    pub fn resource_groups(&self) -> Option<&ResourceGroupsSection> {
        self.sections.iter().find_map(|section| match &section.content {
            SectionContent::ResourceGroups(section) => Some(section),
            _ => None,
        })
    }

    /// Returns the string-pool section if present.
    pub fn string_pool(&self) -> Option<&StringPoolSection> {
        self.sections.iter().find_map(|section| match &section.content {
            SectionContent::StringPool(section) => Some(section),
            _ => None,
        })
    }

    /// Returns the data-pool section if present.
    pub fn data_pool(&self) -> Option<&DataPoolSection> {
        self.sections.iter().find_map(|section| match &section.content {
            SectionContent::DataPool(section) => Some(section),
            _ => None,
        })
    }

    /// Iterates over unknown sections preserved from the source file.
    pub fn unknown_sections(&self) -> impl Iterator<Item = &UnknownSection> + '_ {
        self.sections.iter().filter_map(|section| match &section.content {
            SectionContent::Unknown(section) => Some(section),
            _ => None,
        })
    }

    /// Reads and decodes one file resource by group name and resource path.
    pub fn read_file_resource_bytes(
        &self,
        group_name: &str,
        path: &str,
    ) -> Result<Option<Box<[u8]>>, Error> {
        let resource_groups = match self.sections.iter().find_map(|section| match &section.content {
            SectionContent::ResourceGroups(section) => Some(section),
            _ => None,
        }) {
            Some(section) => section,
            None => return Ok(None),
        };
        let data_pool = self.sections.iter().find_map(|section| match &section.content {
            SectionContent::DataPool(section) => Some(section),
            _ => None,
        });
        let string_pool = self.sections.iter().find_map(|section| match &section.content {
            SectionContent::StringPool(section) => Some(&section.strings),
            _ => None,
        });
        read_file_resource_from_sections(resource_groups, string_pool, data_pool, group_name, path)
    }

    /// Encodes the current file into the Janex binary format.
    ///
    /// Section payloads are serialized first so the metadata section can record
    /// their final lengths and derived checksums.
    pub fn write(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;

        let mut section_infos = Vec::with_capacity(self.sections.len());
        let mut encoded_sections = Vec::with_capacity(self.sections.len());
        let mut external_header = None;
        let mut external_tail = None;
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
            match section.content {
                SectionContent::ExternalHeader(_) => external_header = Some(bytes),
                SectionContent::ExternalTail(_) => external_tail = Some(bytes),
                _ => encoded_sections.push(bytes),
            }
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
        let external_header_length = external_header.as_ref().map_or(0, Vec::len);
        let external_tail_length = external_tail.as_ref().map_or(0, Vec::len);

        let mut writer =
            VecDataWriter::with_capacity(external_header_length + file_length + external_tail_length);
        if let Some(external_header) = &external_header {
            writer.write_all(external_header);
        }
        writer.write_u64_le(Self::MAGIC_NUMBER);
        for section in &encoded_sections {
            writer.write_all(section);
        }
        writer.write_all(&metadata_prefix);
        writer.write_all(&verification_bytes);
        writer.write_u64_le(Self::END_MARK);
        writer.write_u64_le(metadata_length as u64);
        writer.write_u64_le(file_length as u64);
        if let Some(external_tail) = &external_tail {
            writer.write_all(external_tail);
        }
        Ok(writer.into_inner())
    }

    /// Validates section ordering, uniqueness, and cross-section references before encoding.
    fn validate(&self) -> Result<(), Error> {
        let sections_len = self.sections.len();
        let mut root_config_group_count = 0usize;
        let mut attributes_count = 0usize;
        let mut resource_groups_count = 0usize;
        let mut string_pool_count = 0usize;
        let mut data_pool_count = 0usize;
        let mut external_header_position = None;
        let mut external_tail_position = None;
        let mut seen_section_keys = HashSet::with_capacity(self.sections.len());
        let mut seen_resource_group_names = HashSet::new();
        let mut local_group_names = HashSet::new();
        let mut string_pool_position = None;
        let mut resource_groups_position = None;
        let mut data_pool_len = None;
        let mut string_pool_ref = None;

        for (idx, section) in self.sections.iter().enumerate() {
            let key = (section.content.section_type().raw(), section.id);
            if !seen_section_keys.insert(key) {
                return Err(Error::InvalidSectionLayout(format!(
                    "duplicate section id {} for section type 0x{:016x}",
                    section.id,
                    section.content.section_type().raw()
                )));
            }

            match &section.content {
                SectionContent::Padding(_) => {}
                SectionContent::Attributes(_) => {
                    attributes_count += 1;
                }
                SectionContent::ExternalHeader(_) => {
                    external_header_position = Some(idx);
                }
                SectionContent::ExternalTail(_) => {
                    external_tail_position = Some(idx);
                }
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
                SectionContent::Unknown(_) => {}
            }
        }

        if attributes_count > 1
            || root_config_group_count > 1
            || resource_groups_count > 1
            || string_pool_count > 1
            || data_pool_count > 1
            || external_header_position.is_some_and(|_| {
                self.sections
                    .iter()
                    .filter(|section| matches!(section.content, SectionContent::ExternalHeader(_)))
                    .count()
                    > 1
            })
            || external_tail_position.is_some_and(|_| {
                self.sections
                    .iter()
                    .filter(|section| matches!(section.content, SectionContent::ExternalTail(_)))
                    .count()
                    > 1
            })
        {
            return Err(Error::InvalidSectionLayout(
                "Janex files may contain at most one external header, external tail, attributes, root config group, resource groups, string pool, and data pool section".to_string(),
            ));
        }

        if let Some(position) = external_header_position
            && position != 0
        {
            return Err(Error::InvalidSectionLayout(
                "the external header section must be the first section-table entry".to_string(),
            ));
        }

        if let Some(position) = external_tail_position
            && position + 1 != sections_len
        {
            return Err(Error::InvalidSectionLayout(
                "the external tail section must be the last section-table entry".to_string(),
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

    /// Sets the attributes section, replacing any existing one in place.
    pub fn with_attributes<S>(&mut self, section: S) -> &mut Self
    where
        S: Into<SectionBuilder<AttributesSection>>,
    {
        self.replace_unique_section(section.into().into_section(SectionContent::Attributes));
        self
    }

    /// Sets the external header section, replacing any existing one in place.
    pub fn with_external_header<S>(&mut self, section: S) -> &mut Self
    where
        S: Into<SectionBuilder<Box<[u8]>>>,
    {
        self.replace_unique_section(section.into().into_section(SectionContent::ExternalHeader));
        self
    }

    /// Sets the external tail section, replacing any existing one in place.
    pub fn with_external_tail<S>(&mut self, section: S) -> &mut Self
    where
        S: Into<SectionBuilder<Box<[u8]>>>,
    {
        self.replace_unique_section(section.into().into_section(SectionContent::ExternalTail));
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

    /// Adds an unknown section without replacing any existing section.
    pub fn push_unknown_section<S>(&mut self, section: S) -> &mut Self
    where
        S: Into<SectionBuilder<UnknownSection>>,
    {
        self.sections
            .push(section.into().into_section(SectionContent::Unknown));
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

    /// Replaces a singleton section type in place or appends it if it was not present yet.
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

    /// Converts the builder metadata and typed payload into an internal `Section`.
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

impl UnknownSection {
    /// Creates an unknown section from its raw section-type tag and payload.
    pub fn new(section_type: u64, bytes: Vec<u8>) -> Self {
        Self {
            section_type: SectionType::Unknown(section_type),
            bytes: bytes.into_boxed_slice(),
        }
    }
}

impl JanexMetadata {
    /// Validates metadata-level invariants that do not require decoding section bodies.
    pub fn validate(&self) -> Result<(), Error> {
        let sections_len = self.sections.len();
        let mut root_config_group_count = 0usize;
        let mut attributes_count = 0usize;
        let mut resource_groups_count = 0usize;
        let mut string_pool_count = 0usize;
        let mut data_pool_count = 0usize;
        let mut external_header_position = None;
        let mut external_tail_position = None;
        let mut seen_section_keys = HashSet::with_capacity(self.sections.len());
        let mut string_pool_position = None;
        let mut resource_groups_position = None;

        for (idx, section) in self.sections.iter().enumerate() {
            let key = (section.section_type.raw(), section.id);
            if !seen_section_keys.insert(key) {
                return Err(Error::InvalidSectionLayout(format!(
                    "duplicate section id {} for section type 0x{:016x}",
                    section.id,
                    section.section_type.raw()
                )));
            }

            match section.section_type {
                SectionType::Padding => {}
                SectionType::Attributes => {
                    attributes_count += 1;
                }
                SectionType::ExternalHeader => {
                    external_header_position = Some(idx);
                }
                SectionType::ExternalTail => {
                    external_tail_position = Some(idx);
                }
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
                SectionType::FileMetadata => {
                    return Err(Error::UnsupportedFeature(
                        "nested metadata sections are not implemented",
                    ));
                }
                SectionType::Unknown(_) => {}
            }
        }

        if attributes_count > 1
            || root_config_group_count > 1
            || resource_groups_count > 1
            || string_pool_count > 1
            || data_pool_count > 1
            || self
                .sections
                .iter()
                .filter(|section| section.section_type == SectionType::ExternalHeader)
                .count()
                > 1
            || self
                .sections
                .iter()
                .filter(|section| section.section_type == SectionType::ExternalTail)
                .count()
                > 1
        {
            return Err(Error::InvalidSectionLayout(
                "Janex files may contain at most one external header, external tail, attributes, root config group, resource groups, string pool, and data pool section".to_string(),
            ));
        }

        if let Some(position) = external_header_position
            && position != 0
        {
            return Err(Error::InvalidSectionLayout(
                "the external header section must be the first section-table entry".to_string(),
            ));
        }

        if let Some(position) = external_tail_position
            && position + 1 != sections_len
        {
            return Err(Error::InvalidSectionLayout(
                "the external tail section must be the last section-table entry".to_string(),
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

    /// Opens a Janex file whose logical end is known to be before the physical EOF.
    pub fn open_at_end(reader: R, janex_end_offset: u64) -> Result<Self, Error> {
        Self::open_at_end_with_verifier(
            reader,
            janex_end_offset,
            &RejectingDetachedSignatureVerifier,
        )
    }

    /// Opens a Janex file using a detached-signature verifier when needed.
    pub fn open_with_verifier<V: DetachedSignatureVerifier + ?Sized>(
        reader: R,
        verifier: &V,
    ) -> Result<Self, Error> {
        let mut reader = reader;
        let file_size = reader.seek(SeekFrom::End(0))?;
        Self::open_at_end_with_verifier(reader, file_size, verifier)
    }

    /// Opens a Janex file using a detached-signature verifier when its logical end is known.
    pub fn open_at_end_with_verifier<V: DetachedSignatureVerifier + ?Sized>(
        mut reader: R,
        janex_end_offset: u64,
        verifier: &V,
    ) -> Result<Self, Error> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        if janex_end_offset > file_size {
            return Err(Error::InvalidSectionLayout(
                "janex_end_offset is larger than the input size".to_string(),
            ));
        }
        if janex_end_offset < 24 {
            return Err(Error::UnexpectedEndOfFile);
        }

        let footer_bytes = read_exact_at(&mut reader, janex_end_offset - 24, 24)?;
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

        let file_start = janex_end_offset
            .checked_sub(file_length)
            .ok_or_else(|| Error::InvalidSectionLayout("file_length underflow".to_string()))?;
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
            let offset = match record.section_type {
                SectionType::ExternalHeader => file_start.checked_sub(record.length).ok_or_else(
                    || Error::InvalidSectionLayout("external header underflow".to_string()),
                )?,
                SectionType::ExternalTail => janex_end_offset,
                _ => {
                    let offset = next_section_offset;
                    next_section_offset =
                        next_section_offset
                            .checked_add(record.length)
                            .ok_or_else(|| {
                                Error::InvalidSectionLayout("section offset overflow".to_string())
                            })?;
                    offset
                }
            };
            let end = offset
                .checked_add(record.length)
                .ok_or_else(|| Error::InvalidSectionLayout("section offset overflow".to_string()))?;
            if end > file_size {
                return Err(Error::InvalidSectionLayout(
                    "section table points outside the input file".to_string(),
                ));
            }
            sections.push(SectionMetadata {
                section_type: record.section_type,
                id: record.id,
                options: record.options.clone(),
                length: record.length,
                checksum: record.checksum,
                offset,
            });
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

    /// Reads the first `Attributes` section, if present.
    pub fn read_attributes(&mut self) -> Result<Option<AttributesSection>, Error> {
        match self.read_first_section_of_type(SectionType::Attributes)? {
            Some(SectionContent::Attributes(section)) => Ok(Some(section)),
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

    /// Reads the first `ExternalHeader` section, if present.
    pub fn read_external_header(&mut self) -> Result<Option<Box<[u8]>>, Error> {
        match self.read_first_section_of_type(SectionType::ExternalHeader)? {
            Some(SectionContent::ExternalHeader(section)) => Ok(Some(section)),
            None => Ok(None),
            Some(_) => unreachable!(),
        }
    }

    /// Reads the first `ExternalTail` section, if present.
    pub fn read_external_tail(&mut self) -> Result<Option<Box<[u8]>>, Error> {
        match self.read_first_section_of_type(SectionType::ExternalTail)? {
            Some(SectionContent::ExternalTail(section)) => Ok(Some(section)),
            None => Ok(None),
            Some(_) => unreachable!(),
        }
    }

    /// Reads and decodes one file resource by group name and resource path.
    pub fn read_file_resource_bytes(
        &mut self,
        group_name: &str,
        path: &str,
    ) -> Result<Option<Box<[u8]>>, Error> {
        let string_pool = self.read_string_pool()?;
        let resource_groups = match self.read_resource_groups()? {
            Some(section) => section,
            None => return Ok(None),
        };
        let data_pool = self.read_data_pool()?;
        read_file_resource_from_sections(
            &resource_groups,
            string_pool.as_ref().map(|section| &section.strings),
            data_pool.as_ref(),
            group_name,
            path,
        )
    }

    /// Decodes the first indexed section whose type matches `section_type`.
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

    /// Reads, verifies, and parses one section body by index.
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

/// Encodes resource payload bytes according to a `Resource::File` `CompressInfo`.
pub fn encode_resource_content(
    info: &CompressInfo,
    uncompressed: &[u8],
    string_pool: &mut StringPool,
) -> Result<Vec<u8>, Error> {
    compress_resource_bytes(info, uncompressed, string_pool)
}

/// Decodes resource payload bytes according to a `Resource::File` `CompressInfo`.
pub fn decode_resource_content(
    info: &CompressInfo,
    compressed: &[u8],
    string_pool: Option<&StringPool>,
) -> Result<Vec<u8>, Error> {
    let data = decompress_resource_bytes(info, compressed, string_pool)?;
    if data.len() as u64 != info.uncompressed_size {
        return Err(Error::CompressionError(
            "uncompressed size does not match the declared size".to_string(),
        ));
    }
    Ok(data)
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

impl SectionType {
    /// Raw section-type tag for `Padding`.
    pub const PADDING_RAW: u64 = 0x0047_4e49_4444_4150;
    /// Raw section-type tag for `ExternalHeader`.
    pub const EXTERNAL_HEADER_RAW: u64 = 0x4441_4548_4c54_5845;
    /// Raw section-type tag for `ExternalTail`.
    pub const EXTERNAL_TAIL_RAW: u64 = 0x4c49_4154_4c54_5845;
    /// Raw section-type tag for `FileMetadata`.
    pub const FILE_METADATA_RAW: u64 = 0x4154_4144_4154_454d;
    /// Raw section-type tag for `Attributes`.
    pub const ATTRIBUTES_RAW: u64 = 0x2e53_4249_5254_5441;
    /// Raw section-type tag for `DataPool`.
    pub const DATA_POOL_RAW: u64 = 0x4c4f_4f50_4154_4144;
    /// Raw section-type tag for `RootConfigGroup`.
    pub const ROOT_CONFIG_GROUP_RAW: u64 = 0x5055_4f52_4747_4643;
    /// Raw section-type tag for `ResourceGroups`.
    pub const RESOURCE_GROUPS_RAW: u64 = 0x0053_5052_4753_4552;
    /// Raw section-type tag for `StringPool`.
    pub const STRING_POOL_RAW: u64 = 0x004c_4f4f_5052_5453;

    /// Returns the raw 64-bit section-type tag stored in `FileMetadata.section_table`.
    pub const fn raw(self) -> u64 {
        match self {
            SectionType::Padding => Self::PADDING_RAW,
            SectionType::ExternalHeader => Self::EXTERNAL_HEADER_RAW,
            SectionType::ExternalTail => Self::EXTERNAL_TAIL_RAW,
            SectionType::FileMetadata => Self::FILE_METADATA_RAW,
            SectionType::Attributes => Self::ATTRIBUTES_RAW,
            SectionType::DataPool => Self::DATA_POOL_RAW,
            SectionType::RootConfigGroup => Self::ROOT_CONFIG_GROUP_RAW,
            SectionType::ResourceGroups => Self::RESOURCE_GROUPS_RAW,
            SectionType::StringPool => Self::STRING_POOL_RAW,
            SectionType::Unknown(raw) => raw,
        }
    }

    /// Converts a raw 64-bit section-type tag into the typed `SectionType` representation.
    pub const fn from_raw(value: u64) -> Self {
        match value {
            Self::PADDING_RAW => SectionType::Padding,
            Self::EXTERNAL_HEADER_RAW => SectionType::ExternalHeader,
            Self::EXTERNAL_TAIL_RAW => SectionType::ExternalTail,
            Self::FILE_METADATA_RAW => SectionType::FileMetadata,
            Self::ATTRIBUTES_RAW => SectionType::Attributes,
            Self::DATA_POOL_RAW => SectionType::DataPool,
            Self::ROOT_CONFIG_GROUP_RAW => SectionType::RootConfigGroup,
            Self::RESOURCE_GROUPS_RAW => SectionType::ResourceGroups,
            Self::STRING_POOL_RAW => SectionType::StringPool,
            raw => SectionType::Unknown(raw),
        }
    }
}

impl TryFrom<u8> for CompressMethod {
    type Error = Error;

    /// Converts a raw compression-method tag into `CompressMethod`.
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

    /// Converts a raw section-type tag into `SectionType`, preserving unknown values.
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Ok(SectionType::from_raw(value))
    }
}

impl ParsedMetadata {
    /// Verifies the parsed metadata bytes using the configured verification payload.
    fn verify<V: DetachedSignatureVerifier + ?Sized>(
        &self,
        metadata_bytes: &[u8],
        verifier: &V,
    ) -> Result<(), Error> {
        self.verification
            .verify(&metadata_bytes[..self.verification_offset], verifier)
    }
}

/// Parses the encoded `FileMetadata` section footer and metadata records.
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

/// Reads one `SectionInfo` record from `FileMetadata.section_table`.
fn read_section_info_record<R: DataReader>(reader: &mut R) -> Result<SectionInfoRecord, Error> {
    Ok(SectionInfoRecord {
        section_type: SectionType::try_from(reader.read_u64_le()?)?,
        id: reader.read_vuint()?,
        options: read_len_prefixed_vec(reader, read_tagged_field_u32)?,
        length: reader.read_vuint()?,
        checksum: read_checksum(reader)?,
    })
}

/// Writes one `SectionInfo` record into `FileMetadata.section_table`.
fn write_section_info_record(
    writer: &mut VecDataWriter,
    record: &SectionInfoRecord,
) -> Result<(), Error> {
    writer.write_u64_le(record.section_type.raw());
    writer.write_vuint(record.id);
    write_len_prefixed_slice(writer, &record.options, write_tagged_field_u32)?;
    writer.write_vuint(record.length);
    write_checksum(writer, &record.checksum)?;
    Ok(())
}

/// Reads one `CompressInfo` structure from the input stream.
pub(crate) fn read_compress_info<R: DataReader>(reader: &mut R) -> Result<CompressInfo, Error> {
    Ok(CompressInfo {
        method: CompressMethod::try_from(reader.read_u8()?)?,
        uncompressed_size: reader.read_vuint()?,
        compressed_size: reader.read_vuint()?,
        options: reader.read_bytes()?,
    })
}

/// Writes one `CompressInfo` structure to the output stream.
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

/// Reads a `CompressedData<[u8]>` payload, decompresses it, and returns both the header and the decompressed bytes.
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

/// Writes a `CompressedData<[u8]>` payload using the provided compression template and uncompressed bytes.
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

/// Compresses arbitrary section bytes using the compression methods supported for non-resource payloads.
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

/// Compresses resource content bytes, including Janex's class-file-aware transform when requested.
fn compress_resource_bytes(
    info: &CompressInfo,
    data: &[u8],
    string_pool: &mut StringPool,
) -> Result<Vec<u8>, Error> {
    match info.method {
        CompressMethod::None => Ok(data.to_vec()),
        CompressMethod::Zstd => zstd::stream::encode_all(data, 0).map_err(Error::from),
        CompressMethod::Composite => {
            let layers = parse_composite_layers(&info.options)?;
            let mut current = data.to_vec();
            for layer in &layers {
                current = compress_resource_bytes(layer, &current, string_pool)?;
            }
            Ok(current)
        }
        CompressMethod::Classfile => crate::classfile::compress_with_string_pool(data, string_pool),
    }
}

/// Decompresses arbitrary section bytes using the compression methods supported for non-resource payloads.
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

/// Decompresses resource content bytes, resolving class-file-aware compression through the shared `StringPool`.
fn decompress_resource_bytes(
    info: &CompressInfo,
    data: &[u8],
    string_pool: Option<&StringPool>,
) -> Result<Vec<u8>, Error> {
    match info.method {
        CompressMethod::None => Ok(data.to_vec()),
        CompressMethod::Zstd => zstd::stream::decode_all(data).map_err(Error::from),
        CompressMethod::Composite => {
            let layers = parse_composite_layers(&info.options)?;
            let mut current = data.to_vec();
            for layer in layers.iter().rev() {
                current = decompress_resource_bytes(layer, &current, string_pool)?;
            }
            Ok(current)
        }
        CompressMethod::Classfile => {
            let string_pool = string_pool.ok_or_else(|| {
                Error::InvalidReference(
                    "classfile compression requires a string pool section".to_string(),
                )
            })?;
            crate::classfile::decompress_with_string_pool(data, string_pool)
        }
    }
}

/// Parses the `Vec<CompressInfo>` payload stored inside `CompressMethod::Composite` options.
fn parse_composite_layers(bytes: &[u8]) -> Result<Vec<CompressInfo>, Error> {
    let mut reader = ArrayDataReader::new(bytes);
    let layers = read_len_prefixed_vec(&mut reader, read_compress_info)?;
    ensure_fully_consumed(&reader, "composite compression options")?;
    Ok(layers)
}

/// Locates one file resource in the decoded sections, reads its bytes from the `DataPool`, decompresses them, and verifies optional checksums.
fn read_file_resource_from_sections(
    resource_groups: &ResourceGroupsSection,
    string_pool: Option<&StringPool>,
    data_pool: Option<&DataPoolSection>,
    group_name: &str,
    path: &str,
) -> Result<Option<Box<[u8]>>, Error> {
    let group = match resource_groups
        .groups
        .iter()
        .find(|group| group.name == group_name)
    {
        Some(group) => group,
        None => return Ok(None),
    };

    for resource in &group.resources {
        if resource.path().resolve(string_pool)? != path {
            continue;
        }

        if let Resource::File {
            compress_info,
            content_offset,
            fields,
            ..
        } = resource
        {
            let data_pool = data_pool.ok_or_else(|| {
                Error::InvalidReference(
                    "resource groups contain files but no data pool section is present"
                        .to_string(),
                )
            })?;
            let end = content_offset
                .checked_add(compress_info.compressed_size)
                .ok_or_else(|| {
                    Error::InvalidReference("resource content offset overflow".to_string())
                })?;
            if end > data_pool.bytes.len() as u64 {
                return Err(Error::InvalidReference(format!(
                    "resource '{}' points outside the data pool",
                    path
                )));
            }

            let compressed = &data_pool.bytes[*content_offset as usize..end as usize];
            let data = decode_resource_content(compress_info, compressed, string_pool)?;
            for field in fields {
                if let ResourceField::Checksum(checksum) = field {
                    verify_checksum(checksum, &data, "resource")?;
                }
            }
            return Ok(Some(data.into_boxed_slice()));
        }

        return Err(Error::InvalidReference(format!(
            "resource '{}' is not a regular file",
            path
        )));
    }

    Ok(None)
}

/// Reads a `TaggedPayload<u32>` from the input stream.
pub(crate) fn read_tagged_field_u32<R: DataReader>(
    reader: &mut R,
) -> Result<TaggedField<u32>, Error> {
    Ok(TaggedField {
        tag: reader.read_u32_le()?,
        payload: reader.read_bytes()?,
    })
}

/// Writes a `TaggedPayload<u32>` to the output stream.
pub(crate) fn write_tagged_field_u32(
    writer: &mut VecDataWriter,
    field: &TaggedField<u32>,
) -> Result<(), Error> {
    writer.write_u32_le(field.tag);
    writer.write_bytes(&field.payload);
    Ok(())
}

/// Reads a Janex `Vec<T>` using the supplied element decoder.
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

/// Writes a Janex `Vec<T>` using the supplied element encoder.
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

/// Encodes a nested payload into a temporary buffer and writes it as a length-prefixed byte sequence.
pub(crate) fn write_payload<F>(writer: &mut VecDataWriter, encode: F) -> Result<(), Error>
where
    F: FnOnce(&mut VecDataWriter) -> Result<(), Error>,
{
    let mut payload = VecDataWriter::new();
    encode(&mut payload)?;
    writer.write_bytes(&payload.into_inner());
    Ok(())
}

/// Reads an exact byte range from a seekable source.
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

/// Ensures that a nested decoder consumed the entire input slice it was given.
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

/// Converts a format integer to `usize`, rejecting values that do not fit on the host.
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

    fn sample_classfile_bytes() -> Vec<u8> {
        let mut writer = VecDataWriter::new();
        writer.write_u32_be(crate::classfile::ClassFile::MAGIC_NUMBER);
        writer.write_u16_be(0);
        writer.write_u16_be(53);
        writer.write_u16_be(12);
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_Utf8);
        writer.write_u16_be(15);
        writer.write_all(b"com/example/App");
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_Class);
        writer.write_u16_be(1);
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_Utf8);
        writer.write_u16_be(16);
        writer.write_all(b"java/lang/Object");
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_Class);
        writer.write_u16_be(3);
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_Utf8);
        writer.write_u16_be(9);
        writer.write_all(b"fieldName");
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_Utf8);
        writer.write_u16_be(1);
        writer.write_all(b"I");
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_NameAndType);
        writer.write_u16_be(5);
        writer.write_u16_be(6);
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_Utf8);
        writer.write_u16_be(7);
        writer.write_all(b"literal");
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_String);
        writer.write_u16_be(8);
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_Utf8);
        writer.write_u16_be(11);
        writer.write_all(b"com/example");
        writer.write_u8(crate::classfile::ConstantPoolInfo::TAG_Package);
        writer.write_u16_be(10);
        writer.write_u16_be(0x0021);
        writer.write_u16_be(2);
        writer.write_u16_be(4);
        writer.write_u16_be(0);
        writer.write_u16_be(0);
        writer.write_u16_be(0);
        writer.write_u16_be(0);
        writer.into_inner()
    }

    fn encode_composite_layers(layers: &[CompressInfo]) -> Result<Box<[u8]>, Error> {
        let mut writer = VecDataWriter::new();
        write_len_prefixed_slice(&mut writer, layers, write_compress_info)?;
        Ok(writer.into_inner().into_boxed_slice())
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

        assert!(file.root_config_group().is_some());
        assert!(file.string_pool().is_some());
        assert!(file.resource_groups().is_some());
        assert!(file.data_pool().is_some());

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

    #[test]
    fn external_unknown_and_forward_compatible_fields_roundtrip() -> Result<(), Error> {
        let section_checksum = Sha256Checksum::new([0; 32]).to_any();
        let mut string_pool = StringPool::with_empty_root();
        let dir_index = string_pool.push("com/example");
        let file_index = string_pool.push("Future.txt");

        let mut builder = JanexFile::builder();
        builder
            .with_external_header(
                SectionBuilder::from(b"#!/usr/bin/env janex\n".to_vec()).with_checksum(section_checksum),
            )
            .with_attributes(
                SectionBuilder::new(AttributesSection {
                    attributes: vec![Attribute {
                        name: "author".to_string(),
                        value: b"janex".to_vec().into_boxed_slice(),
                    }],
                })
                .with_checksum(section_checksum),
            )
            .with_string_pool(
                SectionBuilder::new(StringPoolSection::new(string_pool)).with_checksum(section_checksum),
            )
            .with_root_config_group(
                SectionBuilder::new(RootConfigGroupSection {
                    root_group: ConfigGroup {
                        fields: vec![
                            ConfigField::MainClass("com.example.Main".to_string()),
                            ConfigField::Unknown {
                                field_type: 0xface_cafe,
                                payload: b"future-config".to_vec().into_boxed_slice(),
                            },
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
                        resources: vec![Resource::File {
                            path: ResourcePath::Ref {
                                directory_index: dir_index,
                                file_name_index: file_index,
                            },
                            compress_info: CompressInfo {
                                method: CompressMethod::None,
                                uncompressed_size: 6,
                                compressed_size: 6,
                                options: Box::new([]),
                            },
                            content_offset: 0,
                            fields: vec![ResourceField::Unknown {
                                id: 0x77,
                                payload: b"future-resource".to_vec().into_boxed_slice(),
                            }],
                        }],
                    }],
                })
                .with_checksum(section_checksum),
            )
            .with_data_pool(
                SectionBuilder::new(DataPoolSection {
                    bytes: b"future".to_vec().into_boxed_slice(),
                })
                .with_checksum(section_checksum),
            )
            .push_unknown_section(
                SectionBuilder::new(UnknownSection::new(0x1122_3344_5566_7788, b"future-section".to_vec()))
                    .with_id(3)
                    .with_checksum(section_checksum),
            )
            .with_external_tail(
                SectionBuilder::from(b"launcher-jar".to_vec()).with_checksum(section_checksum),
            );

        let file = builder.build()?;
        let encoded = file.write()?;

        assert!(JanexArchive::open(Cursor::new(encoded.clone())).is_err());

        let janex_end_offset = encoded.len() as u64 - b"launcher-jar".len() as u64;
        let mut archive = JanexArchive::open_at_end(Cursor::new(encoded.clone()), janex_end_offset)?;
        assert_eq!(
            archive.read_external_header()?.unwrap().as_ref(),
            b"#!/usr/bin/env janex\n"
        );
        assert_eq!(archive.read_external_tail()?.unwrap().as_ref(), b"launcher-jar");
        assert_eq!(
            archive.read_attributes()?.unwrap().attributes[0].name,
            "author"
        );
        assert!(matches!(
            archive
                .sections()
                .iter()
                .find(|section| matches!(section.section_type, SectionType::Unknown(_)))
                .map(|section| section.section_type),
            Some(SectionType::Unknown(0x1122_3344_5566_7788))
        ));

        let decoded = archive.decode_all()?;
        assert_eq!(decoded.external_header(), Some(b"#!/usr/bin/env janex\n".as_slice()));
        assert_eq!(decoded.external_tail(), Some(b"launcher-jar".as_slice()));
        assert_eq!(decoded.attributes().unwrap().attributes[0].name, "author");
        assert!(matches!(
            decoded.unknown_sections().next().map(|section| section.section_type),
            Some(SectionType::Unknown(0x1122_3344_5566_7788))
        ));
        assert_eq!(decoded.write()?, encoded);
        Ok(())
    }

    #[test]
    fn classfile_resource_content_roundtrip() -> Result<(), Error> {
        let class_bytes = sample_classfile_bytes();
        let layers = [
            CompressInfo {
                method: CompressMethod::Classfile,
                uncompressed_size: 0,
                compressed_size: 0,
                options: Box::new([]),
            },
            CompressInfo {
                method: CompressMethod::Zstd,
                uncompressed_size: 0,
                compressed_size: 0,
                options: Box::new([]),
            },
        ];
        let compress_info = CompressInfo {
            method: CompressMethod::Composite,
            uncompressed_size: class_bytes.len() as u64,
            compressed_size: 0,
            options: encode_composite_layers(&layers)?,
        };

        let mut string_pool = StringPool::with_empty_root();
        let data = encode_resource_content(&compress_info, &class_bytes, &mut string_pool)?;
        let dir_index = string_pool.push("com/example");
        let file_index = string_pool.push("App.class");

        let section_checksum = Sha256Checksum::new([0; 32]).to_any();
        let mut builder = JanexFile::builder();
        builder
            .with_string_pool(
                SectionBuilder::new(StringPoolSection::new(string_pool.clone()))
                    .with_checksum(section_checksum),
            )
            .with_resource_groups(
                SectionBuilder::new(ResourceGroupsSection {
                    groups: vec![ResourceGroup {
                        name: "app".to_string(),
                        fields: Vec::new(),
                        resources_compression: CompressInfo::none(),
                        resources: vec![Resource::File {
                            path: ResourcePath::Ref {
                                directory_index: dir_index,
                                file_name_index: file_index,
                            },
                            compress_info: CompressInfo {
                                method: compress_info.method,
                                uncompressed_size: class_bytes.len() as u64,
                                compressed_size: data.len() as u64,
                                options: compress_info.options.clone(),
                            },
                            content_offset: 0,
                            fields: vec![ResourceField::Checksum(
                                Sha256Checksum::compute(&class_bytes).to_any(),
                            )],
                        }],
                    }],
                })
                .with_checksum(section_checksum),
            )
            .with_data_pool(
                SectionBuilder::new(DataPoolSection {
                    bytes: data.clone().into_boxed_slice(),
                })
                .with_checksum(section_checksum),
            );
        let file = builder.build()?;

        assert_eq!(
            file.read_file_resource_bytes("app", "com/example/App.class")?
                .unwrap()
                .as_ref(),
            class_bytes.as_slice()
        );

        let encoded = file.write()?;
        let mut archive = JanexArchive::open(Cursor::new(encoded))?;
        assert_eq!(
            archive
                .read_file_resource_bytes("app", "com/example/App.class")?
                .unwrap()
                .as_ref(),
            class_bytes.as_slice()
        );
        Ok(())
    }

    #[test]
    fn classfile_compression_uses_split_class_strings() -> Result<(), Error> {
        let class_bytes = sample_classfile_bytes();
        let mut string_pool = StringPool::with_empty_root();
        let transformed = encode_resource_content(
            &CompressInfo {
                method: CompressMethod::Classfile,
                uncompressed_size: class_bytes.len() as u64,
                compressed_size: 0,
                options: Box::new([]),
            },
            &class_bytes,
            &mut string_pool,
        )?;

        assert!(transformed.contains(&crate::classfile::ClassFile::TAG_EXTERNAL_UTF8_CLASS));
        assert!(transformed.contains(&crate::classfile::ClassFile::TAG_EXTERNAL_UTF8));
        assert!(string_pool.iter().any(|value| value == "java/lang"));
        assert!(string_pool.iter().any(|value| value == "Object"));
        assert!(string_pool.iter().any(|value| value == "com/example"));
        assert!(string_pool.iter().any(|value| value == "App"));
        assert!(string_pool.iter().any(|value| value == "fieldName"));
        assert!(string_pool.iter().any(|value| value == "I"));
        assert!(string_pool.iter().all(|value| value != "literal"));
        assert!(transformed
            .windows(b"literal".len())
            .any(|window| window == b"literal"));
        assert_eq!(
            decode_resource_content(
                &CompressInfo {
                    method: CompressMethod::Classfile,
                    uncompressed_size: class_bytes.len() as u64,
                    compressed_size: transformed.len() as u64,
                    options: Box::new([]),
                },
                &transformed,
                Some(&string_pool),
            )?,
            class_bytes
        );
        Ok(())
    }

    #[test]
    fn reject_invalid_external_section_order() {
        let mut builder = JanexFile::builder();
        builder
            .push_padding(b"pad".to_vec())
            .with_external_header(b"header".to_vec());
        let error = builder.build().unwrap_err();

        match error {
            Error::InvalidSectionLayout(message) => {
                assert!(message.contains("external header section must be the first"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}

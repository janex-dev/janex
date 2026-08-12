# Janex File Format

Janex is a sectioned container format for packaging Java applications. It supports isolated resource
groups, the Java module system, compressed and shared content, remote dependencies, embedded launch
configuration, conditional runtime selection, and optional data before or after the Janex container.

The Janex Launcher selects a Java runtime and starts the application. Janex Boot provides the class
loader that reads classes and resources from the container.

## Data Types

### Basic Data Types

Janex uses little-endian encoding for fixed-width binary integer and floating-point fields. `vuint`
and values inside CBOR objects use the encodings defined in their respective sections below.

This document uses `u8`/`u16`/`u32`/`u64` to represent 8/16/32/64-bit unsigned integers,
uses `i8`/`i16`/`i32`/`i64` to represent 8/16/32/64-bit signed integers,
and uses `f32`/`f64` to represent 32/64-bit floating-point numbers.

`bool` is represented by `u8`, where `true` is any non-zero value and `false` is zero.

### Complex Data Types

Complex layouts use Rust-like pseudocode. `[T; count]` denotes `count` consecutive values of type `T`.

### Variable-length integers

Janex calls a 64-bit unsigned integer encoded as ULEB128 a `vuint`:

```rust
type vuint = u64;
```

A `vuint` occupies one to ten bytes. Each byte carries seven value bits and uses its most significant
bit as the continuation flag:

- If the MSB is `0`, the current byte is the last byte of the integer.
- If the MSB is `1`, more bytes follow; the next byte continues the encoding.

The least-significant group comes first. The tenth byte may contain at most one nonzero value bit.
Writers use the shortest encoding; readers also accept zero-padded encodings within this limit.

### Dynamic Array

Dynamic arrays store their element count followed by the encoded elements:

```rust
struct Vec<T> {
    /// The number of elements in the array.
    elements_count: vuint,

    /// The array elements, each serialized according to the type `T`.
    elements: [T; elements_count],
}
```

### Byte-Sized Values

`Sized<T>` adds an encoded byte-length boundary around one value:

```rust
struct Sized<T> {
    /// The encoded byte size of `value`, excluding this field.
    byte_size: vuint,

    /// A value whose encoding occupies exactly `byte_size` bytes.
    value: T,
}
```

`byte_size` counts only the encoded bytes of `value`. The value must occupy exactly that many bytes;
`Sized<T>` adds no alignment or padding.

### String

`String` is a UTF-8 byte vector:

```rust
type String = Vec<u8>;
```

### Tagged Payload

`TaggedPayload<T>` prefixes a payload with a tag and its byte length:

```rust
struct TaggedPayload<T> {
    // Always equal to `TAG`.
    tag: T,
    
    /// The number of bytes of the payload.
    payload_bytes: vuint,
    
    /// The payload bytes.
    payload: [u8; payload_bytes],
}
```

This document uses `#[repr(TaggedPayload<T>)]` to denote this layout.

### CBOR

Janex uses [CBOR](https://www.rfc-editor.org/rfc/rfc8949.html) for extensible metadata. Schemas are
written in [CDDL](https://www.rfc-editor.org/rfc/rfc8610.html).

The following non-generic aliases represent bare CBOR values without a Janex length prefix:

```rust
/// Exactly one deterministic CBOR data item permitted by the applicable schema.
type CborValue = ...;

type CborBool = CborValue;  // bool
type CborUint = CborValue;  // uint
type CborInt = CborValue;   // int
type CborBytes = CborValue; // bstr
type CborText = CborValue;  // tstr
type CborArray = CborValue; // array
type CborMap = CborValue;   // map
type CborNull = CborValue;  // null
```

The aliases describe the top-level CBOR type; their contents are defined by the CDDL schema at each use
site. A bare value is used inside CBOR or when an enclosing structure already supplies its byte
boundary. Other binary fields use `Sized<CborValue>` or a more specific alias.

All CBOR values must follow the Core Deterministic Encoding Requirements in RFC 8949 Section 4.2.1 and
match their applicable schema. A `Sized<CborValue>` contains exactly one complete CBOR item.

### `Checksum`

```rust
struct Checksum {
    /// The checksum algorithm.
    algorithm: ChecksumAlgorithm,

    /// The length-prefixed checksum value.
    checksum: Vec<u8>,
}
```

Currently supported checksum algorithms:

```rust
#[repr(u16)]
enum ChecksumAlgorithm {
    NONE = 0,     // 0 bytes
    
    XXH64 = 0x0101,  // 8 bytes
    
    SHA256 = 0x8101, // 32 bytes

    SHA512 = 0x8102, // 64 bytes
    
    SM3 = 0x8301,    // 32 bytes
}
```

The checksum length must match the algorithm. Unknown algorithms can be skipped but cannot satisfy a
required validation.

CBOR metadata represents the same value as a two-element array:

```cddl
ChecksumObject = [
    algorithm: 0..65535,
    checksum: bstr,
]
```

The array must contain exactly two elements. `algorithm` must fit in `u16`, and `checksum` must satisfy
the same algorithm-specific length requirements as the binary `Checksum` structure.

## Janex File Structure

The Janex container has the following layout:

```rust
struct JanexFile {
    /// The magic number identifying this as a Janex file.
    ///
    /// Always `0x5050_4158_454e_414a` ("JANEXAPP").
    magic_number: u64, // 0x50504158454e414a  ("JANEXAPP")

    /// The sections of the Janex file.
    sections: [Section; ...],
}
```

The complete physical file may contain data outside `JanexFile`:

```text
[external header] [JanexFile] [external tail]
```

`FileMetadataObject` may constrain the external regions. They are not Janex sections and do not appear
in the section table.

### `FileMetadata` Section

```rust
struct FileMetadataSection {
    /// The magic number identifying the `FileMetadata` section.
    magic_number: u64, // 0x4154_4144_4154_454d ("METADATA")

    /// The format major version, currently `0`. Unsupported values are invalid.
    major_version: u32,

    /// The format minor version, currently `1`.
    ///
    /// With major version `0`, readers accept only explicitly supported minor versions. For later
    /// major versions, readers should accept higher minor versions where unknown fields are skippable.
    /// Specification edits do not automatically change this value.
    minor_version: u32,

    /// The deterministic CBOR file-metadata map.
    metadata: Sized<CborMap>, // FileMetadataObject
    
    /// The verification information.
    verification_info: VerificationInfo,
    
    /// Always `0x444e_4558_454e_414a` ("JANEXEND").
    end_mark: u64,  // 0x444e_4558_454e_414a ("JANEXEND")
    
    /// The length in bytes of the metadata section.
    metadata_length: u64,

    /// The total byte length of `JanexFile`.
    file_length: u64,
}
```

`metadata.value` must contain a `FileMetadataObject`, represented by the following deterministic CBOR
map:

```cddl
FileMetadataObject = {
    0: [* SectionInfoObject],          ; section_table
    ? 1: ExternalRegionObject,         ; external_header
    ? 2: ExternalRegionObject,         ; external_tail
    * uint => any,
}

NonemptyText = tstr .ne ""
```

`section_table` excludes the final `FileMetadata` section and may be empty.

#### Metadata Evolution and Extensions

Core maps use unsigned integer keys. Text-keyed metadata uses non-empty keys; `janex.` is reserved.
Third-party keys should use a reverse-domain prefix such as `org.example.`.

Unknown keys and section types may be ignored and must not change the meaning of known data. Semantic
rewrites must preserve unknown keys. Changes that alter core interpretation require a format version
that older readers reject.

Empty collections are valid where allowed by the schema. Optional empty maps should be omitted. `null`
is valid only where the schema defines it and is distinct from an omitted key.

#### `SectionInfoObject` Map

Each element of `FileMetadataObject.section_table` is a deterministic CBOR map:

```cddl
SectionInfoObject = {
    0: uint,                         ; section_type
    1: uint,                         ; id
    2: uint,                         ; length
    3: ChecksumObject,               ; checksum
    * uint => any,
}
```

For sections with a magic number, `section_type` normally equals that number. `(section_type, id)` must
be unique within the file. IDs are file-local, may be sparse, and carry no ordering or semantic
meaning. Singleton section types use ID `0`.

`length` is the exact encoded section length. `checksum` covers those bytes and must be verified unless
its algorithm is `NONE`.

Keys `0` through `3` are common to all sections. Other keys are defined by `section_type` and belong
directly to the same map.

#### Section References

A typed section reference is a bare CBOR item. Its logical type determines the required `section_type`
but has no binary representation:

```cddl
SectionRef =
    uint
  / [
        reference_kind: SectionReferenceKindObject,
        payload: bstr .cbor any,
    ]

SectionReferenceKindObject = uint / NonemptyText
```

The `uint` form resolves `(T, id)` in the same file; `id` must fit in `u64`. All core references use
this form.

The array form is reserved for extended reference mechanisms. Its kind is a specification-assigned
integer or an extension name. `payload` contains exactly one deterministic CBOR item. Unsupported
kinds cannot be resolved. No core extended kinds are defined.

Currently supported section types:

```rust
#[repr(u64)]
enum SectionType {
    /// Arbitrary padding bytes; no section magic number is required.
    Padding = 0x0047_4e49_4444_4150, // "PADDING\0"

    /// The final section. It is omitted from its own section table.
    FileMetadata = 0x4154_4144_4154_454d, // "METADATA"

    Attributes = 0x2e53_4249_5254_5441, // "ATTRIBS."
    
    BlobPool = 0x4c4f_4f50_424f_4c42, // "BLOBPOOL"
    
    RootConfigGroup = 0x5055_4f52_4747_4643, // "CFGGROUP"

    ResourceGroups = 0x0053_5052_4753_4552, // "RESGRPS\0"

    StringPool = 0x004c_4f4f_5052_5453, // "STRPOOL\0"
}
```

The table describes consecutive sections. Use `Padding` for bytes between them.

Unknown sections may be skipped.

#### `ExternalRegionObject`

`ExternalRegionObject` describes and constrains bytes outside `JanexFile`:

```cddl
ExternalRegionObject = [
    size: uint,
    checksum: ChecksumObject,
]
```

An omitted `external_header` or `external_tail` key imposes no restriction on that region. A present
key requires the declared size and checksum. A zero size requires `NONE` and explicitly requires
absence; a nonzero size requires a non-`NONE` checksum. Readers must compare the actual size and verify
the non-`NONE` checksum.

For a physical file of `physical_file_size` bytes, a reader receives `external_tail_length` from its
caller and calculates:

```text
janex_end   = physical_file_size - external_tail_length
janex_start = janex_end - file_length
```

The caller supplies `external_tail_length`; standalone Janex uses zero. Both subtractions use checked
arithmetic. The external header precedes `janex_start`, and the external tail starts at `janex_end`.
Janex does not infer the tail length by scanning the file.

The 24-byte footer immediately before `janex_end` locates `FileMetadataSection` through
`metadata_length` and the container start through `file_length`.

`end_mark` must match, `metadata_length` must equal the encoded `FileMetadataSection` length, and:

```text
file_length = 8 + sum(FileMetadataObject.section_table[*].length) + metadata_length
```

The equation must be evaluated with checked arithmetic.

#### `VerificationInfo` Structure

`VerificationInfo` is a tagged payload. `verification_type` is included in `verification_input`; its
payload is not.

```rust
#[repr(TaggedPayload<u8>)]
enum VerificationInfo {
    /// No verification.
    None {
        verification_type: u8, // 0

        /// Always `0`.
        payload_bytes: vuint,
    },

    /// Detects accidental corruption of the metadata.
    Checksum {
        verification_type: u8, // 1

        /// The exact encoded size of `checksum`.
        payload_bytes: vuint,

        /// The checksum of `verification_input`.
        ///
        /// The algorithm must not be `NONE`.
        checksum: Checksum,
    },

    /// Authenticates the metadata using a detached OpenPGP signature.
    OpenPGP {
        verification_type: u8, // 2

        /// The exact number of bytes in `signature`.
        payload_bytes: vuint,

        /// One binary OpenPGP Signature packet conforming to the Janex OpenPGP profile.
        signature: [u8; payload_bytes],
    },

    /// Authenticates the metadata using a detached CMS signature.
    CMS {
        verification_type: u8, // 3

        /// The exact number of bytes in `signature`.
        payload_bytes: vuint,

        /// One DER-encoded CMS ContentInfo value conforming to the Janex CMS profile.
        signature: [u8; payload_bytes],
    },
}
```

`verification_input` is the following byte string:

```text
file_bytes[file_metadata_start .. verification_type_end]
```

The range starts at `FileMetadataSection.magic_number` and ends immediately after
`verification_type`. Verification uses these original bytes without re-encoding them.

The payload must consume exactly `payload_bytes`. OpenPGP and CMS payloads must be nonempty. Unknown
verification types are invalid.

##### Verification Policy

`None` and `Checksum` provide no authentication and must be rejected when authentication is required.
OpenPGP and CMS use caller-provided signer, trust, algorithm, time, and revocation policies. A malformed
or invalid signature is invalid under its declared type; readers must not fall back to another type.

OpenPGP and CMS implementations must support SHA-256 for content digests. Janex signatures must not
use MD5, SHA-1, or RIPEMD-160. Other algorithms are subject to caller policy.

##### OpenPGP Profile

The OpenPGP payload uses the binary packet format defined by
[RFC 9580](https://www.rfc-editor.org/rfc/rfc9580.html). It must contain exactly one Signature packet
and no Marker, Padding, Literal Data, One-Pass Signature, compressed, encrypted, or ASCII-armored
representation. The Signature packet must:

- use packet version 4 or 6;
- use signature type `0x00` (Binary Signature of a Document), with `verification_input` as the exact
  document bytes;
- contain the Signature Creation Time subpacket in its hashed subpacket area;
- contain exactly one Issuer Fingerprint subpacket in its hashed subpacket area, matching the key that
  verifies the signature; and
- satisfy the caller's algorithm, key-strength, key-usage, expiration, and revocation policies.

Unhashed issuer information is advisory. Unsupported critical subpackets are invalid. Keys and trust
data come from the caller or an external key store.

##### CMS Profile

The CMS payload uses the syntax defined by
[RFC 5652](https://www.rfc-editor.org/rfc/rfc5652.html), with the algorithm-protection updates in
[RFC 8933](https://www.rfc-editor.org/rfc/rfc8933.html). It must be exactly one DER-encoded
`ContentInfo` value satisfying all of the following requirements:

- `ContentInfo.contentType` is `id-signedData` and its content is one `SignedData` value;
- `SignedData.encapContentInfo.eContentType` is `id-data` and `eContent` is absent, making the
  signature detached;
- `signerInfos` is non-empty;
- every `SignerInfo` considered by the caller's policy contains signed attributes;
- those signed attributes contain exactly one `content-type` attribute whose value is `id-data`,
  exactly one `message-digest` attribute equal to the digest of `verification_input`, and exactly one
  `CMSAlgorithmProtection` attribute matching that `SignerInfo`'s digest and signature algorithms;
  and
- the same digest algorithm is used for the content digest and the signed attributes, as required by
  RFC 8933.

Each required signer must pass digest, signature, identity, and caller-policy validation. Embedded
certificates and revocation data are not trusted merely because they are embedded. Unsigned attributes
do not affect the primary signature. Caller policy selects the required signers.

##### Authenticated Content Scope

Full-container authentication requires a secure checksum for every section and every nonempty external
region, with both `external_header` and `external_tail` present. Readers must verify all of them.
`SHA256`, `SHA512`, and `SM3` qualify; `NONE` and `XXH64` do not. An omitted external-region key places
that region outside this scope.

The signature binds metadata and the checksums recorded in it. It does not directly sign the
verification payload, footer, or complete physical file representation.

### `Attributes` Section

```rust
struct AttributesSection {
    /// The magic number identifying this as an attributes section.
    ///
    /// Always `0x2e53_4249_5254_5441` ("ATTRIBS.").
    magic_number: u64, // 0x2e53_4249_5254_5441 ("ATTRIBS.")

    /// One deterministic CBOR `AttributesObject`.
    attributes: CborMap, // AttributesObject
}
```

`attributes` occupies the remainder of the section. A file may contain at most one `Attributes`
section, with ID `0`. Attributes are descriptive unless their definitions assign operational semantics.

The CBOR object is a text-keyed map:

```cddl
AttributesObject = { * NonemptyText => any }
```

Attribute names follow the extension naming rules. Unknown attributes may be ignored. Writers should
omit an empty section, but readers must accept its empty map.

### `RootConfigGroup` Section

Each Janex file may contain at most one `RootConfigGroup` section, whose section ID must be `0`.

```rust
struct RootConfigGroupSection {
    /// The magic number identifying the `RootConfigGroup` section.
    ///
    /// Always `0x5055_4f52_4747_4643` ("CFGGROUP").
    magic_number: u64, // 0x5055_4f52_4747_4643 "CFGGROUP"

    /// One deterministic CBOR `ConfigGroupObject`.
    root_group: CborMap, // ConfigGroupObject
}
```

`root_group` occupies the remainder of the section.

#### `ConfigGroupObject` Map

```cddl
ConfigGroupObject = {
    ? 0: tstr,                                  ; condition
    ? 1: (tstr / null),                         ; main_class
    ? 2: (tstr / null),                         ; main_module
    ? 3: ([* ResourceGroupReferenceObject] / null), ; module_path
    ? 4: ([* ResourceGroupReferenceObject] / null), ; class_path
    ? 5: ([* JavaAgentObject] / null),          ; agents
    ? 6: ([* tstr] / null),                     ; jvm_options
    ? 7: [* ConfigGroupObject],                 ; subgroups
    * uint => any,
}
```

An omitted `condition` is unconditional. Otherwise, it is a CEL expression described in
[Conditions](#conditions).

The launcher visits the root and its `subgroups` in depth-first pre-order. Each subgroup is evaluated
independently. For a group whose condition matches, the configuration keys are applied as follows:

- a missing key makes no contribution;
- `main_class` and `main_module` replace the value previously selected, while `null` clears it;
- an array in `module_path`, `class_path`, `agents`, or `jvm_options` appends its elements in array
  order, while `null` clears all elements accumulated for that key; and
- an empty array is valid and appends nothing.

`subgroups` preserves array order and must not be `null`.

#### `ResourceGroupReferenceObject`

A resource-group reference is one of these CBOR arrays:

```cddl
ResourceGroupReferenceObject =
    [local: 0, resource_groups: SectionRef, group_name: tstr]
  / [remote: 1, purl: tstr, checksum: ChecksumObject]
```

Variant `0` selects one named group from the referenced `ResourceGroups` section.

Variant `1` identifies one concrete package version by canonical Package URL. The launcher downloads
it without resolving transitive dependencies and verifies `checksum`. A `vers` qualifier is not
allowed. Maven artifacts use `pkg:maven`; `repository_url` selects a non-default repository.

#### `JavaAgentObject`

```cddl
JavaAgentObject = [reference: ResourceGroupReferenceObject, option: tstr]
```

An empty `option` means that no agent option is supplied.

### `ResourceGroups` Section

`ResourceGroups` contains named embedded resource groups. A file may contain any number of these
sections.

For a `ResourceGroups` section, `SectionInfoObject` additionally defines this field:

```cddl
ResourceGroupsSectionInfoFields = (
    ? 4: SectionRef,        ; string_pool
)
```

`string_pool` selects the pool used by all `ClassFile` transforms in the section. It is required when
such a transform occurs. Group names must be unique within the section.

```rust
struct ResourceGroupsSection {
    /// The magic number identifying this as a resource group.
    ///
    /// Always `0x0053_5052_4753_4552` ("RESGRPS\0").
    magic_number: u64, // 0x0053_5052_4753_4552 ("RESGRPS\0")
    
    /// The resource groups.
    groups: Vec<ResourceGroup>,
}
```

#### `ResourceGroup`

A `ResourceGroup` is a named collection of resources, typically corresponding to a JAR or module. Its
directories form a flat list; paths carry the hierarchy.

```rust
struct ResourceGroup {
    /// The magic number identifying this as a resource group.
    ///
    /// Always `0x47534552` ("RESG").
    magic_number: u32, // 0x47534552 ("RESG")

    /// The unique name of this resource group within the `ResourceGroups` section.
    ///
    /// This name is referenced by local `ResourceGroupReferenceObject` values.
    name: String,

    /// One deterministic CBOR `ResourceGroupMetadataObject`.
    metadata: Sized<CborMap>, // ResourceGroupMetadataObject

    /// The directories in path order.
    directories: Vec<ResourceDirectory>,
}
```

The resource-group metadata is a text-keyed map:

```cddl
ResourceGroupMetadataObject = { * NonemptyText => any }
```

Names follow the extension naming rules. The required map may be empty; unknown entries do not affect
resource decoding.

#### `ResourceDirectory`

Each directory record contains its direct non-directory entries. The entry payload may be inline or
stored in one or more blobs through `Content`.

```rust
struct ResourceDirectory {
    /// The directory path relative to the resource-group root.
    path: String,

    /// One deterministic CBOR resource-metadata map.
    metadata: Sized<CborMap>, // ResourceMetadataObject

    /// The number of direct entries in this directory.
    entries_count: vuint,

    /// The direct entries in name order.
    entries: Content<[DirectoryEntry; entries_count]>,
}
```

The empty path identifies the root directory. Other directory paths are UTF-8, `/`-separated, and
must not start or end with `/` or contain empty, `.` or `..` components. Directory paths are unique
and sorted by their UTF-8 bytes. Parent directories may be implicit; an empty directory or its
metadata is preserved by an explicit record with no entries.

`entries` decodes to exactly `entries_count` values and has no content transforms. Inline entry bytes
can be skipped while scanning the directory list because their length is encoded by `ContentSource`.

#### `DirectoryEntry`

`DirectoryEntry` represents a regular file or symbolic link directly contained by a
`ResourceDirectory`:

```rust
enum DirectoryEntry {
    /// Represents a regular file.
    File {
        /// The resource type tag for this variant.
        ///
        /// Always `0x00534552` ("RES\0").
        resource_type: u32, // 0x00534552 ("RES\0")

        /// The file name within the directory.
        name: String,

        /// The content of this file and its logical transforms.
        content: Content<[u8]>,

        /// One deterministic CBOR resource-metadata map.
        metadata: Sized<CborMap>, // ResourceMetadataObject
    },

    /// Represents a symbolic link.
    SymbolicLink {
        /// The resource type tag for this variant.
        ///
        /// Always `0x4c4d5953` ("SYML").
        resource_type: u32, // 0x4c4d5953 ("SYML")

        /// The symbolic-link name within the directory.
        name: String,

        /// The relative target path.
        target: String,

        /// One deterministic CBOR resource-metadata map.
        metadata: Sized<CborMap>, // ResourceMetadataObject
    },
}
```

Entry names are nonempty UTF-8 strings without `/` and must not be `.` or `..`. They are unique within
their directory and sorted by their UTF-8 bytes. A full resource path is the entry name for the root
directory, or `directory_path + "/" + entry_name` otherwise. Directory records and entries must not
produce conflicting paths.

Symbolic-link targets use normalized relative `/`-separated paths and follow the nonempty path rules
used for non-root directory paths.

#### Resource Metadata

Each `ResourceDirectory` and `DirectoryEntry` contains one `Sized<CborMap>` whose value is a
`ResourceMetadataObject`:

```cddl
ResourceMetadataObject = {
    ? 0: ChecksumObject,             ; checksum
    ? 1: tstr,                       ; comment
    ? 2: UnixNanosecondsObject,      ; creation_time
    ? 3: UnixNanosecondsObject,      ; modification_time
    ? 4: UnixNanosecondsObject,      ; access_time
    ? 5: 0..65535,                   ; posix_permissions
    * uint => any,
}

UnixNanosecondsObject =
    int
  / #6.2(bstr .size (9..16))
  / #6.3(bstr .size (9..16))
```

An empty map represents no metadata.

`checksum` is valid only for regular files and covers the logical content after transforms are
reversed. `posix_permissions` contains the POSIX permission bits.

Time values are signed `i128` POSIX timestamps in nanoseconds. Values in CBOR's basic integer range use
major type `0` or `1`. Larger values use tag `2` or `3` with a minimal 9-to-16-byte big-endian
magnitude; tag `3` encodes `-1 - value`.

### Content

`Content<T>` stores an encoded value inline, in one blob, or across blob slices. `T` is the logical
type obtained after reversing its transforms and has no binary representation of its own:

```rust
struct Content<T> {
    /// The bytes produced after applying all content transforms.
    source: ContentSource,

    /// The transforms in the order in which the encoder applied them.
    transforms: Vec<ContentTransform>,
}
```

`ContentSource` locates the transformed bytes:

```rust
#[repr(u8)]
enum ContentSource {
    /// Stores the complete transformed byte sequence inline.
    Inline {
        source_type: u8, // 0
        bytes: Vec<u8>,
    },

    /// Uses the complete decoded representation of one blob.
    Blob {
        source_type: u8, // 1
        blob: BlobRef,
    },

    /// Concatenates decoded blob ranges in array order.
    BlobSlices {
        source_type: u8, // 2
        slices: Vec<BlobSlice>,
    },
}
```

`Inline` supplies `bytes`, `Blob` supplies one complete decoded blob, and `BlobSlices` concatenates
decoded ranges in array order. `BlobSlices` must be nonempty and must not represent one complete blob;
use `Blob` for that case. Slices are nonempty, so empty content uses an empty `Inline` value.

Content transforms are described by the following structures:

```rust
struct ContentTransform {
    /// The number of bytes supplied to this transform by the encoder.
    input_size: vuint,

    /// Identifies the content transform.
    method: ContentTransformId,

    /// A byte-sized deterministic CBOR map required to reverse the transform.
    properties: Sized<CborMap>, // ContentTransformPropertiesObject
}

#[repr(u8)]
enum ContentTransformId {
    /// A class-file-aware transform using the shared `StringPool`.
    CLASSFILE = 1,
}
```

```cddl
ContentTransformPropertiesObject = { * uint => any }
ClassFileTransformPropertiesObject = {}
```

The schema of `properties.value` is selected by `method`.

Transforms are stored in encoding order and reversed from last to first after blob decoding and slice
concatenation. Each result must match `input_size`; the final value must be a valid encoding of `T`.
An empty transform array means the source already encodes `T`. Unsupported methods are invalid.

`CLASSFILE` is valid only for regular-file `Content<[u8]>`. The transform arrays of
`ResourceDirectory.entries` and `StringPoolSection.data` must therefore be empty.

#### Class File Transform

The class-file transform moves selected constant-pool strings into a shared `StringPool`.

It modifies the class file as follows:

1. The magic number of the transformed class file is rewritten to `0x70CAFECA`
   (`0xCA 0xFE 0xCA 0x70` in file order) to distinguish it from an unmodified class file.
2. The transformed class file may contain new constant types that reference entries in the shared
   `StringPool` by index, replacing the original `CONSTANT_Utf8` entries.

    New constant pool entries include:

    1. `CONSTANT_External_String`:

        ```rust
        struct CONSTANT_External_Utf8 {
            tag: u8, // 0xFF

            /// The index of the string in the shared `StringPool`.
            string_pool_index: vuint,
        }
        ```

    2. `CONSTANT_External_String_Class`:

        ```rust
        struct CONSTANT_External_String {
            tag: u8, // 0xFE

            /// The index of the package name in the shared `StringPool`.
            package_name_index: vuint,

            /// The index of the class name in the shared `StringPool`.
            class_name_index: vuint,
        }
        ```

The input must be a valid Java class file. Reversal uses the `StringPool` selected by the containing
`ResourceGroups` section. `CLASSFILE` properties must be empty.

### `StringPool` Section

A `StringPool` supplies strings for class-file transforms. A file may contain any number of pools, and
multiple `ResourceGroups` sections may share one.

```rust
struct StringPoolSection {
    /// The magic number identifying this section as a string pool.
    /// 
    /// Always `0x004c_4f4f_5052_5453` ("STRPOOL\0")
    magic_number: u64, // 0x004c_4f4f_5052_5453 ("STRPOOL\0")

    /// The string-pool data.
    data: Content<StringPoolData>,
}

struct StringPoolData {
    /// The strings in pool-index order.
    ///
    /// This vector must contain at least one element, and element `0` must be an empty string.
    strings: Vec<String>,
}
```

Strings use UTF-8 and are converted to Modified UTF-8 when restoring a class file.
`StringPoolSection.data.transforms` must be empty; its source may use any `ContentSource` variant.

### `BlobPool` Section

A `BlobPool` stores independently decoded byte blobs. Their logical types and transforms belong to
`Content<T>`, not the pool. A file may contain any number of pools.

```rust
struct BlobPoolSection {
    magic_number: u64, // 0x4c4f_4f50_424f_4c42 ("BLOBPOOL")

    /// Stored blobs and blob-table pages.
    bytes: [u8; ...],
}
```

#### Blob Encoding

Blob and table-page encodings use these CBOR objects:

```cddl
BlobEncodingObject = [stored_size: uint, filters: [* BlobFilterObject]]
BlobFilterObject = [input_size: uint, method: 0..255, properties: BlobFilterPropertiesObject]
BlobFilterPropertiesObject = { * uint => any }
```

Filters are stored in encoding order and reversed from last to first. Each result must match
`input_size`. With no filters, the decoded size is `stored_size`; otherwise, it is the first filter's
`input_size`.

The supported blob filters are:

```rust
#[repr(u8)]
enum BlobFilterId {
    /// Zstandard (zstd) compression.
    ///
    /// See https://github.com/facebook/zstd for details.
    ZSTD = 1,
}
```

The properties schema is selected by `method`. Unsupported filters are invalid. `ZSTD` properties must
be empty.

#### Blob Table

For a `BlobPool` section, `SectionInfoObject` additionally contains a one-level page directory:

```cddl
BlobPoolSectionInfoFields = (
    4: uint,                           ; blob_count
    5: 256..4096,                      ; page_capacity
    6: [* BlobTablePageInfoObject],    ; table_pages
)

BlobTablePageInfoObject = [
    offset: uint,
    encoding: BlobEncodingObject,
    checksum: ChecksumObject,
]

BlobTablePageObject = [* BlobInfoObject]

BlobInfoObject = [
    offset: uint,
    encoding: BlobEncodingObject,
]
```

`BlobPoolSection.bytes` has length `SectionInfoObject.length - 8`. `blob_count` and all offsets and
sizes must fit in `u64`. An empty pool has no pages. Otherwise, the page count is
`1 + (blob_count - 1) / page_capacity`. Every page except the last contains `page_capacity` entries;
the final page contains the remaining entries.

For `blob_index`, the page index is `blob_index / page_capacity` and the index within that page is
`blob_index % page_capacity`. The logical blob table is the concatenation of the pages in directory
order.

Each page descriptor locates a stored page relative to `BlobPoolSection.bytes`. Decoding the page must
produce one deterministic CBOR `BlobTablePageObject`. Its checksum covers those decoded CBOR bytes and
must be verified unless the algorithm is `NONE`. Page filters must be self-contained and pages cannot
depend on other pages or blobs. Table pages are not blobs and cannot be referenced by `BlobRef`.
Resolving a blob requires decoding only its selected page.

Each `BlobInfoObject.offset` locates a stored blob relative to `BlobPoolSection.bytes`. All page and
blob ranges must fit in `bytes` and must not overlap. Writers may place them in any order.

#### Blob References

`BlobRef` identifies one complete, independently decoded blob:

```rust
struct BlobRef {
    /// A reference to the `BlobPool` section, encoded as a `SectionRef` CBOR item.
    blob_pool: SectionRef<BlobPoolSection>,

    /// The zero-based index of the blob in the referenced blob pool's `BlobTable`.
    blob_index: vuint,
}
```

`blob_index` must select an existing entry in the referenced pool's `BlobTable`. Decoding produces
uninterpreted bytes; the containing structure assigns their meaning.

`BlobSlice` identifies a non-empty range in the decoded representation of one blob:

```rust
struct BlobSlice {
    /// The blob containing the decoded byte range.
    blob: BlobRef,

    /// The byte offset of the range in the decoded blob.
    decoded_offset: vuint,

    /// The number of bytes in the range. This value must be greater than zero.
    decoded_length: vuint,
}
```

The selected range must be nonempty and contained in the decoded blob. Blob indices are local to one
pool.

Blob distribution is writer policy; readers must support any valid distribution. Writers may separate
structural data and file content for locality or incremental updates. Content dependencies must be
finite and acyclic.

## Conditions

Conditions are [Common Expression Language (CEL)](https://cel.dev/overview/cel-overview) expressions
used to select Java runtimes and environment-specific configuration.

A condition expression must evaluate to either `bool` or `int`:

- A `bool` result matches when `true`.
- If it evaluates to `int`:
  - for a root group, the value is the runtime priority;
  - for a subgroup, any value matches.

### Environment

The launcher exposes these variables:

```rust
// Information about a candidate Java installation.
let java: Java = ...;

// Information about the current host platform.
let platform: Platform = ...;
```

A candidate Java runtime is described by:

```rust
/// Information about a Java runtime environment.
struct Java {
    /// The version of the Java runtime.
    version: JavaVersion,

    /// The vendor of the Java runtime (e.g., `"Eclipse Adoptium"`, `"Oracle Corporation"`).
    vendor: String,

    /// The operating system for which this Java runtime was built.
    os: OperatingSystem,

    /// The CPU architecture for which this Java runtime was built (e.g., `"x86-64"`, `"aarch64"`).
    arch: String,
}

/// The parsed version of a Java runtime.
struct JavaVersion {
    /// The full, unparsed version string (e.g., `"21.0.3+9"`).
    full: String,

    /// The feature release number (the first version component, e.g., `21` for Java 21).
    feature: uint,

    /// The interim release number (the second version component).
    interim: uint,

    /// The update release number (the third version component).
    update: uint,

    /// The patch release number (the fourth version component).
    patch: uint,

    /// The optional pre-release identifier (e.g., `"ea"` for early-access builds).
    /// Empty string if not present.
    pre: String,

    /// The build number.
    build: uint,

    /// Optional additional build metadata. Empty string if not present.
    optional: String,
}
```

The current platform is described by:

```rust
/// Information about the current host platform.
struct Platform {
    /// The operating system of the host machine.
    os: OperatingSystem,

    /// The CPU of the host machine.
    cpu: CPU,
}

/// Information about an operating system.
struct OperatingSystem {
    /// The normalized name of the operating system (e.g., `"linux"`, `"windows"`, `"macos"`).
    name: String,

    /// The version of the operating system.
    version: OperatingSystemVersion,
}

/// The parsed version of an operating system.
struct OperatingSystemVersion {
    /// The full, unparsed version string.
    full: String,

    /// The major version number.
    major: uint,

    /// The minor version number.
    minor: uint,
}

/// Information about the host CPU.
struct CPU {
    /// The CPU architecture name (e.g., `"x86-64"`, `"aarch64"`, `"x86"`).
    arch: String,
}
```

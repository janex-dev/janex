# Janex File Format

Janex is a sectioned, multi-root container format. Its core stores shared content, metadata, and
verification information. Optional descriptor sections organize content into resource roots and
define uses such as Java applications, Java libraries, or executables for other runtimes.

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

These aliases denote bare CBOR values; the CDDL schema at each use site defines their contents:

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

All values follow RFC 8949 Section 4.2.1 Core Deterministic Encoding and their applicable schema.
Binary fields use `Sized<CborValue>` when they need an explicit byte boundary.

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

In CBOR:

```cddl
ChecksumObject = [
    algorithm: 0..65535,
    checksum: bstr,
]
```

The same algorithm-specific checksum lengths apply.

## Janex File Structure

The Janex container has the following layout:

```rust
struct JanexFile {
    /// The magic number identifying this as a Janex file.
    ///
    /// Always `0x0000_0058_454e_414a` ("JANEX\0\0\0").
    magic_number: u64, // 0x0000_0058_454e_414a ("JANEX\0\0\0")

    /// The ordinary sections of the Janex file.
    sections: [Section; ...],

    /// The file-level metadata at the end of the Janex file.
    file_metadata: FileMetadata,
}
```

The complete physical file may contain data outside `JanexFile`:

```text
[external header] [JanexFile] [external tail]
```

`FileMetadataObject` may constrain the external regions. They are not Janex sections and do not appear
in the section table.

### `FileMetadata`

```rust
struct FileMetadata {
    /// The magic number identifying the file metadata.
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
    
    /// The encoded byte length of this `FileMetadata`.
    metadata_length: u64,

    /// The total byte length of `JanexFile`.
    file_length: u64,
}
```

`metadata.value` is a `FileMetadataObject`:

```cddl
FileMetadataObject = {
    0: [* SectionInfoObject],          ; section_table
    ? 1: ExternalRegionObject,         ; external_header
    ? 2: ExternalRegionObject,         ; external_tail
    * uint => any,
}

NonemptyText = tstr .ne ""
```

`file_metadata` is the final structure within `JanexFile` and has no section type or ID. Its
`section_table` describes `sections` and may be empty.

#### Metadata Evolution and Extensions

Core maps use unsigned integer keys. Text-keyed metadata uses non-empty keys; `janex.` is reserved.
Third-party keys should use a reverse-domain prefix such as `org.example.`.

Unknown keys and section types may be ignored and must not change the meaning of known data. Semantic
rewrites must preserve unknown keys. Changes that alter core interpretation require a format version
that older readers reject.

Empty collections are valid where allowed by the schema. Optional empty maps should be omitted. `null`
is valid only where the schema defines it and is distinct from an omitted key.

#### `SectionInfoObject` Map

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

Other keys are defined by `section_type`.

#### Section References

```cddl
SectionRef = uint
```

A typed `SectionRef<T>` resolves `(T, id)` in the same file. The logical type `T` supplies the required
`section_type` and has no binary representation.

Currently supported section types:

```rust
#[repr(u64)]
enum SectionType {
    /// Arbitrary padding bytes; no section magic number is required.
    Padding = 0x0047_4e49_4444_4150, // "PADDING\0"

    Attributes = 0x2e53_4249_5254_5441, // "ATTRIBS."
    
    BlobPool = 0x4c4f_4f50_424f_4c42, // "BLOBPOOL"
    
    JavaApplicationDescriptor = 0x2e50_5041_4156_414a, // "JAVAAPP."

    JavaLibraryDescriptor = 0x2e42_494c_4156_414a, // "JAVALIB."

    StringPool = 0x004c_4f4f_5052_5453, // "STRPOOL\0"
}
```

The table describes consecutive sections. Use `Padding` for bytes between them.

Unknown sections may be skipped.

#### `ExternalRegionObject`

```cddl
ExternalRegionObject = [
    size: uint,
    checksum: ChecksumObject,
]
```

Omission leaves the region unconstrained. Otherwise, its size and checksum must match; zero size
requires `NONE` and absence, while nonzero size requires a non-`NONE` checksum.

The caller supplies `external_tail_length`; standalone Janex uses zero. It is not inferred from the
file. For a physical file of `physical_file_size` bytes:

```text
janex_end   = physical_file_size - external_tail_length
janex_start = janex_end - file_length
file_length = 8 + sum(FileMetadataObject.section_table[*].length) + metadata_length
```

All arithmetic is checked. The external header precedes `janex_start`, and the tail starts at
`janex_end`. The 24-byte footer before `janex_end` provides `metadata_length` and `file_length`;
`end_mark` must match, and `metadata_length` must equal the encoded `FileMetadata` length.

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

The range starts at `FileMetadata.magic_number` and ends immediately after
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

```cddl
AttributesObject = { * NonemptyText => any }
```

Attribute names follow the extension naming rules. Unknown attributes may be ignored. An empty map is
valid, though writers should omit the section in that case.

### Descriptor Sections

Descriptor sections give meaning to container contents but do not affect core decoding. A file may
contain any number, identified by section type and ID. A file without one is a plain container. Other
descriptor types may be defined independently. Resource roots are local to their descriptor, while
their contents may share blobs.

```rust
struct Descriptor<T> {
    /// One descriptor-specific deterministic CBOR object.
    object: Sized<CborMap>, // T

    /// The anonymous resource roots in local-index order.
    resource_roots: Vec<Sized<ResourceRoot>>,
}
```

`T` selects the schema of `object.value` and has no binary representation. A resource root is
identified by its zero-based index within `resource_roots`.

#### `JavaApplicationDescriptor` Section

```rust
struct JavaApplicationDescriptorSection {
    magic_number: u64, // 0x2e50_5041_4156_414a ("JAVAAPP.")
    descriptor: Descriptor<JavaLaunchConfigObject>,
}
```

##### `JavaLaunchConfigObject`

```cddl
JavaLaunchConfigObject = {
    ? 0: tstr,                                  ; condition
    ? 1: (tstr / null),                         ; main_class
    ? 2: (tstr / null),                         ; main_module
    ? 3: ([* JavaPathEntryObject] / null),       ; module_path
    ? 4: ([* JavaPathEntryObject] / null),       ; class_path
    ? 5: ([* JavaAgentObject] / null),           ; agents
    ? 6: ([* tstr] / null),                     ; jvm_options
    ? 7: [* JavaLaunchConfigObject],             ; overlays
    * uint => any,
}
```

An omitted `condition` is unconditional. Otherwise, it is described in
[Java Application Conditions](#java-application-conditions).

The launcher visits the root configuration and its `overlays` in depth-first pre-order. Each matching
object contributes as follows:

- missing keys make no contribution;
- `main_class` and `main_module` replace the current value, while `null` clears it;
- arrays append to `module_path`, `class_path`, `agents`, or `jvm_options`, while `null` clears that
  list; and
- `overlays` preserves array order and must not be `null`.

##### `JavaPathEntryObject`

```cddl
JavaPathEntryObject =
    {
        0: 0,                    ; embedded
        1: uint,                 ; resource_root_index
        * uint => any,
    }
  / {
        0: 1,                    ; remote
        1: tstr,                 ; purl
        2: ChecksumObject,       ; checksum
        * uint => any,
    }
```

Variant `0` selects a resource root from the containing descriptor. An out-of-range index is invalid.
Variant `1` identifies one concrete package version by canonical Package URL. It is downloaded without
transitive dependency resolution and verified by `checksum`. A `vers` qualifier is not allowed; Maven
artifacts use `pkg:maven` and may select a repository with `repository_url`.

##### `JavaAgentObject`

```cddl
JavaAgentObject = {
    0: JavaPathEntryObject,      ; reference
    1: tstr,                     ; option
    * uint => any,
}
```

An empty `option` means that no agent option is supplied.

#### `JavaLibraryDescriptor` Section

```rust
struct JavaLibraryDescriptorSection {
    magic_number: u64, // 0x2e42_494c_4156_414a ("JAVALIB.")
    descriptor: Descriptor<JavaLibraryDescriptorObject>,
}
```

```cddl
JavaLibraryDescriptorObject = { * uint => any }
```

Resource roots are searched in `descriptor.resource_roots` order.

#### Resource Roots

Each `ResourceRoot` contains one anonymous resource tree owned by its descriptor.

```rust
struct ResourceRoot {
    /// One deterministic CBOR `ResourceRootMetadataObject`.
    metadata: Sized<CborMap>, // ResourceRootMetadataObject

    /// The directories in path order.
    directories: Vec<ResourceDirectory>,
}
```

```cddl
ResourceRootMetadataObject = { * NonemptyText => any }
```

The enclosing `Sized<ResourceRoot>` allows a root to be skipped without parsing it. The metadata map
may be empty. Directories form a flat list; paths carry the hierarchy.

##### `ResourceDirectory`

```rust
struct ResourceDirectory {
    /// The directory path relative to the resource root.
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

`entries.transforms` must be empty.

##### `DirectoryEntry`

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

##### Resource Metadata

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
    /// A Java class-file transform using a shared `StringPool`.
    CLASSFILE = 1,
}
```

```cddl
ContentTransformPropertiesObject = { * uint => any }
ClassFileTransformPropertiesObject = {
    0: SectionRef,                   ; string_pool
    * uint => any,
}
```

The schema of `properties.value` is selected by `method`. `string_pool` selects the `StringPool`
section used by the class-file transform.

Transforms are stored in encoding order and reversed from last to first after blob decoding and slice
concatenation. Each result must match `input_size`; the final value must be a valid encoding of `T`.
An empty transform array means the source already encodes `T`. Unsupported methods are invalid.

`CLASSFILE` is valid only for regular-file `Content<[u8]>`. The transform arrays of
`ResourceDirectory.entries` and `StringPoolSection.data` must therefore be empty.

#### Java Class File Transform

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

The input must be a valid Java class file.

### `StringPool` Section

A `StringPool` supplies strings for class-file transforms. A file may contain any number of pools, and
multiple transforms may share one.

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

## Java Application Conditions

`JavaLaunchConfigObject.condition` is a
[Common Expression Language (CEL)](https://cel.dev/overview/cel-overview) expression used for runtime
and platform selection.

A condition expression must evaluate to either `bool` or `int`:

- A `bool` result matches when `true`.
- If it evaluates to `int`:
  - for the descriptor's root configuration, the value is the runtime priority;
  - for an overlay, any value matches.

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

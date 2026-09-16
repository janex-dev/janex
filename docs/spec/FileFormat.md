# Janex File Format

## Overview

Janex is a sectioned, multi-root container format. Its core stores shared content, metadata, and
verification information. Optional application sections describe launchable targets for supported
runtimes.

## Notation and Encoding

Janex uses little-endian encoding for fixed-width binary integer and floating-point fields. `vuint`
and values inside CBOR objects use the encodings defined in their respective sections below.

This document uses `u8`/`u16`/`u32`/`u64` to represent 8/16/32/64-bit unsigned integers,
uses `i8`/`i16`/`i32`/`i64` to represent 8/16/32/64-bit signed integers,
and uses `f32`/`f64` to represent 32/64-bit floating-point numbers.

Complex layouts use Rust-like pseudocode. `[T; count]` denotes `count` consecutive values of type `T`.

Janex uses [CBOR](https://www.rfc-editor.org/rfc/rfc8949.html) for extensible metadata. Schemas are
written in [CDDL](https://www.rfc-editor.org/rfc/rfc8610.html).

All CBOR values follow RFC 8949 Section 4.2.1 Core Deterministic Encoding and their applicable schema.

## Data Types

### Variable-Length Integers

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
Sized CBOR values have an empty-map encoding defined in [CBOR Values](#cbor-values).

### String

`String` is a UTF-8 byte vector:

```rust
type String = Vec<u8>;
```

```cddl
NonemptyText = tstr .ne ""
```

### Localized Text

`LocalizedText` is a display string, optionally with translations:

```cddl
Locale = tstr .ne ""

LocalizedText =
    tstr
  / { + Locale => tstr }
```

A bare `tstr` declares no language and is used for every locale. The
map form gives every string a [BCP 47](https://www.rfc-editor.org/rfc/rfc5646.html)
language tag such as `en`, `zh-Hans`, or `pt-BR`. There is no distinguished
default entry. Writers should use the shortest well-formed tag that
distinguishes a translation and should include `und` when they want an
explicit language-neutral fallback. The bare string is only for values that
do not declare a language.

A Host selects a string for a user-interface locale as follows. A bare
`tstr` is used for every locale. Otherwise the Host looks up the locale
in the map using BCP 47 language priority lookup
([RFC 4647](https://www.rfc-editor.org/rfc/rfc4647.html) Lookup). Tags are
compared case-insensitively. Keys that are not well-formed BCP 47 language tags
are ignored. If lookup matches no key, the Host uses `und` when present;
otherwise it uses the remaining well-formed key that is first in core
deterministic map order.

The map must contain at least one well-formed language tag. Well-formed tags must be unique under
case-insensitive comparison.

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

### CBOR Values

These aliases denote bare CBOR values; the CDDL schema at each use site defines their contents:

```rust
/// Exactly one deterministic CBOR data item permitted by the applicable schema.
type CborValue = ...;

type CborMap = CborValue;   // map
```

CBOR fields in binary structures must use `Sized<CborValue>` or `Sized<CborMap>`.
Values nested within CBOR use native CBOR encoding without a `Sized` wrapper.

For `Sized<CborValue>`, including `Sized<CborMap>`, `byte_size = 0` with no payload always represents
an empty map. A positive `byte_size` must contain exactly one deterministic CBOR data item other than
an empty map; `01 A0` is invalid. The decoded value must satisfy its applicable schema.
Empty arrays, empty strings, and `null` use their ordinary CBOR encodings with a positive byte length.
This special case does not apply to other `Sized<T>` types or values nested within CBOR.

### `Checksum`

```rust
struct ChecksumValue {
    /// The checksum algorithm.
    algorithm: ChecksumAlgorithm,

    /// The checksum bytes, occupying the remainder of the containing field.
    digest: [u8; ...],
}
```

Checksum algorithms:

```rust
#[repr(u8)]
enum ChecksumAlgorithm {
    XXH3_64 = 0x11,  // 8 bytes
    XXH3_128 = 0x12, // 16 bytes
    
    SHA256 = 0x21, // 32 bytes
    SHA512 = 0x22, // 64 bytes
    
    SM3 = 0x31, // 32 bytes
}
```

Algorithm ID `0` is reserved. `digest` has no length prefix; its length must match the algorithm.
Readers may skip unknown algorithms when the containing field provides a byte boundary. Required
validation accepts supported algorithms only.

Digests use each algorithm's standard byte representation, independently of Janex's integer byte
order. `XXH3_64` and `XXH3_128` use standard `XXH3_64bits` and `XXH3_128bits`, respectively,
with the default secret and seed `0`. Both use big-endian canonical representation; the 128-bit
digest stores the high 64 bits before the low 64 bits. Neither is cryptographically secure.

In CBOR:

```cddl
ChecksumObject = bstr .size (1..)
```

The byte string contains the exact encoding of one `ChecksumValue`: the first byte is the algorithm
ID and the remaining bytes are `digest`.

## File Structure

The Janex container has the following layout:

```rust
struct JanexFile {
    /// The magic number identifying this as a Janex file.
    ///
    /// Always `0x0000_0058_454e_414a` ("JANEX\0\0\0").
    magic_number: u64, // 0x0000_0058_454e_414a ("JANEX\0\0\0")

    /// The sections of the Janex file.
    sections: [Section; ...],

    /// The file-level metadata at the end of the Janex file.
    file_metadata: FileMetadata,
}
```

The complete physical file may contain data outside `JanexFile`:

```text
[external header] [JanexFile] [external tail]
```

`FileMetadataObject` records optional size and checksum constraints for the external regions.

### `FileMetadata`

```rust
struct FileMetadata {
    /// The magic number identifying the file metadata.
    magic_number: u64, // 0x4154_4144_4154_454d ("METADATA")

    /// The format major version. Must be `0`.
    major_version: u32,

    /// The format minor version. Must be `1`.
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

This document defines format version `0.1`. Readers must reject unsupported major or minor versions
before interpreting the metadata or sections.

`metadata.value` is a `FileMetadataObject`:

```cddl
FileMetadataObject = {
    0: [* SectionInfoObject],                    ; section_table
    ? 1: ExternalRegionObject,                   ; external_header
    ? 2: ExternalRegionObject,                   ; external_tail
    ? 3: NonemptyText,                          ; package_name
    ? 4: NonemptyText,                          ; package_version
    * NonemptyText => any,
    * uint => any,
}
```

`section_table` describes `JanexFile.sections` and may be empty.

Unsigned integer keys are container mechanics. Text keys are attributes. The `janex.` prefix is
reserved. Third-party keys should use a reverse-domain prefix such as `org.example.`. Attributes are
descriptive unless their definitions assign operational semantics. Unknown text keys may be ignored.
Writers should omit unused keys.

`package_name` is the stable name of an installable Janex package within its publisher or source
scope. `package_version` is that package's version string. Non-installable files may omit both.
Installation identifies a launch target by (`package_name`, `application_id`) within the same scope.

### Metadata Evolution and Extensions

Core maps use unsigned integer keys for container mechanics. Text-keyed metadata uses non-empty keys;
`janex.` is reserved. Third-party keys should use a reverse-domain prefix such as `org.example.`.

Unknown keys and section types may be ignored and must not change the meaning of known data. Semantic
rewrites must preserve unknown keys. Changes that alter core interpretation require a format version
that older readers reject.

Empty collections are valid where allowed by the schema. Optional empty maps should be omitted. `null`
is valid only where the schema defines it and is distinct from an omitted key.

### `SectionInfoObject` Map

```cddl
SectionInfoObject = {
    0: uint,                         ; section_type
    1: uint,                         ; id
    2: uint,                         ; length
    ? 3: ChecksumObject,             ; checksum
    ? 4: { * uint => any },          ; type_info
    * uint => any,
}
```

For sections with a magic number, `section_type` normally equals that number. `id` must be unique
within the file across all section types. IDs may be sparse and carry no ordering or semantic
meaning.

`length` is the exact encoded section length. When present, `checksum` covers those bytes and must be
verified.

`type_info` follows the schema selected by `section_type` and is omitted when the type defines no
additional information. Other root keys extend the common section information.

### Section References

```cddl
SectionRef = uint
```

`SectionRef` is a section `id`. The referenced row must exist. When the context requires a type `T`,
that row's `section_type` must be `T`.

Section types:

```rust
#[repr(u64)]
enum SectionType {
    /// Arbitrary padding bytes; no section magic number is required.
    Padding = 0x0047_4e49_4444_4150, // "PADDING\0"

    BlobPool = 0x4c4f_4f50_424f_4c42, // "BLOBPOOL"

    Application = 0x5050_4158_454e_414a, // "JANEXAPP"
}
```

The table describes consecutive sections. Use `Padding` for bytes between them.

Unknown sections may be skipped.

### `ExternalRegionObject`

```cddl
ExternalRegionObject = {
    0: uint,                         ; size
    ? 1: ChecksumObject,             ; checksum
    * uint => any,
}
```

Omission leaves the region unconstrained. Otherwise, `size` must match, and `checksum` must be verified
when present. A size of zero requires the region to be absent.

The caller may supply `external_tail_length`, or a reader may derive it using
[External Wrappers](#external-wrappers). Recorded region constraints are checked after locating
the metadata.

### `VerificationInfo` Structure

`VerificationInfo` is a tagged payload. `verification_input` ends after `verification_type` and before
the payload.

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

        /// One `ChecksumValue` covering `verification_input`.
        checksum: ChecksumValue,
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
`verification_type`. Verification uses these exact bytes.

The payload must consume exactly `payload_bytes`. A checksum payload must be one valid
`ChecksumValue`. OpenPGP and CMS payloads must be nonempty. Unknown verification types are invalid.

See [Verification](#verification) for signature profiles and authenticated content scope.

## External Wrappers

### Locating Janex

`external_tail_length` is supplied by the caller or derived from a recognized wrapper. With no
external tail, it is zero. For a physical file of `physical_file_size` bytes:

```text
janex_end   = physical_file_size - external_tail_length
janex_start = janex_end - file_length
file_length = 8 + sum(FileMetadataObject.section_table[*].length) + metadata_length
```

All arithmetic is checked. The external header precedes `janex_start`, and the tail starts at
`janex_end`. The 24-byte footer before `janex_end` provides `metadata_length` and `file_length`;
`end_mark` must match, and `metadata_length` must equal the encoded `FileMetadata` length.
The file magic, metadata, section boundaries, and recorded external-region constraints must also
validate. Automatic discovery must reject missing or ambiguous valid Janex boundaries.

### JAR Tail Wrapper

A complete single-disk JAR may immediately follow `JanexFile` and end at the physical end of the file:

```text
[external header] [JanexFile] [JAR, including its ZIP end records and comment]
```

ZIP offsets are relative to the start of this JAR, not the physical file. The unencrypted,
uncompressed central directory immediately precedes the ZIP end records. Ordinary ZIP uses an
end-of-central-directory record (EOCD); ZIP64 uses a ZIP64 EOCD, its locator, and the EOCD,
consecutively. Record layouts follow the
[ZIP specification](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT).

Readers locate an EOCD in the final 65,557 bytes whose 22-byte header and declared comment end
exactly at the physical end of the file. Without ZIP64, `directory_end` is the physical EOCD offset,
and the central-directory size and offset come from that record.

With ZIP64, the 20-byte locator immediately precedes the EOCD. Locate a ZIP64 EOCD whose signature
and declared record length place its end exactly at the locator; its total length is 12 plus its
size field and is not necessarily 56 bytes. `directory_end` is its physical offset, and its 64-bit
fields supply the central-directory size and offset.

```text
jar_start = directory_end - central_directory_size - central_directory_offset
janex_end = jar_start
external_tail_length = physical_file_size - jar_start
```

For ZIP64, `jar_start` plus the locator's ZIP64 EOCD offset must equal `directory_end`.
The ZIP directory and referenced records must validate within the JAR region. Each candidate must
also pass the Janex boundary checks above; a ZIP signature alone does not establish a boundary.

## Blob Pools

A `BlobPool` stores a numbered collection of logical byte blobs. Each blob has one table entry:
`Stored` locates encoded bytes, while `Extents` assembles decoded ranges of stored blobs.
Containing structures assign the blobs' logical types. A file may contain any number of pools.

### Section Layout

The pool's index and data are stored in two places:

| Location | Contents |
| --- | --- |
| `SectionInfoObject.type_info` in file metadata | The blob count and a directory of table pages. |
| `BlobPoolSection.bytes` | Encoded table pages and encoded bytes of stored blobs. |

```rust
struct BlobPoolSection {
    magic_number: u64, // 0x4c4f_4f50_424f_4c42 ("BLOBPOOL")

    /// Encoded table pages and stored-blob data addressed by offsets.
    bytes: [u8; ...],
}
```

`bytes` has length `SectionInfoObject.length - 8`. Page and stored-blob offsets are relative to
the start of `bytes`, immediately after the magic number. All ranges must fit in `bytes` and must
not overlap; writers may place them in any order. Blob counts, offsets, and sizes must fit in `u64`.

Table pages contain blob descriptions, not the blobs' contents. Page descriptors and `Stored`
entries use [Blob Encoding](#blob-encoding) to describe the length and filters of their encoded ranges.
An `Extents` blob has a table entry but no encoded data range of its own.

### Blob Table

The logical blob table is divided into independently decoded pages. Blob indices follow table order,
regardless of where pages or stored bytes appear in the section.

#### Page Directory

For a `BlobPool` section, `SectionInfoObject.type_info` is the following CBOR page directory:

```cddl
BlobPoolTypeInfoObject = {
    0: uint,                           ; blob_count
    1: 8..12,                          ; page_entry_shift
    2: [* BlobTablePageInfoObject],    ; table_pages
    * uint => any,
}

BlobTablePageInfoObject = [
    offset: uint,
    encoding: BlobEncodingObject,
    ? checksum: ChecksumObject,
]
```

`blob_count` includes every table entry. `entries_per_page` is `1 << page_entry_shift`.
An empty pool has no pages. Otherwise, the page count is
`1 + ((blob_count - 1) >> page_entry_shift)`. Every page except the last contains
`entries_per_page` entries; the final page contains the remaining entries. `table_pages` must contain
that many descriptors. Each page's stored and decoded byte lengths are determined by its encoding.

For `blob_index`, the page index is `blob_index >> page_entry_shift` and the index within that page is
`blob_index & (entries_per_page - 1)`. The logical blob table is the concatenation of the pages in
directory order.

Each page descriptor locates an encoded page relative to `BlobPoolSection.bytes`. Decoding it must
produce exactly one `BlobTablePage` and consume every decoded byte. When present, the checksum covers
the decoded page bytes and must be verified. Each page decodes independently using self-contained
filters.

#### Table Pages and Entries

Each decoded page contains consecutive binary table entries:

```rust
struct BlobTablePage {
    /// Entries in blob-index order.
    entries: [BlobTableEntry; page_entry_count],
}

#[repr(TaggedPayload<u8>)]
enum BlobTableEntry {
    /// An independently stored blob.
    Stored {
        entry_type: u8, // 0
        payload_bytes: vuint,

        /// The offset of the encoded bytes in `BlobPoolSection.bytes`.
        offset: vuint,

        encoding: BlobEncoding,
    },

    /// A blob assembled from decoded ranges of stored blobs.
    Extents {
        entry_type: u8, // 1
        payload_bytes: vuint,

        extents: Vec<BlobExtent>,
    },
}

struct BlobExtent {
    stored_blob_index: vuint,
    decoded_offset: vuint,
    decoded_length: vuint,
}
```

Each `BlobTableEntry` payload must consume exactly `payload_bytes`. Unknown entry types may be skipped
but cannot be resolved. A reader may use `payload_bytes` to skip preceding entries when locating one
entry within a decoded page. An `Extents` entry must contain at least one extent.

For a `Stored` entry, `offset` and `encoding.stored_size` locate its encoded bytes in
`BlobPoolSection.bytes`. Reversing `encoding.filters` produces the logical blob bytes.

An extents entry concatenates its decoded ranges in array order. Each `stored_blob_index` must select
a stored entry in the same pool. Each range must be nonempty and fit in that entry's decoded bytes.
Extents cannot refer to other extents entries.

### Blob Encoding

Stored blobs and blob-table pages use the same encoding description:

```rust
struct BlobEncoding {
    /// The number of encoded bytes.
    stored_size: vuint,

    /// Filters in encoding order.
    filters: Vec<BlobFilter>,
}

struct BlobFilter {
    /// The number of bytes supplied to this filter by the encoder.
    input_size: vuint,

    /// The filter method.
    method: BlobFilterId,

    /// Method-specific properties.
    properties: Sized<CborMap>, // BlobFilterPropertiesObject
}
```

CBOR layouts carry the binary `BlobEncoding` in a byte string:

```cddl
BlobEncodingObject = bstr .size (1..)
BlobFilterPropertiesObject = { * uint => any }
```

`BlobEncodingObject` must contain exactly one `BlobEncoding`.

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

The properties schema is selected by `method`. Unsupported filters are invalid.

#### Zstandard

`ZSTD` uses the following properties:

```cddl
ZstdPropertiesObject = {
    ? 0: BlobRefObject,                         ; dictionary
    * uint => any,
}
```

The encoded bytes contain one or more Zstandard frames, optionally interspersed with skippable frames,
as defined by [RFC 8878](https://www.rfc-editor.org/rfc/rfc8878.html). Decoding must consume every
encoded byte and concatenate frame outputs in order, producing exactly `input_size` bytes.

Omitting `dictionary` means no dictionary is used. Otherwise, the referenced blob's decoded bytes
provide the formatted or raw-content dictionary defined by RFC 8878 Section 5 for every frame.
Multiple blobs may share a dictionary. The reference selects the dictionary; `Dictionary_ID` is not
used to locate it. A nonzero frame `Dictionary_ID` must match the selected dictionary's ID.

Blob-table page filters must omit `dictionary`. Filters used to decode a dictionary blob, including
any stored sources of its extents, must also omit `dictionary`.
Whether to use a dictionary and how to train it are writer policy.

### Blob References

`BlobRef` identifies one complete logical blob. Binary layouts use:

```rust
struct BlobRef {
    /// The ID of a `BlobPool` section.
    blob_pool: vuint, // SectionRef

    /// The zero-based index of the blob in the referenced pool's `BlobTable`.
    blob_index: vuint,
}
```

CBOR layouts use:

```cddl
BlobRefObject = [
    blob_pool: SectionRef,           ; must identify a BlobPool section
    blob_index: uint                 ; must select an existing blob
]
```

A `BlobRef` and a `BlobRefObject` identify the same blob. `blob_pool` must be a `SectionRef` whose
section is a `BlobPool`. `blob_index` must select an existing entry in that pool's `BlobTable`.
Resolving a stored entry decodes its stored bytes. Resolving an extents entry concatenates its ranges.
The result is uninterpreted bytes whose meaning is assigned by the containing structure.

Locating a stored blob requires only its selected table page; resolving an extents blob may also
require the pages containing its stored sources.
Decoding a blob may additionally require resolving its dictionary blobs.

Whether a blob is stored directly or assembled from extents is writer policy. Readers must support
both forms. Writers may store frequently accessed structural blobs directly for better locality.

## Content and Transforms

`Content<T>` stores an encoded value inline or in one blob. Reversing its transforms produces a
logical value of type `T`:

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

    /// Uses the resolved bytes of one blob.
    Blob {
        source_type: u8, // 1
        blob: BlobRef,
    },
}
```

`Inline` supplies `bytes`, while `Blob` supplies one complete logical blob. Empty content uses an
empty `Inline` value.

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
    /// A Java class-file transform using a shared `DataPool`.
    CLASSFILE = 1,
}
```

```cddl
ContentTransformPropertiesObject = { * uint => any }
```

The schema of `properties.value` is selected by `method`.

Transforms are stored in encoding order and reversed from last to first after resolving the source.
Each result must match `input_size`; the final value must be a valid encoding of `T`. An empty
transform array means the source already encodes `T`. Unsupported methods are invalid.

`CLASSFILE` is valid only for regular-file `Content<[u8]>`. The transform array of
`ResourceDirectory.entries` must therefore be empty.

### Java Class File Transform

The class-file transform moves selected constant-pool bytes into a shared `DataPool`.

```cddl
ClassFileTransformPropertiesObject = {
    ? 0: BlobRefObject,                         ; data_pool
    * uint => any,
}
```

`data_pool` selects the pool when present; otherwise, the containing `ResourceRoot.data_pool`
is used. An invalid explicit pool is an error, with no fallback.

The input must be a valid Java class file, and reversing the transform must reproduce its exact
bytes. The transform changes only the magic bytes and selected `CONSTANT_Utf8` entries; constant-pool
counts, slots, reference indices, and all other bytes remain unchanged. Original class-file fields
retain their big-endian encoding; external data indices use `DataPoolIndex` (`vuint`).

The transformed magic bytes are `CA FE CA 70`; decoding restores `CA FE BA BE`.
The following entries may replace `CONSTANT_Utf8` entries:

```rust
struct CONSTANT_External_String {
    tag: u8, // 0xFF

    /// The index of the complete Modified UTF-8 bytes in the selected data pool.
    data_pool_index: DataPoolIndex,
}

struct CONSTANT_External_String_Class {
    tag: u8, // 0xFE

    /// The index of the slash-separated package name, or 0 for the unnamed package.
    package_name_index: DataPoolIndex,

    /// The index of the nonempty class name without the package prefix.
    class_name_index: DataPoolIndex,
}

struct CONSTANT_External_String_Template {
    tag: u8, // 0xFD

    /// The index of a byte template in the selected data pool.
    template_index: DataPoolIndex,
}
```

`CONSTANT_External_String` copies the selected bytes. `CONSTANT_External_String_Class` concatenates
`package + "/" + class` as bytes when the package is nonempty, or just `class` otherwise. Array class names
may use `CONSTANT_External_String`, `CONSTANT_External_String_Template`, or remain unchanged.

`CONSTANT_External_String_Template` expands its template from start to end. Nonzero bytes are copied
literally; `00` is followed by a package index and a nonempty class-name index, both `DataPoolIndex`,
expanded as for `CONSTANT_External_String_Class`. Referenced bytes are copied without recursive
expansion. The template ends at the end of its pool entry. Descriptors and generic signatures can
share class-name components this way while preserving their punctuation and other literal bytes.

All three entries decode to `CONSTANT_Utf8` (tag `0x01`), followed by a big-endian `u16` byte length and
the restored bytes without transcoding. The result must be valid Modified UTF-8 and fit in 65,535 bytes.

Decoders check transform framing, pool references, and byte lengths. Validation of class-file
internals, including Modified UTF-8, may be deferred to the JVM when the class is loaded.

## Resource Roots

A `ResourceRoot` is a resource tree built from ordered layers. Using the consumer's
[evaluation context](#conditionobject), matching layers are merged from first to last; later values
override earlier ones at the same path. No application section or Java runtime is required.

A blob reference naming a resource root must resolve to exactly one `ResourceRoot` and consume every
resolved byte. Consumers may share a root; different contexts may produce different merged trees.

```rust
struct ResourceRoot {
    /// The path data pool and default pool for `CLASSFILE` transforms.
    data_pool: BlobRef,

    /// One deterministic CBOR `ResourceRootMetadataObject`.
    metadata: Sized<CborMap>, // ResourceRootMetadataObject

    /// Layers in application order.
    layers: Vec<ResourceLayer>,
}

struct ResourceLayer {
    /// One deterministic CBOR `ConditionObject`. An empty map is unconditional.
    condition: Sized<CborMap>, // ConditionObject

    /// The directories in this layer, in path order.
    directories: Vec<ResourceDirectory>,
}
```

```cddl
ResourceRootMetadataObject = { * NonemptyText => any }
```

The metadata map may be empty. Readers must resolve `data_pool` before using references to it.
Multiple resource roots may name the same data-pool blob.

The optional text attribute `janex.java.jar_name` preserves the JAR filename used when materializing
this root as a Java path entry, including filename-derived automatic module names. It must be a
single filename ending in `.jar`, without `/`, `\`, or NUL. The default is `resources.jar`.
Consumers materialize different roots in separate directories to avoid filename collisions.

### Data Pools

A data-pool blob must resolve to exactly one `DataPoolData` and consume every resolved byte.

```rust
/// A zero-based index into `DataPoolData.entries`.
type DataPoolIndex = vuint;

struct DataPoolData {
    /// Distinct byte sequences in pool-index order.
    entries: Vec<Sized<[u8]>>,
}
```

The pool contains unique byte sequences, with the empty sequence at index `0`.
A `DataPoolIndex` must select an existing element. Referencing fields define the interpretation
and validity constraints of the selected bytes.

### `ResourceDirectory`

```rust
struct ResourceDirectory {
    /// The directory path relative to the resource root, as an index into the root's data pool.
    path: DataPoolIndex,

    /// One deterministic CBOR resource-metadata map.
    metadata: Sized<CborMap>, // ResourceMetadataObject

    /// The number of direct entries in this directory.
    entries_count: vuint,

    /// The direct entries in name order.
    entries: Content<[DirectoryEntry; entries_count]>,
}
```

`path` index `0` identifies the root directory. Other resolved paths are UTF-8, `/`-separated, and
must not start or end with `/` or contain empty, `.` or `..` components.
Directory paths are unique within a layer and sorted by the UTF-8 bytes of the resolved strings.
Parent directories may be implicit. An explicit record with no entries preserves an empty directory
or its metadata.

When a later matching layer contains the same directory path, its metadata replaces the earlier
metadata and its entries are merged into the directory. A later file or symbolic link with the same
name replaces the earlier one. A tombstone removes an earlier file or symbolic link with that name.
A tombstone for a name that is not present is ignored.

For each matching layer, apply its tombstones to the existing tree before merging its other records.
After each layer, no file or symbolic link may occupy the path of an explicit or implicit directory;
such a conflict invalidates the resource root. Replacing a file or symbolic link with a directory
requires a tombstone in the same or an earlier matching layer. Tombstones do not remove directories,
so replacing a directory with a file or symbolic link is invalid.

`entries.transforms` must be empty.

### `DirectoryEntry`

```rust
enum DirectoryEntry {
    /// Represents a regular file.
    File {
        /// The resource type tag for this variant.
        ///
        /// Always `0x00534552` ("RES\0").
        resource_type: u32, // 0x00534552 ("RES\0")

        /// The file name within the directory.
        name: NonemptyStringValue,

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
        name: NonemptyStringValue,

        /// The target path relative to the symbolic link's containing directory.
        target: NonemptyStringValue,

        /// One deterministic CBOR resource-metadata map.
        metadata: Sized<CborMap>, // ResourceMetadataObject
    },

    /// Removes an earlier file or symbolic link with this name.
    Tombstone {
        /// The resource type tag for this variant.
        ///
        /// Always `0x424d4f54` ("TOMB").
        resource_type: u32, // 0x424d4f54 ("TOMB")

        /// The name to remove within the directory.
        name: NonemptyStringValue,
    },
}
```

Entry names and symbolic-link targets use `NonemptyStringValue`:

| Encoding | Meaning |
| --- | --- |
| A positive `vuint` | An existing index in the root's data pool; no additional bytes. |
| `0`, then a nonempty `String` | An inline value. |
| `0`, then an empty `String`, then `Vec<DataPoolIndex>` | A concatenation of root data-pool entries. |

Referenced entries must be valid UTF-8. The concatenation index array must contain at least two
entries. Their bytes are concatenated in array order without separators, and the result must be nonempty.
The empty `String` is a marker, not the resolved value.

For example, `05` references pool entry `5`; `00 03 66 6F 6F` encodes inline `"foo"`.
If pool entries `5` and `6` contain `"Object"` and `".class"`, `00 00 02 05 06` encodes `"Object.class"`.
All forms may be mixed; writers choose the representation. Name and path constraints, sorting, and
uniqueness checks apply to the resolved UTF-8 bytes.

Entry names are nonempty UTF-8 strings without `/` and must not be `.` or `..`. They are unique within
their directory, including tombstones, and sorted by the UTF-8 bytes of the resolved strings. A full
resource path is the resolved entry name for the root directory, or `directory_path + "/" +
entry_name` otherwise, using the resolved strings. Directory records and file or symbolic-link
entries must not produce conflicting paths.

Symbolic-link targets are nonempty relative `/`-separated paths with no leading or trailing `/`
and no empty components. They are resolved component by component in the merged resource tree,
relative to the link's containing directory. `.` denotes the current directory and `..` its parent.
Resolution must not escape the resource root, including when following other symbolic links;
an attempt to do so is an error.

### Resource Metadata

```cddl
ResourceMetadataObject = {
    ? 0: ChecksumObject,             ; checksum
    ? 1: tstr,                       ; comment
    ? 2: UnixNanosecondsObject,      ; creation_time
    ? 3: UnixNanosecondsObject,      ; modification_time
    ? 4: UnixNanosecondsObject,      ; access_time
    ? 5: 0..4095,                    ; posix_permissions
    * uint => any,
}

UnixNanosecondsObject =
    int
  / #6.2(bstr .size (9..16))
  / #6.3(bstr .size (9..16))
```

An empty map represents no metadata.

`checksum` is valid only for regular files and covers the logical content after transforms are
reversed.

`posix_permissions` is valid only for regular files and directories, not symbolic links. It contains
the nine POSIX read/write/execute bits and the setuid, setgid, and sticky bits (octal `0000..7777`),
excluding file-type bits. Omission leaves permissions unspecified; `0` means no permission bits are set.
Consumers apply permissions according to platform capabilities and local policy.

Time values are signed `i128` POSIX timestamps in nanoseconds. Values in CBOR's basic integer range use
major type `0` or `1`. Larger values use tag `2` or `3` with a minimal 9-to-16-byte big-endian
magnitude; tag `3` encodes `-1 - value`.

## Conditions

Launch configurations and resource layers use conditions to select data for a supplied environment.

### `ConditionObject`

```cddl
NameSelector = NonemptyText / [+ NonemptyText]

ConditionObject = {
    ? 1: NameSelector,                          ; os
    ? 2: NameSelector,                          ; arch
    ? 4: NameSelector,                          ; invocation
    ? 5: RuntimeConditionObject,                ; runtime
    * uint => any,
}
```

Keys `0` and `3` are reserved and must not occur. All present constraints must match; omitted keys
impose no constraint. An empty map is unconditional.

The consumer supplies the execution OS and CPU architecture, an optional invocation channel, and an
optional runtime with its type and properties. With a selected runtime, `arch` describes that runtime,
not the launcher process or the system's native architecture. Conditions do not select a runtime.

`os` uses `linux`, `windows`, or `macos`; `arch` uses `x86`, `x86-64`, or `aarch64`.
Other names match by exact equality.

`invocation` uses `run` for `janex run`, `open` for file-association or double-click launch, and
`command` for an installed command. It identifies the launch channel, not the windowing mode.
Unknown tokens or a missing context channel do not match.

A `NameSelector` matches one name or any name in its array. Comparisons are case-sensitive.

### `RuntimeConditionObject`

```cddl
RuntimeConditionObject = {
    0: NonemptyText,                            ; runtime_type
    1: { * uint => any },                       ; requirements
    * uint => any,
}
```

`runtime_type` selects the schema and matching rules of `requirements`. Names are case-sensitive;
`janex.` is reserved, and third-party types use reverse-domain names.

A missing context runtime or a different type does not match. Otherwise, all requirements must
match. The required `requirements` map may be empty to accept any runtime of that type.

Unknown types may be preserved as opaque data and need not be understood to reject a type mismatch.
If the type matches but the requirements cannot be interpreted, report an unsupported condition.

Readers must validate condition structure and requirements for supported types, even in unmatched
conditions. Invalid conditions invalidate the containing application descriptor or resource root.

See [Java Runtime Requirements](#janexjava-runtime-requirements) for `janex.java`.

## Applications

A file may contain any number of `Application` sections. Each section is one independently launchable
target. Multiple targets may share blobs. The section body after the magic number is one
`Sized<CborMap>` containing a deterministic CBOR `ApplicationObject`. A file with no application
section has no launch target.

Separate commands with independent launch configurations use separate application sections.
Subcommands interpreted by one program are ordinary application arguments.

For an `Application` section, `SectionInfoObject.type_info` is:

```cddl
ApplicationTypeInfoObject = {
    0: NonemptyText,                            ; application_id
    1: NonemptyText,                            ; application_type
    * uint => any,
}
```

```rust
struct ApplicationSection {
    magic_number: u64, // 0x5050_4158_454e_414a ("JANEXAPP")

    /// One deterministic CBOR `ApplicationObject`.
    application: Sized<CborMap>, // ApplicationObject
}
```

The complete `application` field, including its length prefix, must occupy exactly the remainder of
the section.

### `ApplicationObject`

```cddl
ApplicationObject = {
    0: ApplicationDescriptorObject,             ; descriptor
    ? 1: LocalizedText,                         ; name
    ? 2: NonemptyText,                          ; version
    ? 3: LocalizedText,                         ; comment
    ? 4: ApplicationIntegrationObject,          ; integration
    ? 5: ApplicationLaunchMode,                 ; launch_mode
    * uint => any,
}

ApplicationDescriptorObject = { * uint => any }

ApplicationLaunchMode =
    0                                           ; console
  / 1                                           ; windowed
```

`application_id` identifies the launch target within the file and must be unique among its
application sections. A caller may select an application by this ID. Without an explicit selection,
the only application section is selected; multiple application sections are ambiguous. The same
logical target should retain its `application_id` across versions of the same package.
`SectionInfoObject.id` locates the section and is not part of the installed target identity.
`application_type` selects the schema and semantics of `descriptor`; this document defines
`janex.java`, and reserves the `janex.` prefix. Third-party types use a reverse-domain name.
`descriptor` must follow the selected schema. `name` is a display name and may be localized.
When present, its bare string or every translation must be nonempty.
`version` is the application's own version string. `comment` is a short description and may be
localized; its text may be empty. `launch_mode` defaults to `console`. `windowed` suppresses console
window creation on platforms that distinguish the two modes.

`name` and `comment` are for presentation. They are not command names and are not part of the
installed target identity. When presenting this application, including a desktop launcher, the Host
selects `name` and `comment` for the current user-interface locale. If `name` is omitted, the
desktop title is `command` if present, otherwise `application_id`.

Unsupported application types may be displayed and preserved but cannot be launched.

### `ApplicationIntegrationObject`

```cddl
ApplicationIntegrationObject = {
    ? 0: NonemptyText,                          ; command
    ? 1: bool,                                  ; desktop_launcher
    ? 2: [+ ApplicationIconObject],             ; icons
    * uint => any,
}
```

These fields request installation integration. The Host applies them according to local policy.

`command` is the name of a command to install for this application. It must be a single file name
valid on the target platform, not a path. It is never localized. The Host typically makes it
available on `PATH`. An omitted value makes no request.

`desktop_launcher` asks the installer to create a Start Menu entry on Windows, a `.desktop` entry on
Linux, or an equivalent launcher on other platforms. An omitted value or `false` makes no request.
The launcher's visible title is the localized `name` as specified above, not `command`.

`icons` supplies image blobs for those launchers. The Host selects a suitable image for the
platform. An empty array should be omitted.

### `ApplicationIconObject`

```cddl
ApplicationIconObject = {
    0: NonemptyText,                            ; media_type
    1: BlobRefObject,                           ; image
    * uint => any,
}
```

`media_type` is a non-empty IANA media type such as `image/png`, `image/jpeg`,
`image/vnd.microsoft.icon`, or `image/icns`. Unknown types may be ignored. `image` names a blob whose
resolved bytes are the image.

## Java Applications

### `janex.java` Runtime Requirements

For `runtime_type` `janex.java`, `requirements` is:

```cddl
JavaRuntimeRequirementsObject = {
    ? 0: NonemptyText,                          ; version
    ? 1: NonemptyText,                          ; vendor
    * uint => any,
}
```

`version` is a `jep322` [VERS](#version-ranges), matched against the runtime's Java version with
Java 8 alias expansion. An invalid VERS invalidates the condition. `vendor` matches the runtime's
vendor string exactly, without normalization.

### `janex.java` Application Descriptor

For `application_type` `janex.java`, `descriptor` is:

```cddl
JavaApplicationDescriptorObject = {
    0: JavaLaunchConfigObject,                  ; launch
    * uint => any,
}
```

#### `JavaLaunchConfigObject`

```cddl
JavaLaunchConfigObject = {
    ? 0: ConditionObject,                       ; condition
    ? 1: (JavaEntryPointObject / null),          ; entry_point
    ? 2: ([* JavaPathEntryObject] / null),       ; module_path
    ? 3: ([* JavaPathEntryObject] / null),       ; class_path
    ? 4: ([* JavaAgentObject] / null),           ; agents
    ? 5: ([* tstr] / null),                     ; jvm_options
    ? 6: [* JavaLaunchConfigObject],             ; overlays
    ? 7: ([* tstr] / null),                     ; arguments
    * uint => any,
}
```

An omitted `condition` is unconditional. See [Conditions](#conditions) for the condition model.

The launcher visits the root configuration and its `overlays` in depth-first pre-order. A nonmatching
object and its entire subtree are skipped. Each matching object contributes as follows:

- missing keys make no contribution;
- `entry_point` replaces the current value, while `null` clears it;
- arrays append to `module_path`, `class_path`, `agents`, `jvm_options`, or `arguments`, while `null`
  clears that list; an empty array makes no contribution; and
- `overlays` preserves array order and must not be `null`.

The resulting configuration must contain an `entry_point`.

`arguments` supplies preset program arguments, followed at launch by user-supplied program arguments.
Each string is one complete argument and may be empty. The launcher preserves argument boundaries
and performs no shell splitting, variable expansion, or wildcard expansion. The program determines
how repeated or conflicting arguments are handled.

#### `JavaEntryPointObject`

```cddl
JavaEntryPointObject = {
    ? 0: NonemptyText,                          ; main_class
    ? 1: NonemptyText,                          ; main_module
    * uint => any,
}
```

At least one of `main_class` and `main_module` must be present. Without `main_module`, the launcher
launches `main_class` from the class path. With `main_module`, `main_class` selects a class in that
module; when omitted, the module supplies its main class.

#### Java Runtime Selection

For each candidate, the launcher evaluates the root configuration and its overlays using the
current host, invocation channel, and that candidate's `janex.java` version and vendor.

The launcher considers each candidate against the root condition. A candidate that does not match
is discarded. For each remaining candidate, the launcher walks the root configuration and its
`overlays` in depth-first pre-order and applies each overlay whose condition matches. A candidate is
discarded if any module requirement cannot be satisfied by the runtime, another module-path entry,
or an allowed provider. The launcher then selects a remaining candidate using the implementation's
runtime selection order, which may prefer runtimes that provide required modules directly.

#### `JavaPathEntryObject`

```cddl
JavaPathEntryObject =
    {
        0: 0,                    ; local
        1: BlobRefObject,        ; resource_root
        * uint => any,
    }
  / {
        0: 1,                    ; external
        1: NonemptyText,         ; uri
        ? 2: ChecksumObject,     ; checksum
        * uint => any,
    }
```

Variant `0` selects a blob in this file whose resolved representation is one `ResourceRoot`, as
defined in [Resource Roots](#resource-roots). The blob is named by a `BlobRefObject`, defined in
[Blob References](#blob-references). Variant `1` contains a URI, such as a Package URL or an HTTP(S)
URL. A `pkg:` URI must be a canonical Package URL. When present, `checksum` verifies the resolved
content.

Local resource roots use the selected runtime and the same host and invocation channel. Each merged
tree forms one classpath or module-path entry. Multiple entries may share a resource-root blob.

In a `JavaPathEntryObject`, a `janex` PURL must use the `java-module` requirement kind and is allowed
only in `module_path`. Resolving it produces zero or more physical path entries at the same position.
No path entry is needed when the selected runtime already provides the module. The selected runtime
and resulting module path must together provide the named module and, when present in the PURL, its
exact version.

#### `JavaAgentObject`

```cddl
JavaAgentObject = {
    0: JavaPathEntryObject,      ; reference
    1: tstr,                     ; option
    * uint => any,
}
```

An empty `option` means that no agent option is supplied.

### Multi-Release JAR Mapping

When importing a Multi-Release JAR, the base tree becomes an unconditional layer. Each
`META-INF/versions/N/` tree becomes a layer constrained only by `janex.java` version
`vers:jep322/>=N`, in increasing `N` order.

For Java 21, the condition in CBOR diagnostic notation is:

```cbor-diag
{5: {0: "janex.java", 1: {0: "vers:jep322/>=21"}}}
```

A resource root can be exported back to a Multi-Release JAR when its first layer is unconditional
and every remaining layer has only one of these Java version constraints in increasing `N`.

## Verification

### Verification Policy

Authentication requires OpenPGP or CMS and successful validation under the declared type. These
variants use caller-provided signer, trust, algorithm, time, and revocation policies.

OpenPGP and CMS implementations must support SHA-256 for content digests. Janex signatures must not
use MD5, SHA-1, or RIPEMD-160. Other algorithms are subject to caller policy.

### OpenPGP Profile

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

### CMS Profile

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

Each required signer must pass digest, signature, identity, and caller-policy validation. Caller policy
establishes trust for embedded certificates and revocation data and selects the required signers. Only
signed attributes contribute to primary signature validation.

### Authenticated Content Scope

Full-container authentication includes both `external_header` and `external_tail` and records a secure
checksum for every section and every nonempty external region. Readers must verify all of them. Secure
checksum algorithms are `SHA256`, `SHA512`, and `SM3`.

The signature authenticates `verification_input`; the recorded secure checksums authenticate section
and external-region bytes.

## Package URLs

Janex uses canonical [Package URLs (ECMA-427)](https://ecma-tc54.github.io/ECMA-427/) to name packages and
resolvable requirements.

### The `janex` Type

The `janex` type identifies a virtual package resolved by the Host:

```text
pkg:janex/<requirement-kind>/<name>@<version>
```

The namespace is one case-sensitive requirement-kind segment. The name is case-sensitive. The
optional version identifies one exact version. Qualifiers and subpaths are not allowed, and the type
has no package repository. A selected provider keeps its own Package URL.

The `java-module` requirement kind names a JPMS module. Its name is the exact module name, and its
version is the exact module descriptor version. A Host may satisfy it with the selected Java runtime
or another provider. For example, `pkg:janex/java-module/javafx.controls` requires the
`javafx.controls` module.

## Version Ranges

Janex uses the [VErsion Range Specifier (VERS)](https://www.packageurl.org/docs/vers/specification.html)
to describe version constraints. A VERS is a URI of the form:

```text
vers:<type>/<constraint>[|<constraint>]...
```

`<type>` selects the version notation and comparison rules. Each constraint is either `*`, a bare
version, or a comparator (`=`, `!=`, `<`, `<=`, `>`, `>=`) immediately followed by a version. The
pipe separates constraints. It is not a boolean operator. Constraints are signposts on a version
timeline: they are sorted in version order and define intervals as specified by VERS.

A VERS in a Janex file must be in the canonical form required by VERS. Whitespace is not permitted.
An invalid or non-canonical VERS is invalid.

This document defines the `jep322` type for Java SE platform versions. Later uses, such as unresolved
library requirements, may use other registered types such as `maven`.

### The `jep322` Type

The `jep322` type compares Java SE 8 and later. Java 9 and later use version strings as defined by
[JEP 223](https://openjdk.org/jeps/223) and [JEP 322](https://openjdk.org/jeps/322) and parsed by
`java.lang.Runtime.Version`. Java 8 uses the aliases below; they are rewritten to the same
four-tuple form before comparison.

A canonical version has a non-empty numeric sequence `$FEATURE.$INTERIM.$UPDATE.$PATCH` and optional
later numeric elements, optionally followed by a pre-release identifier, a build number, and
additional build information:

```text
$VNUM(-$PRE)?(\+$BUILD)?(-$OPT)?
$VNUM-$PRE(-$OPT)?
$VNUM(\+-$OPT)?
```

`$VNUM` is a period-separated sequence of decimal integers without leading zeros. Trailing zero
elements are omitted from the written `$VNUM`. `$PRE` is a pre-release identifier such as `ea`.
`$BUILD` is a decimal build number. `$OPT` is additional build information. Examples: `8`, `8.0.402`,
`17`, `17.0.10`, `21.0.2+13`, `21-ea+11`.

Writers should use this canonical form in a VERS. A bare version requires equality under the
comparison rules below: `8` does not match `8.0.402`, which denotes update 402 of Java 8.
Use `vers:jep322/>=8|<9` for versions at least `8` and less than `9`.

#### Java 8 Aliases

The following Java 8 strings are aliases. They are not canonical JEP 322 versions. Readers rewrite
each alias to a canonical version, then compare. `$N` and `$BUILD` are decimal integers without
leading zeros, except that `$N` may be `0`:

```text
1.8.0               →  8
1.8.0_$N            →  8.0.$N
1.8.0_$N-b$BUILD    →  8.0.$N
8u$N                →  8.0.$N
```

An optional `-$PRE` may follow `1.8.0` or `1.8.0_$N` and is kept on the canonical form, so
`1.8.0-ea` becomes `8-ea` and `1.8.0_402-ea` becomes `8.0.402-ea`. Other `1.x` forms, including
`1.8`, `1.8.0_402-b06-extra`, `1.7.0_80`, and `1.9.0`, are invalid.

Alias expansion applies to versions in a `jep322` VERS and to the version string reported by a
candidate runtime. After expansion, VERS uniqueness and ordering use the canonical versions.
`1.8.0_402` and `8.0.402` in the same VERS are therefore the same version and make the VERS invalid.

Comparison uses the numeric version elements and `$PRE` only. `$BUILD` and `$OPT` are ignored, so
`21.0.2` and `21.0.2+13` compare equal. Missing numeric elements are treated as zero, so `21`,
`21.0`, and `21.0.0` compare equal. Numeric elements are compared from left to right.

When the numeric elements are equal:

- a version with no `$PRE` is greater than a version with a `$PRE`;
- two `$PRE` values that contain only digits are compared numerically;
- otherwise `$PRE` values are compared lexicographically by ASCII code point;
- a numeric `$PRE` is less than a non-numeric `$PRE`.

Therefore `21-ea` is less than `21`, and `>=21` does not contain `21-ea`. `>=21-ea` contains both
`21-ea` and `21`.

A candidate version satisfies a `jep322` VERS when it lies in one of the intervals defined by the
constraint timeline, using this comparison. The VERS `type` in
`JavaRuntimeRequirementsObject.version` must be `jep322`.

```text
vers:jep322/>=17.0.10|<18|>=21.0.2|<22
```

This range contains Java 17 starting at 17.0.10, and Java 21 starting at 21.0.2. It does not contain
Java 18, Java 22, or `17-ea`.

```text
vers:jep322/>=8.0.402|<9
```

This range contains Java 8 starting at update 402. A candidate that reports `1.8.0_402` matches.

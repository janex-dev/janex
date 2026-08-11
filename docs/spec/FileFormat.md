# Janex File Format

The Janex format is a modern executable packaging format for Java programs.

The Janex format is designed as a better alternative to Shadow JAR (Fat JAR) and launch4j,
aiming to be the optimal solution for single-file packaging and distribution of Java programs.
Its key features include:

- **Module system support**: Unlike Shadow JAR (Fat JAR), Janex properly supports the Java module system.
  Resources from different JARs are isolated under different resource groups instead of being mixed together.
- **Zstandard compression**: Janex uses [Zstandard](https://github.com/facebook/zstd) for compression,
  which provides faster decompression and smaller file sizes compared to the deflate compression used by JAR.
  Additionally, Janex shares strings from the constant pool of Java class files across resource groups,
  further reducing the overall file size.
- **Remote dependencies**: Janex files can declare dependencies on packages from external sources (such as Maven
  repositories) using Package URLs (PURLs). These dependencies are not bundled in the Janex file but are resolved
  and downloaded on demand before the program starts.
- **Automatic Java runtime selection**: Users can specify conditions (such as a minimum Java version, operating system,
  or CPU architecture), and the Janex Launcher will find a suitable installed Java runtime to run the program.
- **Embedded JVM options**: Janex files can contain JVM options (such as `--add-exports`, `--enable-native-access`,
  `-D`, etc.) that are passed to the JVM at runtime.
- **Conditional configuration**: Janex supports dynamic selection of Java runtimes, classpath entries,
  module path entries, and JVM arguments based on runtime environment conditions using
  [Common Expression Language (CEL)](https://cel.dev/overview/cel-overview) expressions.
- **Prepended data support**: The Janex format allows arbitrary custom data (such as PE/ELF executables or shell
  scripts) to be prepended to the file, enabling shebang support for direct execution on Unix-like systems and
  self-contained distribution with an embedded launcher.

When launching a Janex program, the **Janex Launcher** reads the launcher metadata to find a suitable Java runtime,
evaluates the appropriate conditions to determine the JVM arguments, and starts a Java process accordingly.

In the Java process, the **Janex Boot** (provided as a JAR on the class path) is loaded, which supplies a custom
ClassLoader that reads classes and resources directly from the Janex file.

Before this, we already had a prototype, whose documentation includes design concepts and other content about this file.
For more information, please refer to [janex-dev/japp](https://github.com/janex-dev/japp).

This document is an improvement based on the prototype, and we hope to make it safer, more stable, and easier to extend.

## Data Types

### Basic Data Types

Janex uses little-endian encoding for fixed-width binary integer and floating-point fields. `vuint`
and values inside CBOR objects use the encodings defined in their respective sections below.

This document uses `u8`/`u16`/`u32`/`u64` to represent 8/16/32/64-bit unsigned integers,
uses `i8`/`i16`/`i32`/`i64` to represent 8/16/32/64-bit signed integers,
and uses `f32`/`f64` to represent 32/64-bit floating-point numbers.

`bool` is represented by `u8`, where `true` is any non-zero value and `false` is zero.

### Complex Data Types

This document uses pseudocode similar to Rust structs to represent complex data types. For example:

```rust
struct MyStruct {
    length: u32,
    data: [u8; length],
}
```

Here, `length` is a 32-bit unsigned integer, and `data` is a byte array of length `length`.

### Variable-length integers

Janex uses unsigned Little Endian Base 128 (ULEB128) to efficiently encode unsigned integers in some structures.
This document calls the resulting 64-bit value type `vuint`:

```rust
type vuint = u64;
```

A `vuint` uses the standard ULEB128 byte layout. It is stored as one to ten bytes, with the least-significant
seven-bit group first. The lower seven bits of each byte carry integer data, and the most significant bit (MSB)
serves as the continuation flag:

- If the MSB is `0`, the current byte is the last byte of the integer.
- If the MSB is `1`, more bytes follow; the next byte continues the encoding.

Because `vuint` has a width of 64 bits, its encoding must not exceed ten bytes, and the tenth byte can contain
at most one non-zero payload bit. Janex writers emit the shortest ULEB128 representation. Readers also accept
zero-padded ULEB128 representations within the ten-byte width limit.

### Dynamic Array

Janex uses the following structure to store dynamically sized arrays. The `elements_count` field specifies the
number of elements, followed by the elements themselves serialized in sequence:

```rust
struct Vec<T> {
    /// The number of elements in the array.
    elements_count: vuint,

    /// The array elements, each serialized according to the type `T`.
    elements: [T; elements_count],
}
```

### String

String is a special `Vec<u8>` where the bytes are UTF-8 encoded string data:

```rust
type String = Vec<u8>;
```

### Tagged Payload

A variable-length structure with a integer tag:

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

This structure makes it easy for parsers to skip unrecognized fields. 

In this document, we use `#[repr(TaggedPayload<T>)]` to annotate a struct to use such a layout.

Since the binary structure of `String` is also `payload_bytes` + `payload`, 
for `TaggedPayload` with a `String` payload,
the `String` type can be used directly to replace these two fields. 

For example:

```rust
#[repr(TaggedPayload<u32>)]
struct MyStruct {
    tag: u32,
    value: String,
}
```

### Deterministic CBOR Objects

Janex uses [CBOR](https://www.rfc-editor.org/rfc/rfc8949.html) for extensible metadata structures that
explicitly declare a CBOR representation.

```rust
/// A length-delimited byte sequence containing exactly one CBOR data item.
type CborObject = Vec<u8>;
```

Every CBOR data item used by Janex, whether length-delimited by `CborObject` or embedded directly by a
schema, must satisfy the Core Deterministic Encoding Requirements in RFC 8949 Section 4.2.1, including
preferred serialization, definite lengths, and deterministic map-key ordering. A `CborObject` must
contain exactly one complete CBOR data item and no trailing bytes. Readers must reject:

- non-shortest integer, length, or tag encodings;
- indefinite-length items;
- duplicate map keys;
- invalid UTF-8 text strings;
- trailing bytes after the first data item within a `CborObject` or another explicitly delimited CBOR
  payload; and
- an item that does not match the schema required at that location.

Current Janex CBOR schemas use unsigned integers, signed integers, byte strings, text strings, arrays,
maps, booleans, and explicitly permitted `null` values. Floating-point values, CBOR `undefined`,
bignum tags, and all other tags or simple values are invalid unless a future schema explicitly permits
them. Unsigned integers must fit in `u64`, and negative integers must fit in `i64`. Integers inside
CBOR use CBOR's own deterministic integer representation, not `vuint` or Janex's little-endian fixed
integer representation.

Readers must apply implementation limits to encoded size, collection length, text and byte-string
length, and nesting depth before allocating resources.

### Timestamp

Janex uses a 96-bit high-precision timestamp, which can represent times approximately 292.2 billion years
before or after the Unix epoch (`1970-01-01T00:00:00Z`), with nanosecond precision.

```rust
struct Timestamp {
    /// The number of seconds elapsed since `1970-01-01T00:00:00Z`.
    /// May be negative for timestamps before the epoch.
    epoch_second: i64,

    /// The sub-second component of the timestamp, in nanoseconds.
    ///
    /// Must be in the range `[0, 1_000_000_000)`.
    nanos: u32,
}
```

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
    /// No checksum.
    /// 
    /// The `checksum` field is empty.
    NONE = 0,
    
    /// XXH64 checksum.
    /// 
    /// The elements count of the `checksum` field is `8`.
    XXH64 = 0x0101,
    
    /// SHA-256 checksum.
    /// 
    /// The elements count of the `checksum` field is `32`.
    SHA256 = 0x8101,

    /// SHA-512 checksum.
    ///
    /// The elements count of the `checksum` field is `64`.
    SHA512 = 0x8102,
    
    /// SM3 checksum.
    ///
    /// The elements count of the `checksum` field is `32`.
    SM3 = 0x8301,
}
```

For a known algorithm, the length of `checksum` must match the length specified by
`ChecksumAlgorithm`. The length prefix permits readers to skip the value of an unknown algorithm and
continue parsing subsequent metadata. If a checksum is required for validation, a reader must reject
an unsupported algorithm because it cannot perform that validation.

CBOR metadata represents the same value as a two-element array:

```cbor
ChecksumObject = [algorithm: uint, checksum: bstr]
```

The array must contain exactly two elements. `algorithm` must fit in `u16`, and `checksum` must satisfy
the same algorithm-specific length requirements as the binary `Checksum` structure.

## Janex File Structure

The Janex file is the binary format produced by the Janex build tool for packaging and distributing
Java programs as self-contained executables. Its overall layout is as follows:

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

External regions may be described by `FileMetadataObject.external_header` and
`FileMetadataObject.external_tail`. Their physical presence is independent of whether these
descriptions are present. External regions are not Janex sections and do not appear in the section
table.

### `FileMetadata` Section

```rust
struct FileMetadataSection {
    /// The magic number identifying the `FileMetadata` section.
    magic_number: u64, // 0x4154_4144_4154_454d ("METADATA")

    /// The major version number of the Janex file format.
    ///
    /// Readers must reject files with an unsupported major version.
    /// 
    /// Currently, Janex is in the early development stage, and the major version number is 0.
    major_version: u32,

    /// The minor version number of the Janex file format.
    ///
    /// For a nonzero major version, readers should accept files with a higher minor version within
    /// the same major version, ignoring unknown fields or entries where their enclosing structures
    /// permit this.
    ///
    /// While `major_version` is `0`, readers must reject files whose minor version they do not
    /// explicitly support. During active format design, edits to this specification, including
    /// changes to the described binary layout, do not automatically change the minor version; a new
    /// value is assigned only when establishing a distinct supported format revision.
    ///
    /// The current minor version is `1`.
    minor_version: u32,

    /// The deterministic CBOR file-metadata map.
    metadata: CborObject, // FileMetadataObject
    
    /// The verification information.
    verification_info: VerificationInfo,
    
    /// The end mark of the file.
    ///
    /// Always `0x444e_4558_454e_414a` ("JANEXEND").
    ///
    /// A reader first determines the end offset of `JanexFile`, reads this footer, uses
    /// `metadata_length` to locate `FileMetadata`, and uses `file_length` to locate the start of
    /// `JanexFile`.
    end_mark: u64,  // 0x444e_4558_454e_414a ("JANEXEND")
    
    /// The length in bytes of the metadata section.
    metadata_length: u64,

    /// The total length in bytes of the `JanexFile` structure.
    ///
    /// The reader subtracts an externally supplied tail length from the physical file size to obtain
    /// the end offset of `JanexFile`, then uses this value to locate its start.
    file_length: u64,
}
```

`metadata` must contain a `FileMetadataObject`, represented by the following deterministic CBOR map:

```cbor
FileMetadataObject = {
    0: flags,             // uint; currently 0
    1: section_table,     // [SectionInfoObject, ...]
    2: external_header,   // ExternalRegionObject
    3: external_tail,     // ExternalRegionObject
    ? 4: extensions,      // map<tstr, any>
    ? 5: required_features, // [FeatureIdObject, ...]
}

FeatureIdObject = uint / tstr
```

Keys `0` through `3` are required. `flags` must currently be `0`. `section_table` may be empty because
it does not include the final `FileMetadata` section. `extensions`, when present, is a text-keyed map
whose values follow their key-specific schemas; an empty map is valid, although writers should omit
it.

#### Metadata Evolution and Extensions

Maps that define Janex core structures use unsigned integer keys. Maps intended for attributes or
third-party extensions use non-empty text keys. The `janex.` key prefix is reserved for this
specification; third-party keys should use a reverse-domain prefix such as `org.example.`. The
definition of a key determines the exact schema and semantics of its value.

An unknown key must not change the interpretation of known keys and may be ignored. A new field that
is required to interpret existing data must instead be guarded by an explicit required feature, a new
section type, or a new supported format revision. A tool that rewrites a file while claiming to
preserve its semantics must preserve unknown optional map entries at the data-model level, although it
may re-encode them in the required deterministic form.

Empty maps and arrays are valid wherever their schema permits an empty collection. Writers should
omit an optional empty map, but readers must accept it when the containing key or section is present.
An omitted key and a `null` value are distinct: omission means that no value was supplied, while
`null` has meaning only where the field's schema explicitly defines it.

`required_features` declares features that a reader must understand to interpret the file correctly.
Numeric feature IDs are assigned by this specification. A text feature ID must be non-empty and
follow the text-key naming rules above. Feature IDs must be unique. The array must be sorted by the
bytewise lexicographic order of each feature ID's deterministic encoding, matching the map-key ordering
in RFC 8949 Section 4.2.1. A reader must reject a file containing an unsupported required feature. No
core required feature IDs are currently defined. A writer that uses no required features should omit
key `5`; readers must also accept an empty array, which has the same meaning as omission.

#### `SectionInfoObject` Map

Each element of `FileMetadataObject.section_table` is a deterministic CBOR map:

```cbor
SectionInfoObject = {
    0: section_type,      // uint fitting in u64
    1: id,                // uint fitting in u64
    2: length,            // uint fitting in u64
    3: checksum,          // ChecksumObject
    ? 4: metadata,        // section-type-specific map
}
```

Generally, `section_type` is the same as the `magic_number` of the section content when that section
has a magic number. A section is identified within one `JanexFile` by `(section_type, id)`. Two entries
with the same pair are invalid. IDs may be sparse and carry no ordering, priority, or semantic
meaning. They are file-local identities rather than stable cross-file identities. A tool may remap an
ID only when it also rewrites every affected reference. A section type that permits at most one
instance in a file must use ID `0` for that instance.

`length` is the exact encoded byte length of the corresponding section. `checksum` covers those exact
section bytes. Unless its algorithm is `NONE`, readers must verify the checksum before trusting the
section contents.

The optional `metadata` map is interpreted according to `section_type`. An empty map is valid; writers
should omit key `4` when it is empty. Unknown keys in a known section-type metadata map are optional
and may be ignored unless their semantics are guarded by a feature listed in
`FileMetadataObject.required_features`.

#### Section References

A typed section reference is one deterministic CBOR data item embedded directly at the reference's
position. It is not wrapped in `CborObject` and therefore has no preceding Janex `Vec<u8>` length. The
type parameter determines the required `section_type` and has no binary representation:

```cbor
SectionRef<T> =
    id: uint
  / [reference_kind: SectionReferenceKindObject, payload: bstr]

SectionReferenceKindObject = uint / tstr
```

The `uint` form is a local reference and must resolve to the unique section identified by `(T, id)` in
the same `JanexFile`; `id` must fit in `u64`. All currently defined section references use this form.
In particular, IDs from `0` through `23` occupy one byte.

The two-element array reserves an explicitly framed form for future reference mechanisms. Numeric
reference kinds are assigned by this specification. A text reference kind must be non-empty and
follow the text-key naming rules in
[Metadata Evolution and Extensions](#metadata-evolution-and-extensions). The `payload` byte string
must contain exactly one complete deterministic CBOR data item and no trailing bytes. Its byte-string
length lets a reader skip an unsupported payload without recursively scanning it. A reader that must
resolve a reference must reject an unsupported reference kind. No core extended reference kinds are
currently defined. Writers that do not use an extension-defined reference kind must use the `uint`
form.

An untyped reference, if introduced by a future schema, must additionally identify its target section
type.

Currently supported section types:

```rust
#[repr(u64)]
enum SectionType {
    /// Represents arbitrary data in a Janex file. Janex tools will not use these sections.
    ///
    /// Padding sections do not require a `magic_number`.
    Padding = 0x0047_4e49_4444_4150, // "PADDING\0"

    /// The `FileMetadata` section. Used to store file metadata.
    /// 
    /// This section is always the last section in `sections`.
    /// 
    /// It is not recorded in `FileMetadataObject.section_table`, because that table is inside this
    /// section and recording the section there would create a self-reference.
    /// This section verifies itself using the internal `verification_info` information.
    FileMetadata = 0x4154_4144_4154_454d, // "METADATA"

    /// The `Attributes` section.
    Attributes = 0x2e53_4249_5254_5441, // "ATTRIBS."
    
    /// Stores indexed blobs, including structural metadata and file contents.
    BlobPool = 0x4c4f_4f50_424f_4c42, // "BLOBPOOL"
    
    /// The `RootConfigGroup` section.
    RootConfigGroup = 0x5055_4f52_4747_4643, // "CFGGROUP"

    /// The `ResourceGroups` section. Contains all embedded resource groups.
    ResourceGroups = 0x0053_5052_4753_4552, // "RESGRPS\0"

    /// The `StringPool` section. A shared string pool used by the class file transform
    /// and `RefBody` resource paths.
    StringPool = 0x004c_4f4f_5052_5453, // "STRPOOL\0"
}
```

All entries in `FileMetadataObject.section_table` describe sections arranged consecutively within
`JanexFile.sections`. If additional data or padding needs to be inserted between them, the `Padding`
section can be used.

Unknown sections may be skipped. A feature that requires an unknown section type must be declared in
`FileMetadataObject.required_features`, causing readers that do not support it to reject the file.

#### `ExternalRegionObject`

`ExternalRegionObject` states whether bytes outside the `JanexFile` structure are described and
constrained by Janex metadata. External regions do not have Janex section magic numbers and are not
addressed by `SectionInfoObject`.

```cbor
ExternalRegionObject =
    [0]                               // NotDescribed
  / [1, size: uint, ChecksumObject]   // Described
```

The array tag is part of an explicit two-variant enum. `NotDescribed` contains exactly one element.
`Described` contains exactly three elements. Other tags or array lengths are invalid.

`NotDescribed` means that Janex metadata does not describe or constrain the corresponding external
region. The physical region may be absent or may contain arbitrary bytes, and tools may modify it
without rewriting the Janex metadata. `Described` requires the physical region to have exactly the
specified `size`. Consequently, `[1, 0, [0, h'']]` (where checksum algorithm `0` is `NONE`)
explicitly requires that the physical region be absent; it is not equivalent to `[0]`. When `size` is
zero, the checksum must use the `NONE` algorithm. When `size` is nonzero, the checksum must not use
the `NONE` algorithm and a reader must verify it.
Metadata associated with the containing format may be stored in `FileMetadataObject.extensions`.

For a physical file of `physical_file_size` bytes, a reader receives `external_tail_length` from its
caller and calculates:

```text
janex_end   = physical_file_size - external_tail_length
janex_start = janex_end - file_length
```

Both subtractions must use checked arithmetic. The reader locates the fixed footer immediately before
`janex_end`. The actual external header is the byte range before `janex_start`, and the actual external
tail is the byte range from `janex_end` to `physical_file_size`. After decoding `FileMetadata`, the
reader must compare an external region's actual size with its declared `size` and verify its checksum
only when the corresponding `ExternalRegionObject` is `Described`. When it is `NotDescribed`, the reader
imposes no metadata-derived size or checksum requirement on that region.

The reader must reject the file unless `end_mark` has its required value, `metadata_length` equals the
exact encoded length of `FileMetadataSection`, and the following equation holds using checked
arithmetic:

```text
file_length = 8 + sum(FileMetadataObject.section_table[*].length) + metadata_length
```

These requirements ensure that the located `JanexFile`, its consecutively stored sections, and its
final metadata section have one consistent set of boundaries.

The standalone Janex layout uses `external_tail_length = 0`. A containing format may define how its
caller obtains a nonzero value. For example, a JAR-formatted launcher appended as an external tail may
determine the length of its own JAR region before locating the preceding `JanexFile`. Janex itself does
not scan external bytes for `end_mark` and does not infer the tail length from its contents.

#### `VerificationInfo` Structure

`VerificationInfo` is a tagged payload. Its `verification_type` is part of the authenticated input;
the payload containing the resulting checksum or signature is not.

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

`file_metadata_start` is the offset of the first byte of `FileMetadataSection.magic_number`, and
`verification_type_end` is the offset immediately after the `verification_type` tag byte. This range
is therefore the exact, contiguous on-disk representation from the start of `FileMetadataSection`
through and including `verification_type`. These original bytes must be used directly without parsing,
normalizing, or re-encoding them. Consequently, changing the verification type invalidates an existing
checksum or signature instead of allowing the same verification payload to be interpreted under
another type.

For every known variant, the decoded payload must consume exactly `payload_bytes`; trailing or
unconsumed bytes are invalid. An empty OpenPGP or CMS payload is invalid. A reader must reject an
unknown `verification_type`, because it cannot determine whether or how the metadata was verified.

##### Verification Policy

`None` and `Checksum` do not authenticate the file against an active attacker. A reader must report
them as unauthenticated and must reject them whenever its caller requires authenticated input.
Preventing removal of an entire signature requires such an external verification policy: a file
whose signature was replaced with `None` is indistinguishable from a file originally written with
`None` until the caller requires a particular authenticated verification type and trusted signer.
OpenPGP and CMS verification must therefore use caller-provided trust and algorithm policies; a
cryptographically valid signature from an untrusted signer is not sufficient.

An OpenPGP- or CMS-tagged file with a malformed or cryptographically invalid signature is invalid. A
reader must not retry it as another verification type, treat it as `None` or `Checksum`, or continue
using content obtained from it. An implementation that only inspects the file without authenticating
it must explicitly report that no authentication was performed and must not expose the result as
authenticated.

The caller's verification policy must define at least:

- whether authenticated input is required;
- which verification types, signature algorithms, digest algorithms, keys, and certificates are
  accepted;
- which signer or signers must succeed when a signature container identifies more than one signer;
- the validation time and any required expiration or revocation checks.

Algorithm support does not imply algorithm acceptance. An implementation may decode an algorithm that
the caller's current policy rejects. Implementations supporting OpenPGP or CMS verification must
support SHA-256 as a content-digest algorithm for that signature format. Writers must not create, and
readers must not accept, Janex signatures that depend on MD5, SHA-1, or RIPEMD-160. Other algorithms
are accepted only when permitted by the caller's policy.

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

Issuer information in the unhashed subpacket area is advisory and must not establish signer identity
or trust. A reader must reject an unsupported critical subpacket. The signing key and its trust data
are supplied by the caller or an external key store; they are not carried by `VerificationInfo`.

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

For each signer required by policy, the reader must independently recompute the digest of
`verification_input`, validate the signed attributes and signature, identify the signing certificate
or externally supplied key, and apply the caller's trust, key-usage, validity, and revocation rules.
Certificates or revocation information embedded in `SignedData` are inputs to that process and are
not trusted merely because they are embedded. Unsigned attributes must not affect acceptance of the
primary signature. If multiple `SignerInfo` values are present, the caller's policy determines whether
one, a specified subset, or all of them must succeed.

##### Authenticated Content Scope

To authenticate every Janex section and both external data regions, each entry in
`FileMetadataObject.section_table` must use a cryptographically secure checksum, both external regions
must be `Described`, and each described non-empty external region must use a cryptographically secure
checksum. The signed
`verification_input` then binds those checksums and any explicit zero-size requirements to the file
metadata. `SHA256`, `SHA512`, and `SM3` are cryptographically secure checksum algorithms for this
purpose; `NONE` and `XXH64` are not. A reader claiming this scope of authentication must verify every
required checksum before trusting the corresponding bytes. An external region marked
`NotDescribed` is outside the integrity guarantees provided by Janex metadata.

The verification payload itself is not part of `verification_input`, and the footer is enforced by
the structural boundary checks rather than by the detached signature. Authentication therefore binds
the file's metadata and all content represented by authenticated checksums; it is not a canonical
byte-for-byte signature of the complete physical representation.

Readers must verify a byte source that remains stable for the duration of verification and use. If an
attacker may modify the source concurrently, the reader must use an immutable snapshot or another
mechanism that prevents verified bytes from being replaced before they are consumed.

##### Rollback and Replacement

A valid signature proves integrity and signer authorization under the caller's policy; it does not
prove that the file is the newest authorized release. An attacker may replace a Janex file with an
older file or a different file signed by the same trusted signer. Applications requiring rollback or
object-substitution protection must compare an expected file identity, release identifier, monotonic
version, or digest obtained from trusted state outside the file. `major_version` and `minor_version`
identify the Janex format revision and must not be used as application release counters. OpenPGP
Signature Creation Time and CMS signing-time attributes are signer assertions and do not by themselves
provide trusted freshness.

### `Attributes` Section

```rust
struct AttributesSection {
    /// The magic number identifying this as an attributes section.
    ///
    /// Always `0x2e53_4249_5254_5441` ("ATTRIBS.").
    magic_number: u64, // 0x2e53_4249_5254_5441 ("ATTRIBS.")

    /// One deterministic CBOR `AttributesObject`.
    attributes: CborObject,
}
```

Each Janex file may contain at most one `Attributes` section, whose section ID must be `0`.
Attributes are descriptive metadata unless another part of this specification explicitly assigns a
particular attribute operational semantics.

The CBOR object is a text-keyed map:

```cbor
AttributesObject = {
    * attribute_name: attribute_value,
}
```

Attribute names follow the text-key naming rules in
[Metadata Evolution and Extensions](#metadata-evolution-and-extensions). Each value is interpreted
according to its attribute name. Readers that do not recognize an attribute must ignore it and may
preserve its encoded value when rewriting the file.

Writers should omit the section when it would contain no attributes. Readers must nevertheless accept
an empty map. The top-level value must not be `null` or an array. An individual attribute value may be
`null` only when that attribute's schema explicitly assigns meaning to `null`.

### `RootConfigGroup` Section

Each Janex file may contain at most one `RootConfigGroup` section, whose section ID must be `0`.

```rust
struct RootConfigGroupSection {
    /// The magic number identifying the `RootConfigGroup` section.
    ///
    /// Always `0x5055_4f52_4747_4643` ("CFGGROUP").
    magic_number: u64, // 0x5055_4f52_4747_4643 "CFGGROUP"

    /// One deterministic CBOR `ConfigGroupObject`.
    root_group: CborObject,
}
```

#### `ConfigGroupObject` Map

```cbor
ConfigGroupObject = {
    ? 0: condition,       // tstr
    ? 1: main_class,      // tstr / null
    ? 2: main_module,     // tstr / null
    ? 3: module_path,     // [ResourceGroupReferenceObject, ...] / null
    ? 4: class_path,      // [ResourceGroupReferenceObject, ...] / null
    ? 5: agents,          // [JavaAgentObject, ...] / null
    ? 6: jvm_options,     // [tstr, ...] / null
    ? 7: subgroups,       // [ConfigGroupObject, ...]
    ? 8: extensions,      // map<tstr, any>
}
```

An empty `ConfigGroupObject` is valid. A missing `condition` means that the group is unconditional;
`condition` must not be `null`. When present, it is a CEL expression that must evaluate to `bool` or
`int` as described in [Conditions](#conditions). The root condition may be used to determine whether a
runtime and platform are suitable. Subgroup conditions select environment-specific configuration.

The launcher visits the root and its `subgroups` in depth-first pre-order. Each subgroup is evaluated
independently. For a group whose condition matches, the configuration keys are applied as follows:

- a missing key makes no contribution;
- `main_class` and `main_module` replace the value previously selected, while `null` clears it;
- an array in `module_path`, `class_path`, `agents`, or `jvm_options` appends its elements in array
  order, while `null` clears all elements accumulated for that key; and
- an empty array is valid and appends nothing.

`subgroups` preserves array order and must not be `null`. `extensions`, when present, follows the
text-key naming rules in [Metadata Evolution and Extensions](#metadata-evolution-and-extensions). An
empty extension map is valid, although writers should omit it.

#### `ResourceGroupReferenceObject`

A resource-group reference is one of the following exact-length CBOR arrays:

```cbor
ResourceGroupReferenceObject =
    [0, resource_groups: SectionRef<ResourceGroupsSection>, group_name: tstr]
  / [1, purl: tstr, checksum: ChecksumObject]
```

Variant `0` refers to an embedded resource group. `resource_groups` must resolve to a
`ResourceGroups` section, and `group_name` must match exactly one group in that section.

Variant `1` refers to a package that the launcher resolves and downloads without resolving transitive
dependencies. `purl` must be a canonical Package URL identifying one concrete package version and must
not use a `vers` qualifier. Maven artifacts should use `pkg:maven`; a non-default Maven repository is
specified with the `repository_url` qualifier. The checksum verifies the downloaded package and is
part of the Janex trust policy rather than a PURL `checksum` qualifier. Examples include:

- `pkg:maven/org.slf4j/slf4j-api@2.0.9`
- `pkg:maven/org.apache.xmlgraphics/batik-anim@1.9.1?type=pom`
- `pkg:maven/net.sf.jacob-projec/jacob@1.14.3?classifier=x64&type=dll`

#### `JavaAgentObject`

```cbor
JavaAgentObject = [reference: ResourceGroupReferenceObject, option: tstr]
```

The array must contain exactly two elements. An empty `option` means that no agent option is supplied.

### `ResourceGroups` Section

`ResourceGroups` contains embedded resource groups. A Janex file may contain any number of
`ResourceGroups` sections. Each is identified by the ID of its corresponding `SectionInfoObject`.

When key `4` (`metadata`) is present in the corresponding `SectionInfoObject`, it must be the following
deterministic CBOR map:

```cbor
ResourceGroupsMetadataObject = {
    ? 0: string_pool,     // SectionRef<StringPoolSection>
    ? 1: extensions,      // map<tstr, any>
}
```

`string_pool` selects the string pool used by every `ClassFile` transform and `RefBody` path contained
in this `ResourceGroups` section. It is required if any such value occurs and may refer to a string
pool shared with other resource-group sections. `extensions`, when present, follows the text-key
naming rules in [Metadata Evolution and Extensions](#metadata-evolution-and-extensions). Empty maps
are valid, although writers should omit an empty `extensions` map and omit section metadata entirely
when it would be empty.

Each resource group is a logical container of related files, typically corresponding to a single JAR
or module from the original Java project. Group names must be unique within the section.

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

A `ResourceGroup` represents a logical container of related files, typically corresponding to a single
JAR or module from the original Java project.

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
    metadata: CborObject,

    /// The number of `Resource` entries stored in this group.
    resources_count: vuint,

    /// A reference to the compressed array of resource metadata entries for this group.
    ///
    /// Writers normally place this structural data in a blob pool separate from file contents.
    resources: BlobRef<[Resource; resources_count]>
}
```

The resource-group metadata is a text-keyed map:

```cbor
ResourceGroupMetadataObject = {
    * metadata_name: metadata_value,
}
```

Metadata names follow the text-key naming rules in
[Metadata Evolution and Extensions](#metadata-evolution-and-extensions). Unknown entries have no
effect on resource decoding and may be ignored. The empty map is valid and is the canonical
representation of no group metadata; unlike an optional map field in a surrounding CBOR object, this
object is required by the binary `ResourceGroup` structure.

#### `Resource`

A `Resource` represents a single entry (regular file, directory, or symbolic link) within a resource
group.

Resource entries normally contain only metadata. Small regular-file contents may be stored inline;
other contents are stored in one or more blobs in `BlobPool` sections.

```rust
enum Resource {
    /// Represents a regular file.
    File {
        /// The resource type tag for this variant.
        /// 
        /// Always `0x00534552` ("RES\0")
        resource_type: u32, // 0x00534552 ("RES\0")

        /// The path of this resource within its resource group.
        path: ResourcePath,

        /// The content of this resource and its logical transforms.
        content: FileContent,

        /// Optional metadata fields associated with this resource (e.g., timestamps, checksum).
        fields: Vec<ResourceField>,
    },

    /// Represents a directory entry.
    Directory {
        /// The resource type tag for this variant.
        ///
        /// Always 0x00524944 ("DIR\0")
        resource_type: u32, // 0x00524944 ("DIR\0")

        /// The path of this directory within its resource group.
        path: ResourcePath,

        /// Optional metadata fields associated with this directory (e.g., timestamps, permissions).
        fields: Vec<ResourceField>,
    },

    /// Represents a symbolic link.
    SymbolicLink {
        /// The resource type tag for this variant.
        /// 
        /// Always 0x4c4d5953 ("SYML")
        resource_type: u32, // 0x4c4d5953 ("SYML")

        /// The path of this symbolic link within its resource group.
        path: ResourcePath,

        /// The target path that this symbolic link points to.
        target: ResourcePath,

        /// Optional metadata fields associated with this symbolic link.
        fields: Vec<ResourceField>,
    }
}
```

#### File Content

`ContentRef<T>` stores the encoded representation of a value in one of three forms:

```rust
#[repr(u8)]
enum ContentRef<T> {
    /// Stores the complete encoded representation of `T` inline.
    Inline {
        content_type: u8, // 0
        bytes: Vec<u8>,
    },

    /// Uses the complete decoded representation of one blob.
    BlobRef {
        content_type: u8, // 1
        blob: BlobRef<T>,
    },

    /// Concatenates decoded blob ranges in array order.
    BlobSlices {
        content_type: u8, // 2
        slices: Vec<BlobSlice>,
    },
}
```

For `Inline`, `bytes` must be a valid encoding of `T`; an empty representation is encoded by an
empty `bytes` vector. For `BlobRef`, the complete decoded blob is the encoded representation of `T`.
For `BlobSlices`, concatenating the selected ranges in array order must produce a valid encoding of
`T`. Readers must use checked arithmetic when calculating the total concatenated size and must reject
an unsupported `content_type`.

`BlobSlices.slices` must contain at least one element. A `BlobSlices` value containing exactly one
slice that covers its complete decoded blob is not permitted; it must use the `BlobRef` variant
instead. Zero-length slices are not permitted; therefore, an empty representation has the single
canonical representation `Inline` with an empty byte vector. The size threshold at which a writer
chooses `Inline` instead of pooled storage is a writer policy and is not part of the format semantics.

`FileContent` describes the encoded bytes associated with a regular-file entry and any logical
transforms applied before those bytes were placed inline or in blobs:

```rust
struct FileContent {
    /// The bytes produced after applying all content transforms.
    source: ContentRef<[u8]>,

    /// The transforms in the order in which the encoder applied them.
    transforms: Vec<ContentTransform>,
}

struct ContentTransform {
    /// The number of bytes supplied to this transform by the encoder.
    input_size: vuint,

    /// Identifies the content transform.
    method: ContentTransformId,

    /// Length-prefixed properties required to reverse the transform.
    properties: Vec<u8>,
}

#[repr(u8)]
enum ContentTransformId {
    /// A class-file-aware transform using the shared `StringPool`.
    CLASSFILE = 1,
}
```

As with blob filters, transforms are stored in encoding order and reversed by readers. The byte
sequence obtained from `source` is passed through the transforms from last to first. Reversing each
transform must produce exactly its `input_size` bytes. An empty transform array means that `source`
already contains the logical file content. Otherwise, the logical file size is the `input_size` of the
first transform.

Content transforms operate after blob decoding and slice concatenation. They are properties of a
logical file entry, not of a physical blob. Consequently, a solid-compressed blob may contain encoded
ranges belonging to entries with different transforms. A reader must reject a regular-file entry that
uses an unsupported content transform because it cannot reconstruct the logical content. The length
prefix of `properties` still permits readers to skip the properties and continue parsing subsequent
metadata.

##### Class File Transform

Janex typically extracts frequently occurring strings (such as common package names, type descriptors,
and method signatures) from class file constant pools into a shared `StringPool`. Writers store the
transformed class representations as file-content sources and may pack them with other sources into
blobs before applying Zstandard at the blob layer. This approach allows strings that appear across
many class files to be stored only once while retaining solid-compression support.

The `ClassFile` content transform largely preserves the standard class file format, but introduces
the following modifications:

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

The input to the `ClassFile` transform must be a valid Java class file. The containing
`ResourceGroups` section must select a `StringPool` through its `ResourceGroupsMetadataObject`, and
reversing the transform uses that string pool. The `properties` field of `CLASSFILE` is currently
empty.

#### `ResourcePath`

`ResourcePath` represents the path of a `Resource`, for example `java/lang/Object.class`.

`ResourcePath` is composed of several parts separated by `/`. The `/` cannot be the first or last character, and cannot be empty.
Each part of `ResourcePath` cannot be empty, cannot be `.` or `..`, and cannot contain `/`.

In each `ResourceGroup`, `ResourcePath` must be unique.

`ResourcePath` using one of two encodings selected by the value of `length`:

- **`StringBody`** (when `length != 0`): the full path string is stored inline, with `length` giving
  its byte length.
- **`RefBody`** (when `length == 0`): the path is described by two integer indices into the shared
  `StringPool` — one for the directory component and one for the file name component. This encoding
  avoids repeating path strings that appear across many resources.

```rust
struct ResourcePath {
    /// The byte length of the inline path string, or `0` to indicate `RefBody` encoding.
    length: vuint,
    content: ResourcePathContent,
}

enum ResourcePathContent {
    /// Inline path encoding, used when `length != 0`.
    StringBody {
        /// The raw UTF-8 bytes of the full resource path (e.g., `"com/example/Foo.class"`).
        body: [u8; length],
    },

    /// Reference-based path encoding, used when `length == 0`.
    ///
    /// Requires the containing `ResourceGroups` section to select a `StringPool`.
    RefBody {
        /// The index of the directory path component in the `StringPool`
        /// (e.g., the index for `"com/example"`).
        directory_index: vuint,
        /// The index of the file name component in the `StringPool`
        /// (e.g., the index for `"Foo.class"`).
        file_name_index: vuint,
    }
}
```

#### `ResourceField`

`ResourceField` carries optional metadata attached to a resource entry. Each field is identified by a
1-byte `id`.

The supported fields are:

```rust
#[repr(TaggedPayload<u8>)]
enum ResourceField {
    /// Checksum of the logical resource content.
    ///
    /// Can be used by the extractor to verify data integrity after resolving the content reference.
    Checksum {
        /// The field ID for this variant.
        id: u8, // 0x01

        /// The number of bytes of the checksum payload.
        payload_bytes: vuint,

        /// The checksum of the logical resource content.
        checksum: Checksum,
    },

    Comment {
        /// The field ID for this variant.
        id: u8, // 0x02

        /// A UTF-8 encoded comment string associated with this resource.
        comment: String,
    },

    /// File creation timestamp.
    FileCreateTime {
        /// The field ID for this variant.
        id: u8, // 0x03

        /// The number of bytes of the timestamp payload.
        /// 
        /// Always `12` (the size of the `Timestamp` structure).
        payload_bytes: vuint,

        /// The file creation timestamp.
        timestamp: Timestamp,
    },

    /// File last-modification timestamp.
    FileModifyTime {
        /// The field ID for this variant.
        id: u8, // 0x04

        /// The number of bytes of the timestamp payload.
        /// 
        /// Always `12` (the size of the `Timestamp` structure).
        payload_bytes: vuint,

        /// The file last-modification timestamp.
        timestamp: Timestamp,
    },

    /// File last-access timestamp.
    FileAccessTime {
        /// The field ID for this variant.
        id: u8, // 0x05

        /// The number of bytes of the timestamp payload.
        /// 
        /// Always `12` (the size of the `Timestamp` structure).
        payload_bytes: vuint,

        /// The file last-access timestamp.
        timestamp: Timestamp,
    },

    /// POSIX file permission bits (e.g., `0o755`).
    PosixFilePermissions {
        /// The field ID for this variant.
        id: u8, // 0x06

        /// The number of bytes of the permissions payload.
        /// 
        /// Always `2` (the size of the `u16` structure).
        payload_bytes: vuint,

        /// The POSIX permission bits for this resource, stored as a 16-bit unsigned integer.
        permissions: u16,
    },

    /// A custom, application-defined metadata field.
    /// 
    /// Users should prefer this field type for custom metadata rather than using a custom `id`, 
    /// to avoid conflicts with field IDs that Janex may add in the future.
    ///
    /// Custom fields are not interpreted by Janex and are ignored during normal processing.
    /// They can be used to attach arbitrary metadata for tooling or third-party extensions.
    Custom {
        id: u8, // 0x7F

        /// The number of bytes of the name and content payload.
        payload_bytes: vuint,

        /// The name of the custom field, used to identify its purpose.
        name: String,

        /// The raw content bytes of the custom field.
        content: Vec<u8>,
    }
}
```

### `StringPool` Section

A shared string pool used by the class file transform and `RefBody` resource paths.

A Janex file may contain any number of `StringPool` sections. Each is identified by the ID of its
corresponding `SectionInfoObject`. A `ResourceGroups` section explicitly selects its string pool
through `ResourceGroupsMetadataObject.string_pool`; multiple resource-group sections may select the
same string pool.

```rust
struct StringPoolSection {
    /// The magic number identifying this section as a string pool.
    /// 
    /// Always `0x004c_4f4f_5052_5453` ("STRPOOL\0")
    magic_number: u64, // 0x004c_4f4f_5052_5453 ("STRPOOL\0")

    /// A reference to the string-pool data.
    data: BlobRef<StringPoolData>,
}

struct StringPoolData {
    /// The strings in pool-index order.
    ///
    /// This vector must contain at least one element, and element `0` must be an empty string.
    strings: Vec<String>,
}
```

All strings stored in the `StringPool` are encoded in standard UTF-8. When used to reverse a
`ClassFile` content transform, they need to be converted back to Modified UTF-8 encoding. The string
pool itself cannot use a content transform because it is bootstrap data required by `ClassFile`.

### `BlobPool` Section

A `BlobPool` stores independently decoded blobs. Stored blobs may contain resource metadata arrays,
string-pool data, file contents, or extension-defined data. Object semantics are assigned by typed
references and are not part of the blob-pool layout.

A Janex file may contain any number of `BlobPool` sections, including none when the file contains no
`BlobRef`. Each blob pool is identified by the `id` of its corresponding `SectionInfoObject`.
Blob-pool IDs must be unique within the section type, as required for all section IDs, and carry no
semantic meaning. Readers must not assume that a particular ID, physical position, or creation order
identifies a particular kind of data.

```rust
struct BlobPoolSection {
    magic_number: u64, // 0x4c4f_4f50_424f_4c42 ("BLOBPOOL")

    /// Blob representations and the blob table in the layout selected by the writer.
    bytes: [u8; ...],
}
```

#### Blob Encoding

`BlobEncoding` describes the stored representation of a blob and the filters applied to produce it:

```rust
struct BlobEncoding {
    /// The number of bytes in the stored representation.
    stored_size: vuint,

    /// The filters in the order in which the encoder applied them.
    filters: Vec<BlobFilter>,
}

struct BlobFilter {
    /// The number of bytes supplied to this filter by the encoder.
    input_size: vuint,

    /// Identifies the filter algorithm.
    method: BlobFilterId,

    /// Length-prefixed properties required to reverse the filter.
    properties: Vec<u8>,
}
```

A reader reverses the filters in array order: it processes the last filter first and the first filter
last. Reversing a filter must produce exactly `input_size` bytes. An empty filter array means that the
blob is stored without transformation, in which case its decoded size equals `stored_size`. Otherwise,
the decoded size of the blob is the `input_size` of the first filter.

The stored size and every filter input size are independently authoritative. Readers must enforce
implementation resource limits before allocating output buffers and must reject a filter that produces
a different number of bytes. Filter properties contain only information required by a decoder; writer
settings such as a compression level are not part of the format.

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

A reader must reject a blob that uses an unsupported filter because it cannot reconstruct the decoded
representation. The length prefix of `properties` still permits readers to skip an unsupported
filter's properties and continue parsing subsequent metadata.

The `properties` field of `ZSTD` is currently empty.

#### Blob Table

The length of `bytes` is `SectionInfoObject.length - 8`; therefore, a `BlobPool` section must have a
length of at least 8 bytes. Key `4` (`metadata`) of the corresponding `SectionInfoObject` is required
and must be the following deterministic CBOR map:

```cbor
BlobPoolMetadataObject = {
    0: table_offset,      // uint
    1: table_encoding,    // BlobEncodingObject
    2: table_checksum,    // ChecksumObject
    ? 3: extensions,      // map<tstr, any>
}

BlobEncodingObject = [stored_size: uint, filters: [BlobFilterObject, ...]]
BlobFilterObject = [input_size: uint, method: uint, properties: bstr]
```

Keys `0` through `2` are required. `table_offset`, `stored_size`, and each `input_size` must fit in
`u64`; `method` must fit in `u8`. `BlobEncodingObject` must contain exactly two elements, and each
`BlobFilterObject` must contain exactly three. The `filters` array may be empty, uses encoding order,
and has the same decoding and size semantics as binary `BlobEncoding`. `extensions`, when present,
follows the text-key naming rules in
[Metadata Evolution and Extensions](#metadata-evolution-and-extensions). An empty extension map is
valid, although writers should omit it. A reader must reject a blob pool whose table encoding uses an
unsupported filter. The section metadata uses `BlobEncodingObject` to locate and decode the bootstrap
blob table. Each `BlobInfo` uses the binary `BlobEncoding` structure defined above.

```rust
struct BlobTable {
    /// Blob metadata in blob-index order.
    blobs: Vec<BlobInfo>,
}

struct BlobInfo {
    /// The byte offset of the stored blob relative to the first byte of
    /// `BlobPoolSection.bytes`.
    offset: vuint,

    /// The encoding metadata for the stored and decoded blob representations.
    encoding: BlobEncoding,

    /// Optional metadata associated with this blob.
    fields: Vec<TaggedPayload<u32>>,
}
```

The stored blob table occupies the half-open range
`[BlobPoolMetadataObject.table_offset, BlobPoolMetadataObject.table_offset +
BlobPoolMetadataObject.table_encoding.stored_size)`. Reversing its filter chain must produce a valid
`BlobTable` and, unless the checksum algorithm is `NONE`, the decoded representation must match
`BlobPoolMetadataObject.table_checksum`. Blob-table filters must be self-contained and must not depend
on a `BlobRef` or other external data because the table is bootstrap metadata.

Each stored blob occupies the half-open range
`[BlobInfo.offset, BlobInfo.offset + BlobInfo.encoding.stored_size)`. Readers must reverse its filters
as specified by `BlobEncoding` and must validate all stored and decoded ranges using checked
arithmetic. Stored blob ranges must be within `BlobPoolSection.bytes` and must not overlap one another
or the stored blob table. The stored blob-table range must also be within `BlobPoolSection.bytes`. A
writer may place the table anywhere in the section; placing it after the blob representations permits
forward-only construction of the section payload.

#### Blob References

`BlobRef<T>` identifies one complete blob in a `BlobPool` section. A blob is an independently decoded
physical storage unit. The type parameter `T` describes the logical type of the complete decoded blob
and has no separate binary representation in `BlobRef`:

```rust
struct BlobRef<T> {
    /// A reference to the `BlobPool` section, encoded as a `SectionRef` CBOR item.
    blob_pool: SectionRef<BlobPoolSection>,

    /// The zero-based index of the blob in the referenced blob pool's `BlobTable`.
    blob_index: vuint,
}
```

The `blob_pool` reference must resolve to exactly one `BlobPool` section. The selected `BlobInfo`
supplies the location and encoding metadata of the stored representation. The `blob_index` must be
less than the number of entries in the selected `BlobTable`. After decoding, the complete result must
be a valid encoding of `T`. Multiple references may identify the same blob.

`BlobSlice` identifies a non-empty range in the decoded representation of one blob:

```rust
struct BlobSlice {
    /// The blob containing the decoded byte range.
    blob: BlobRef<[u8]>,

    /// The byte offset of the range in the decoded blob.
    decoded_offset: vuint,

    /// The number of bytes in the range. This value must be greater than zero.
    decoded_length: vuint,
}
```

The half-open range `[decoded_offset, decoded_offset + decoded_length)` must be contained in the
decoded blob. Readers must validate the range using checked arithmetic before copying or exposing it.

Each `BlobRef` is resolved independently and may reference any blob pool. A blob pool may contain both
structural metadata and file contents, and references from different roots may share a blob. Blob
indices are local to one blob pool and do not provide stable identity across files or revisions.

Writers should normally use separate blob pools for structural data and file contents when both are
substantial. In the typical layout, resource metadata arrays and string-pool data are placed in one
structural blob pool, while file contents are placed in another. Writers may combine them into one pool
for small files, and may use additional pools when this improves access locality, integrity isolation,
incremental updates, or independent distribution. Writers should not create one blob pool per resource
group without a concrete storage-policy reason.

This separation is a writer layout policy only. Readers must support any valid distribution of blobs
across blob pools. The ordinary sections located through `FileMetadata` must provide the initial
`BlobRef` values needed to begin decoding pooled data. A pooled value may contain references to other
pooled values, but the resulting reference graph must be finite and acyclic; no value may directly or
indirectly depend on itself.

## Conditions

Janex allows users to declare runtime environment requirements for a program, such as the minimum Java version,
operating system, and CPU architecture.

The Janex Launcher evaluates these conditions against each candidate Java installation
and the current host platform to determine which installations are eligible and which has the highest priority.

Conditions also govern which classpath entries, module path entries, JVM arguments, and other
configuration values are applied at launch time, enabling a single Janex file to carry
platform-specific or version-specific configuration.

Users express these requirements using [Common Expression Language (CEL)](https://cel.dev/overview/cel-overview)
when building a Janex file.

A condition expression must evaluate to either `bool` or `int`:

- If it evaluates to `bool`, the associated configuration is applied when the result is `true`.
- If it evaluates to `int`:
  - For a root group, the value represents a priority score. The launcher selects the Java installation with the highest score.
  - For a subgroup, any value is equivalent to `true`.

### Environment

At runtime, the Janex Launcher makes the following variables available for use in condition expressions:

```rust
// Information about a candidate Java installation.
let java: Java = ...;

// Information about the current host platform.
let platform: Platform = ...;
```

The `Java` struct provides information about a candidate Java runtime environment:

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

The `Platform` struct provides information about the current host platform:

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

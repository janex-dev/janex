# Janex File Format

Janex is a sectioned, multi-root container format. Its core stores shared content, metadata, and
verification information. Optional application sections describe launchable targets for supported
runtimes.

## Data Types

### Basic Data Types

Janex uses little-endian encoding for fixed-width binary integer and floating-point fields. `vuint`
and values inside CBOR objects use the encodings defined in their respective sections below.

This document uses `u8`/`u16`/`u32`/`u64` to represent 8/16/32/64-bit unsigned integers,
uses `i8`/`i16`/`i32`/`i64` to represent 8/16/32/64-bit signed integers,
and uses `f32`/`f64` to represent 32/64-bit floating-point numbers.

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

type CborMap = CborValue;   // map
```

All values follow RFC 8949 Section 4.2.1 Core Deterministic Encoding and their applicable schema.
Binary fields use `Sized<CborValue>` when they need an explicit byte boundary.

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
    XXH64 = 1,  // 8 bytes
    
    SHA256 = 2, // 32 bytes

    SHA512 = 3, // 64 bytes
    
    SM3 = 4,    // 32 bytes
}
```

Algorithm ID `0` is reserved. `digest` has no length prefix; its length must match the algorithm.
Readers may skip unknown algorithms when the containing field provides a byte boundary. Required
validation accepts supported algorithms only.

In CBOR:

```cddl
ChecksumObject = bstr .size (1..)
```

The byte string contains the exact encoding of one `ChecksumValue`: the first byte is the algorithm
ID and the remaining bytes are `digest`.

## Janex File Structure

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

`metadata.value` is a `FileMetadataObject`:

```cddl
FileMetadataObject = {
    0: [* SectionInfoObject],                    ; section_table
    ? 1: ExternalRegionObject,                   ; external_header
    ? 2: ExternalRegionObject,                   ; external_tail
    ? 3: SectionRef,                            ; default_application
    ? 4: NonemptyText,                          ; bundle_id
    * NonemptyText => any,
    * uint => any,
}

NonemptyText = tstr .ne ""
```

`section_table` describes `JanexFile.sections` and may be empty.

Unsigned integer keys are container mechanics. Text keys are attributes. The `janex.` prefix is
reserved. Third-party keys should use a reverse-domain prefix such as `org.example.`. Attributes are
descriptive unless their definitions assign operational semantics. Unknown text keys may be ignored.
Writers should omit unused keys.

`default_application` is the application section used when the caller does not name one. It must
refer to an existing `Application` section. The reference is invalid if the section is missing or has
a different type.

The caller may select an application by `ApplicationObject.id`. Otherwise the launcher uses
`default_application`. If the key is omitted and the file contains exactly one application section,
that section is the default. If the file contains several application sections and none is selected,
the launcher must reject the ambiguity.

`bundle_id` is the stable identity of an installable Janex bundle. It must be globally unique and
should use a reverse-domain name such as `org.example.tools`. Local execution does not require it.
Installation identifies a launch target by the pair (`bundle_id`, `ApplicationObject.id`).

#### Metadata Evolution and Extensions

Core maps use unsigned integer keys for container mechanics. Text-keyed metadata uses non-empty keys;
`janex.` is reserved. Third-party keys should use a reverse-domain prefix such as `org.example.`.

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
    ? 3: ChecksumObject,             ; checksum
    * uint => any,
}
```

For sections with a magic number, `section_type` normally equals that number. `id` must be unique
within the file across all section types. IDs may be sparse and carry no ordering or semantic
meaning.

`length` is the exact encoded section length. When present, `checksum` covers those bytes and must be
verified.

Other keys are defined by `section_type`.

#### Section References

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

#### `ExternalRegionObject`

```cddl
ExternalRegionObject = {
    0: uint,                         ; size
    ? 1: ChecksumObject,             ; checksum
    * uint => any,
}
```

Omission leaves the region unconstrained. Otherwise, `size` must match, and `checksum` must be verified
when present. A size of zero requires the region to be absent.

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

##### Verification Policy

Authentication requires OpenPGP or CMS and successful validation under the declared type. These
variants use caller-provided signer, trust, algorithm, time, and revocation policies.

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

Each required signer must pass digest, signature, identity, and caller-policy validation. Caller policy
establishes trust for embedded certificates and revocation data and selects the required signers. Only
signed attributes contribute to primary signature validation.

##### Authenticated Content Scope

Full-container authentication includes both `external_header` and `external_tail` and records a secure
checksum for every section and every nonempty external region. Readers must verify all of them. Secure
checksum algorithms are `SHA256`, `SHA512`, and `SM3`.

The signature authenticates `verification_input`; the recorded secure checksums authenticate section
and external-region bytes.

### `Application` Section

A file may contain any number of `Application` sections. Each section is one independently launchable
target. Multiple targets may share blobs. The section body after the magic number is one deterministic
CBOR `ApplicationObject`. A file with no application section has no launch target.

Separate commands with independent launch configurations use separate application sections.
Subcommands interpreted by one program are ordinary application arguments.

```rust
struct ApplicationSection {
    magic_number: u64, // 0x5050_4158_454e_414a ("JANEXAPP")

    /// One deterministic CBOR `ApplicationObject`.
    application: CborMap, // ApplicationObject
}
```

`application` occupies the remainder of the section.

#### `ApplicationObject`

```cddl
ApplicationObject = {
    0: NonemptyText,                            ; id
    1: NonemptyText,                            ; application_type
    2: ApplicationDescriptorObject,            ; descriptor
    ? 3: NonemptyText,                          ; name
    ? 4: tstr,                                  ; version
    ? 5: tstr,                                  ; comment
    ? 6: ApplicationIntegrationObject,          ; integration
    ? 7: ApplicationLaunchMode,                 ; launch_mode
    * uint => any,
}

ApplicationDescriptorObject = { * uint => any }

ApplicationLaunchMode =
    0                                           ; console
  / 1                                           ; windowed
```

`id` identifies the launch target within the file and must be unique among its application sections.
The same logical target should retain its `id` across files with the same `bundle_id`.
`SectionInfoObject.id` locates the section and is not part of the installed target identity.
`application_type` selects the schema and semantics of `descriptor`; this document defines
`janex.java`, and reserves the `janex.` prefix. Third-party types use a reverse-domain name.
`descriptor` must follow the selected schema. `name` is a display name. `version` is the application's
own version string. `comment` is a short description. `launch_mode` defaults to `console`. `windowed`
suppresses console window creation on platforms that distinguish the two modes.

Unsupported application types may be displayed and preserved but cannot be launched.

#### `ApplicationIntegrationObject`

```cddl
ApplicationIntegrationObject = {
    ? 0: NonemptyText,                          ; path_command
    ? 1: bool,                                  ; desktop_launcher
    ? 2: [+ ApplicationIconObject],             ; icons
    * uint => any,
}
```

These fields request installation integration. The Host applies them according to local policy.

`path_command` requests a command with that name on `PATH`. It is a command name, not a path, and must
be valid as a single file name on the target platform. An omitted value makes no request.

`desktop_launcher` asks the installer to create a Start Menu entry on Windows, a `.desktop` entry on
Linux, or an equivalent launcher on other platforms. An omitted value or `false` makes no request.

`icons` supplies image blobs for those launchers. The Host selects a suitable image for the
platform. An empty array should be omitted.

#### `ApplicationIconObject`

```cddl
ApplicationIconObject = {
    0: tstr,                                    ; media_type
    1: BlobRefObject,                           ; image
    * uint => any,
}
```

`media_type` is a non-empty IANA media type such as `image/png`, `image/jpeg`,
`image/vnd.microsoft.icon`, or `image/icns`. Unknown types may be ignored. `image` names a blob whose
resolved bytes are the image.

#### `janex.java` Application Descriptor

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
    * uint => any,
}
```

An omitted `condition` is unconditional. `ConditionObject` is defined below.

The launcher visits the root configuration and its `overlays` in depth-first pre-order. Each matching
object contributes as follows:

- missing keys make no contribution;
- `entry_point` replaces the current value, while `null` clears it;
- arrays append to `module_path`, `class_path`, `agents`, or `jvm_options`, while `null` clears that
  list; and
- `overlays` preserves array order and must not be `null`.

The resulting configuration must contain an `entry_point`.

#### `JavaEntryPointObject`

```cddl
JavaEntryPointObject =
    {
        0: 0,                                    ; class
        1: NonemptyText,                         ; main_class
        * uint => any,
    }
  / {
        0: 1,                                    ; module
        1: NonemptyText,                         ; main_module
        ? 2: NonemptyText,                       ; main_class
        * uint => any,
    }
```

The class variant launches `main_class`. The module variant launches `main_module`; when
`main_class` is omitted, the module supplies its main class.

#### `ConditionObject`

```cddl
ConditionObject = {
    ? 0: tstr,                                  ; java
    ? 1: (tstr / [+ tstr]),                     ; os
    ? 2: (tstr / [+ tstr]),                     ; arch
    ? 3: tstr,                                  ; vendor
    ? 4: (tstr / [+ tstr]),                     ; invocation
    * uint => any,
}
```

An empty `ConditionObject` is unconditional.

`java` is a VERS whose type is `jep322`, as defined in [Version Ranges](#version-ranges). It is
matched against the candidate runtime's version string, including Java 8 alias expansion. `vendor` is
an exact match against the candidate runtime's vendor string. This document does not normalize vendor
strings. `os` is matched against the host operating system. `arch` is matched against the host CPU.
The normalized operating-system names are `linux`, `windows`, and `macos`. The normalized CPU names
are `x86`, `x86-64`, and `aarch64`. Other names match only by exact equality.

`invocation` is the channel that started this launch. The defined values are `run` for
`janex run` of a Janex file, `open` for a file-association or double-click open, and `command` for
an installed `PATH` command. It is not a windowing mode: a graphical application started with
`janex run` still has invocation `run`.

A text `os`, `arch`, or `invocation` value matches that one name. An array matches if the candidate
equals any element. An omitted key imposes no constraint.

A condition matches a candidate Java runtime, the current host, and the current invocation when every
present constraint matches. An invalid `java` VERS makes the application descriptor invalid; it is
not treated as a non-match. An unknown `invocation` token does not match.

The launcher considers each candidate runtime against the root condition. A candidate that does not
match is discarded. Among remaining candidates, the launcher walks the root configuration and its
`overlays` in depth-first pre-order and applies each overlay whose condition matches. It then selects
a remaining candidate using the implementation's runtime selection order.

#### `JavaPathEntryObject`

```cddl
JavaPathEntryObject =
    {
        0: 0,                    ; local
        1: BlobRefObject,        ; resource_root
        * uint => any,
    }
  / {
        0: 1,                    ; remote
        1: tstr,                 ; purl
        2: ChecksumObject,       ; checksum
        * uint => any,
    }
```

Variant `0` selects a blob in this file whose resolved representation is one `ResourceRoot`, as
defined in [Resource Roots](#resource-roots). The blob is named by a `BlobRefObject`, defined in
[Blob References](#blob-references). Variant `1` resolves exactly the package identified by
a canonical Package URL and verifies it by `checksum`. A `vers` qualifier is not allowed; Maven
artifacts use `pkg:maven` and may select a repository with `repository_url`.

#### `JavaAgentObject`

```cddl
JavaAgentObject = {
    0: JavaPathEntryObject,      ; reference
    1: tstr,                     ; option
    * uint => any,
}
```

An empty `option` means that no agent option is supplied.

### Resource Roots

A local `JavaPathEntryObject` names one blob with a `BlobRefObject`. Resolving that blob must produce
exactly one `ResourceRoot` and consume every resolved byte. Multiple path entries may name the same
resource-root blob.

A `ResourceRoot` is one logical tree built from ordered layers. Each layer has a `ConditionObject`.
After a Java runtime is selected, the launcher evaluates every layer against that runtime and the
host. Matching layers are applied from first to last. Later layers override earlier values at the
same path. The merged tree is one classpath or module-path entry. It is not a list of separate
entries.

```rust
struct ResourceRoot {
    /// The string pool used by paths in this root and by `CLASSFILE` transforms in this root.
    string_pool: BlobRef,

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

The metadata map may be empty. An invalid `java` VERS
in a layer condition makes the resource root invalid. Readers must resolve `string_pool` before
interpreting directory paths, entry names, or symbolic-link targets. Multiple resource roots may
name the same string-pool blob. Directory paths are unique within a layer, not across layers.

A Multi-Release JAR is an import mapping, not the native layout. The base tree becomes an
unconditional layer. Each `META-INF/versions/N/` tree becomes a layer whose `java` constraint is
`vers:jep322/>=N`. Those layers must appear in increasing `N` so that a later matching layer
overrides an earlier one. Export to a JAR may reconstruct `META-INF/versions/` only when every
layer condition is a Java feature range and does not constrain `os`, `arch`, or `vendor`.

##### String Pools

A `ResourceRoot` names one string-pool blob with a `BlobRef`. Resolving that blob must produce exactly
one `StringPoolData` and consume every resolved byte.

```rust
/// A zero-based index into `StringPoolData.strings`.
type StringPoolIndex = vuint;

struct StringPoolData {
    /// Distinct interned strings in pool-index order.
    strings: Vec<String>,
}
```

The pool is an intern table. Each string must appear at most once. A `StringPoolIndex` must select an
existing element. Index `0` has no special meaning. The empty string appears only when something
refers to it.

Directory paths, entry names, and symbolic-link targets are `StringPoolIndex` values. The root
directory is the record whose resolved path is empty. All other path and name rules apply to the
resolved UTF-8 strings.

`CLASSFILE` transforms in the resource root use this same pool. Strings are UTF-8. Restoring a class
file converts selected strings to Modified UTF-8.

##### `ResourceDirectory`

```rust
#[repr(u8)]
enum DirectoryOp {
    /// Merge `entries` into this directory.
    Merge = 0,

    /// Discard earlier direct entries in this directory, then apply `entries`.
    Replace = 1,

    /// Discard this directory and all descendant paths. `entries_count` must be `0`.
    Remove = 2,
}

struct ResourceDirectory {
    /// The directory path relative to the resource root, as an index into the root's string pool.
    path: StringPoolIndex,

    /// How this record updates the merged tree.
    op: DirectoryOp,

    /// One deterministic CBOR resource-metadata map.
    metadata: Sized<CborMap>, // ResourceMetadataObject

    /// The number of direct entries in this directory.
    entries_count: vuint,

    /// The direct entries in name order.
    entries: Content<[DirectoryEntry; entries_count]>,
}
```

The empty path identifies the root directory. Other directory paths are UTF-8, `/`-separated, and
must not start or end with `/` or contain empty, `.` or `..` components.
Directory paths are unique within a layer and sorted by the UTF-8 bytes of the resolved strings.
Parent directories may be implicit; an empty directory or its metadata is preserved by an explicit
`Merge` or `Replace` record with no entries.

When applying a matching layer, directory records are applied in path order. `Remove` deletes the
directory path and every descendant path contributed by earlier layers. `Replace` deletes earlier
direct entries of this directory and does not delete descendant directories. `Merge` keeps earlier
entries and then applies `entries`. A later file or symbolic link with the same name replaces the
earlier one. A tombstone removes an earlier file or symbolic link with that name. A tombstone for a
name that is not present is ignored.

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

        /// The file name within the directory, as an index into the root's string pool.
        name: StringPoolIndex,

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

        /// The symbolic-link name within the directory, as an index into the root's string pool.
        name: StringPoolIndex,

        /// The relative target path, as an index into the root's string pool.
        target: StringPoolIndex,

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
        name: StringPoolIndex,
    },
}
```

Entry names are nonempty UTF-8 strings without `/` and must not be `.` or `..`. They are unique within
their directory, including tombstones, and sorted by the UTF-8 bytes of the resolved strings. A full
resource path is the resolved entry name for the root directory, or `directory_path + "/" +
entry_name` otherwise, using the resolved strings. Directory records and file or symbolic-link
entries must not produce conflicting paths.

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
    /// A Java class-file transform using a shared `StringPool`.
    CLASSFILE = 1,
}
```

```cddl
ContentTransformPropertiesObject = { * uint => any }
ClassFileTransformPropertiesObject = { * uint => any }
```

The schema of `properties.value` is selected by `method`. `CLASSFILE` currently defines no
properties. It uses the string pool named by the containing `ResourceRoot`.

Transforms are stored in encoding order and reversed from last to first after resolving the source.
Each result must match `input_size`; the final value must be a valid encoding of `T`. An empty
transform array means the source already encodes `T`. Unsupported methods are invalid.

`CLASSFILE` is valid only for regular-file `Content<[u8]>`. The transform array of
`ResourceDirectory.entries` must therefore be empty.

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

            /// The index of the string in the containing resource root's string pool.
            string_pool_index: StringPoolIndex,
        }
        ```

    2. `CONSTANT_External_String_Class`:

        ```rust
        struct CONSTANT_External_String {
            tag: u8, // 0xFE

            /// The index of the package name in the containing resource root's string pool.
            package_name_index: StringPoolIndex,

            /// The index of the class name in the containing resource root's string pool.
            class_name_index: StringPoolIndex,
        }
        ```

The input must be a valid Java class file. External string indices select entries in the containing
resource root's string pool.

### `BlobPool` Section

A `BlobPool` stores logical byte blobs. A blob is either independently stored or assembled from
decoded ranges of independently stored blobs. Containing structures assign their logical types. A
file may contain any number of pools.

```rust
struct BlobPoolSection {
    magic_number: u64, // 0x4c4f_4f50_424f_4c42 ("BLOBPOOL")

    /// Stored blob bytes and blob-table pages.
    bytes: [u8; ...],
}
```

#### Stored Blob Encoding

Stored-blob and table-page encodings use these CBOR objects:

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
    ? checksum: ChecksumObject,
]

BlobTablePageObject = [* BlobInfoObject]

BlobInfoObject =
    [
        0,                              ; stored
        offset: uint,
        encoding: BlobEncodingObject,
    ]
  / [
        1,                              ; extents
        extents: [+ BlobExtentObject],
    ]

BlobExtentObject = [
    stored_blob_index: uint,
    decoded_offset: uint,
    decoded_length: uint,
]
```

`BlobPoolSection.bytes` has length `SectionInfoObject.length - 8`. `blob_count` and all offsets and
sizes must fit in `u64`. `blob_count` includes stored and extents entries. An empty pool has no pages.
Otherwise, the page count is
`1 + (blob_count - 1) / page_capacity`. Every page except the last contains `page_capacity` entries;
the final page contains the remaining entries.

For `blob_index`, the page index is `blob_index / page_capacity` and the index within that page is
`blob_index % page_capacity`. The logical blob table is the concatenation of the pages in directory
order.

Each page descriptor locates a stored page relative to `BlobPoolSection.bytes`. Decoding the page must
produce one deterministic CBOR `BlobTablePageObject`. When present, its checksum covers those decoded
CBOR bytes and must be verified. Each page decodes independently using self-contained filters.
`BlobRef` addresses logical blobs. Locating a stored blob requires only its selected table page;
resolving an extents blob may also require the pages containing its stored sources.

For a stored entry, `offset` locates its encoded bytes relative to `BlobPoolSection.bytes`. All page
and stored-blob ranges must fit in `bytes` and must not overlap. Writers may place them in any order.

An extents entry concatenates its decoded ranges in array order. Each `stored_blob_index` must select
a stored entry in the same pool. Each range must be nonempty and fit in that entry's decoded bytes.
Extents cannot refer to other extents entries.

#### Blob References

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

Whether a blob is stored directly or assembled from extents is writer policy. Readers must support
both forms. Writers may store frequently accessed structural blobs directly for better locality.

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
library requirements, may use other registered types such as `maven`. Remote `JavaPathEntryObject`
values remain exact Package URLs and must not carry a `vers` qualifier.

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

Writers should use this canonical form in a VERS. `8` means any Java 8 release. `8.0.402` means
update 402 of Java 8.

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
constraint timeline, using this comparison. The VERS `type` in a Java launch condition must be
`jep322`.

```text
vers:jep322/>=17.0.10|<18|>=21.0.2|<22
```

This range contains Java 17 starting at 17.0.10, and Java 21 starting at 21.0.2. It does not contain
Java 18, Java 22, or `17-ea`.

```text
vers:jep322/>=8.0.402|<9
```

This range contains Java 8 starting at update 402. A candidate that reports `1.8.0_402` matches.

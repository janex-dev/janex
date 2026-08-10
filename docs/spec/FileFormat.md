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

Janex uses little-endian encoding for all integer and floating-point numbers.

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
zero-padded ULEB128 representations within the ten-byte width limit. Janex does not currently use signed
LEB128 (SLEB128).

Reading `vuint` follows this algorithm:

```rust
fn read_vuint(read: &mut impl Read) -> Result<vuint, Error> {
    let first = read.read_u8()?;

    if first < 0x80 {
        return Ok(first as u64);
    }

    let mut result = (first & 0x7F) as u64;

    for i in 1..10 {
        let byte = read.read_u8()?;
        let low_bits = byte & 0x7F;

        // the 10th byte can have at most 1 valid bit
        if i == 9 && low_bits > 1 {
            return Err(Error::InvalidVUInt);
        }

        result |= (low_bits as u64) << (7 * i);

        if byte == low_bits {
            return Ok(result);
        }
    }

    Err(Error::InvalidVUInt)
}
```

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

    /// File-level flags. Currently unused and must be `0`.
    flags: u64,

    /// Records the length and other information of each section.
    section_table: Vec<SectionInfo>,

    /// Currently, all fields will be skipped. Reserved for future use.
    fields: Vec<TaggedPayload<u32>>,
    
    /// The verification information.
    verification_info: VerificationInfo,
    
    /// The end mark of the file.
    ///
    /// When reading a Janex file, the tool first locates the `end_mark`, 
    /// and then uses `metadata_length` to reverse-lookup the offset of `FileMetadata`,
    /// and uses `file_length` to reverse-lookup the starting offset of the `JanexFile` structure.
    end_mark: u64,  // 0x444e_4558_454e_414a ("JANEXEND")
    
    /// The length in bytes of the metadata section.
    metadata_length: u64,

    /// The total length in bytes of the `JanexFile` structure.
    ///
    /// The reader uses this value together with the actual file size to determine
    /// the byte offset at which the Janex content begins.
    file_length: u64,
}
```

#### `SectionInfo` Structure

The structure of the `SectionInfo` is as follows:

```rust
struct SectionInfo {
    /// The type of a section.
    ///
    /// Generally, `section_type` is the same as the `magic_number` of the section content (if the section has a `magic_number`).
    section_type: SectionType,
    
    /// The ID of a section.
    /// 
    /// Sections with the same `section_type` must have different IDs; two `section_type`s can have the same ID.
    id: vuint,
    
    /// Options related to the section.
    options: Vec<TaggedPayload<u32>>,
    
    /// The length in bytes of the section content.
    length: vuint,
    
    /// The checksum of the section content.
    checksum: Checksum,
}
```

Currently supported section types:

```rust
#[repr(u64)]
enum SectionType {
    /// Represents arbitrary data in a Janex file. Janex tools will not use these sections.
    ///
    /// Padding sections do not require a `magic_number`.
    Padding = 0x0047_4e49_4444_4150, // "PADDING\0"

    /// A special section whose content is not within `sections`, but before the `JanexFile` structure.
    ///
    /// This is an optional section. If present, the `SectionInfo` must be the first element in `section_table`.
    ExternalHeader = 0x4441_4548_4c54_5845, // "EXTLHEAD"

    /// A special section whose content is not within `sections`, but after the `JanexFile` structure.
    ///
    /// This section allows users to attach a Janex Launcher packaged as a JAR to the end of the Janex file,
    /// enabling program startup using `java -jar xxx.janex`.
    /// The JAR-formatted Janex Launcher should be able to read the size of the JAR portion and locate the end of the `JanexFile` structure.
    /// 
    /// This is an optional section. If present, the `SectionInfo` must be the last element in `section_table`.
    ExternalTail = 0x4c49_4154_4c54_5845, // "EXTLTAIL"
    
    /// The `FileMetadata` section. Used to store file metadata.
    /// 
    /// This section is always the last section in `sections`.
    /// 
    /// It will not be recorded in `section_table`,
    /// because `section_table` is inside this section, and attempting to record it in `section_table` would create a self-referential problem.
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

Except for the `ExternalHeader` and `ExternalTail` sections, all other sections are arranged consecutively within the `sections` of `JanexFile`.
If additional data or padding needs to be inserted within them, the `Padding` section can be used.

Unknown sections will be ignored.

#### `VerificationInfo` Structure

The structure of the `VerificationInfo` is as follows:

```rust
struct VerificationInfo {
    verification_type: VerificationType,
    data: Vec<u8>,
}
```

#### `VerificationType` Structure

The supported verification types are:

```rust
#[repr(u8)]
enum VerificationType {
    /// No verification.
    /// 
    /// The data field is empty.
    None = 0,
    
    /// Checksum verification.
    /// 
    /// The data field contains a `Checksum` for the `FileMetadataSection` structure (ignoring the `verification_info`, `end_mark`, `metadata_length`, and `file_length` fields).
    Checksum = 1,
    
    /// OpenPGP signature verification.
    /// 
    /// The data field contains an OpenPGP signature for the `FileMetadata` section (ignoring the `verification_info`, `end_mark`, `metadata_length`, and `file_length` fields).
    OpenPGP = 2,
    
    /// CMS signature verification.
    /// 
    /// The data field contains a CMS signature for the `FileMetadata` section (ignoring the `verification_info`, `end_mark`, `metadata_length`, and `file_length` fields).
    CMS = 3,
}

```

If you want to sign or verify the integrity of the entire file, you should ensure that each element
in the `section_table` contains a valid and secure `checksum`.

### `Attributes` Section

```rust
struct AttributesSection {
    /// The magic number identifying this as a attributes section.
    ///
    /// Always `0x2e53_4249_5254_5441` ("ATTRIBS.").
    magic_number: u64, // 0x2e53_4249_5254_5441 ("ATTRIBS.")
    
    /// The list of attributes.
    attributes: Vec<Attribute>,
}
```

In the future, we may use it to record author names and other information.

#### `Attribute` Structure

```rust
struct Attribute {
    /// The name of the attribute.
    name: String,
    
    /// The value of the attribute.
    /// 
    /// This `Vec<u8>` can actually be interpreted as a `String` (They have the same binary representation),
    /// or it may carry arbitrary binary data.
    value: Vec<u8>,
}
```

### `RootConfigGroup` Section

The structure of the `RootConfigGroup` is as follows:

```rust
struct RootConfigGroupSection {
    /// The magic number identifying the `RootConfigGroup` section.
    /// 
    /// Always `0x5055_4f52_4747_4643` ("CFGGROUP").
    magic_number: u64, // 0x5055_4f52_4747_4643 "CFGGROUP"
    
    /// The root config group.
    root_group: ConfigGroup,
}
```

#### `ConfigGroup` Structure

A `ConfigGroup` is a logical grouping of configuration fields.

Groups may be nested via `SubGroups` fields, forming a configuration tree.

Each group can carry an optional `Condition` field.
For the root group, the `condition` is used to detect whether the Java runtime and platform environment are suitable
for this program, and to select the optimal Java runtime based on this;
For subgroups, the `condition` is used to determine whether the group is applicable to the current environment,
and if so, apply its configuration.

This design allows the launcher to express conditional configurations such as
"add this JVM option only when running on Java 21 or newer" or
"use this native library path only on Linux/aarch64".

When launching a program, the Janex Launcher starts from the root group and traverses all subgroups in a depth-first order, 
applying configurations that meet the conditions.

When applying configurations, if the value type of `ConfigField` is `Vec`,
all matching `ConfigGroup` fields will be aggregated into a single `Vec`,
with the element order consistent with the traversal order;  
If the value type of `ConfigField` is another type, the field of the last matching `ConfigGroup` will be applied to the configuration.


```rust
struct ConfigGroup {
    /// The magic number identifying this as a configuration group.
    ///
    /// Always `0x50524743` ("CGRP").
    magic_number: u32, // 0x50524743 ("CGRP")

    /// The list of configuration fields contained in this group.
    fields: Vec<ConfigField>,
}
```

#### `ConfigField`

Configuration fields carry the actual launch settings within a `ConfigGroup`.

Each field begins with a 4-byte type tag followed by a length-prefixed payload.

Readers must skip unknown field types to allow forward compatibility.

Supported fields:

```rust
#[repr(TaggedPayload<u32>)]
enum ConfigField {
    /// A CEL condition expression that guards the enclosing `ConfigGroup`.
    ///
    /// See the [Conditions](#conditions) section for details.
    Condition {
        field_type: u32, // 0x444e4f43 ("COND")

        /// The CEL expression string. Must evaluate to `bool` or `int`.
        condition: String,
    },

    /// The fully qualified binary name of the application's main class.
    MainClass {
        field_type: u32, // 0x534c434d ("MCLS")

        /// The fully qualified binary name of the main class (e.g., `"com.example.Main"`).
        value: String,
    },

    /// The name of the application's main module.
    ///
    /// Passed to the JVM via `--module` when launching with the Java module system.
    MainModule {
        field_type: u32, // 0x444f4d4d ("MMOD")

        /// The main module name.
        value: String,
    },

    /// The ordered list of resource groups to place on the module path (`--module-path`).
    ModulePath {
        field_type: u32, // 0x50444f4d ("MODP")

        /// The number of bytes of the items.
        payload_bytes: vuint,

        /// The resource group references to add to the module path, in order.
        items: Vec<ResourceGroupReference>,
    },

    /// The ordered list of resource groups to place on the class path (`-classpath`).
    ClassPath {
        field_type: u32, // 0x50534c43 ("CLSP")

        /// The number of bytes of the items.
        payload_bytes: vuint,

        /// The resource group references to add to the class path, in order.
        items: Vec<ResourceGroupReference>,
    },

    /// The list of resource groups to load as Java agents (`-javaagent`).
    Agents {
        field_type: u32, // 0x544e4741 ("AGNT")

        /// The number of bytes of the items.
        payload_bytes: vuint,

        /// The agents.
        items: Vec<JavaAgent>,
    },

    /// A list of additional JVM options to pass when launching the application.
    ///
    /// Each element is a single JVM option string
    /// (e.g., `"--add-exports=java.base/sun.nio.ch=ALL-UNNAMED"` or `"-Xmx512m"`).
    JvmOptions {
        field_type: u32, // 0x54504f4a ("JOPT")

        /// The number of bytes of the options.
        payload_bytes: vuint,

        /// The list of JVM option strings, each passed as a separate argument to the JVM.
        options: Vec<String>
    },

    /// A list of nested `ConfigGroup` entries within the enclosing group.
    ///
    /// Each subgroup may carry its own `Condition`, enabling fine-grained conditional configuration.
    /// The launcher evaluates each subgroup independently and applies those whose conditions are satisfied.
    SubGroups {
        field_type: u32, // 0x50524753 ("SGRP")

        /// The number of bytes of the subgroups.
        payload_bytes: vuint,

        /// The list of nested configuration groups.
        subgroups: Vec<ConfigGroup>
    }
}
```

#### `ResourceGroupReference` Structure

A `ResourceGroupReference` identifies a resource group to be placed on the class path, module path,
or agent list.

It is either a reference to a resource group embedded in the Janex file itself or a reference
to an external package that is resolved and downloaded at launch time.

```rust
enum ResourceGroupReference {
    /// A reference to a resource group embedded in this Janex file.
    Local {
        /// The reference type tag for this variant.
        ref_type: u32, // 0x00434f4c ("LOC\0")

        /// The name of the local resource group, matching the `name` field of a `ResourceGroup`
        /// declared in the `ResourceGroups` section.
        group_name: String,
    },

    /// A reference to a package hosted in a remote repository.
    ///
    /// The package is not embedded in the Janex file. The Janex Launcher resolves and downloads
    /// it at launch time (if not already present in a local cache) before starting the JVM.
    ///
    /// At runtime, only the specified package is downloaded, and dependencies are not resolved.
    /// If this package depends on other packages, those dependencies should also be recorded.
    Purl {
        /// The reference type tag for this variant.
        ref_type: u32, // 0x4c525550 ("PURL")

        /// The canonical Package URL of the remote package.
        ///
        /// Maven artifacts should use the `pkg:maven` type. A runtime dependency must identify
        /// a single package version, so the PURL must include a concrete version and must not use
        /// a `vers` qualifier. For non-default Maven repositories, use the `repository_url`
        /// qualifier in the PURL.
        ///
        /// Examples:
        ///
        /// - `pkg:maven/org.slf4j/slf4j-api@2.0.9`
        /// - `pkg:maven/org.apache.xmlgraphics/batik-anim@1.9.1?type=pom`
        /// - `pkg:maven/net.sf.jacob-projec/jacob@1.14.3?classifier=x64&type=dll`
        purl: String,

        /// The expected checksum of the downloaded package, used to verify the integrity of the download.
        ///
        /// This checksum is part of Janex's trust policy and should not be replaced by a PURL
        /// `checksum` qualifier.
        checksum: Checksum,
    }
}
```

#### `JavaAgent` Structure

`JavaAgent` represents a Java Agent JAR and its option.

```rust
struct JavaAgent {
    /// The resource group reference for the Java agent JAR.
    reference: ResourceGroupReference,
    
    /// The agent option string passed to the JVM via `-javaagent`.
    option: String,
}
```

### `ResourceGroups` Section

`ResourceGroups` contains all embedded resource groups in the Janex file. 

Each resource group is a logical container of related files, typically corresponding to a single JAR or module from the original Java project.

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

    /// The unique name of this resource group within the `ResourceGroups` entry.
    /// 
    /// This name is referenced by `ResourceGroupReference::Local` in the launcher configuration
    /// to add this group to the class path, module path, or agent list.
    name: String,

    /// Currently, all fields will be skipped. Reserved for future use.
    fields: Vec<TaggedPayload<u32>>, 

    /// The number of `Resource` entries stored in this group.
    resources_count: vuint,

    /// A reference to the compressed array of resource metadata entries for this group.
    ///
    /// Writers normally place this structural data in a blob pool separate from file contents.
    resources: BlobRef<[Resource; resources_count]>
}
```

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

The input to the `ClassFile` transform must be a valid Java class file. A Janex file containing a
`ClassFile` transform must contain exactly one `StringPool` section, and reversing the transform uses
that string pool. The `properties` field of `CLASSFILE` is currently empty.

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
    /// Requires a `StringPool` section to be present in the Janex file.
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

Each Janex file may contain at most one `StringPool` section.
When present, it must appear before the `ResourceGroups` section.

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
`BlobRef`. Each blob pool is identified by the `id` of its corresponding `SectionInfo`. Blob-pool IDs
must be unique within the `BlobPool` section type, as required for all section IDs, and carry no
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

The length of `bytes` is `SectionInfo.length - 8`; therefore, a `BlobPool` section must have a length
of at least 8 bytes. The corresponding `SectionInfo.options` must contain exactly one
`BlobPoolIndex` option:

```rust
#[repr(TaggedPayload<u32>)]
struct BlobPoolIndex {
    option_type: u32, // 0x5844_4942 ("BIDX")

    /// The number of bytes in this option payload.
    payload_bytes: vuint,

    /// The byte offset of the stored blob table relative to the first byte of
    /// `BlobPoolSection.bytes`.
    offset: vuint,

    /// The encoding metadata for the stored blob table.
    encoding: BlobEncoding,

    /// The checksum of the decoded `BlobTable` representation.
    checksum: Checksum,
}

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
`[BlobPoolIndex.offset, BlobPoolIndex.offset + BlobPoolIndex.encoding.stored_size)`. Reversing its
filter chain must produce a valid `BlobTable` and, unless the checksum algorithm is `NONE`, the decoded
representation must match `BlobPoolIndex.checksum`. Blob-table filters must be self-contained and must
not depend on a `BlobRef` or other external data because the table is bootstrap metadata.

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
    /// The ID of the referenced `BlobPool` section.
    ///
    /// This value must match the `SectionInfo.id` of exactly one `BlobPool` section.
    blob_pool_id: vuint,

    /// The zero-based index of the blob in the referenced blob pool's `BlobTable`.
    blob_index: vuint,
}
```

The selected `BlobInfo` supplies the location and encoding metadata of the stored representation.
The `blob_index` must be less than the number of entries in the selected `BlobTable`. After decoding,
the complete result must be a valid encoding of `T`. Multiple references may identify the same blob.

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

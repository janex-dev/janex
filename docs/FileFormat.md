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
- **Remote dependencies**: Janex files can declare dependencies on JARs from external sources (such as Maven
  repositories). These dependencies are not bundled in the Janex file but are resolved and downloaded on demand
  before the program starts.
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

Janex uses `vuint` to efficiently encode unsigned integers in some structures.

```rust
type vuint = u64;
```

A `vuint` is stored in the file as one or more bytes. The lower seven bits of each byte carry integer
data, and the most significant bit (MSB) of each byte serves as the continuation flag:

- If the MSB is `0`, the current byte is the last byte of the integer.
- If the MSB is `1`, more bytes follow; the next byte continues the encoding.

Reading `vuint` should follow the following algorithm:

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

Janex uses a 96-bit high-precision timestamp capable of representing nanosecond-level accuracy.
The timestamp is measured relative to the Unix epoch (`1970-01-01T00:00:00Z`):

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

### Compression

Janex uses the following structure to represent compression metadata. It describes how a block of data
was compressed and provides the information needed to decompress it:

```rust
struct CompressInfo {
    /// The compression method used to compress the data.
    method: CompressMethod,

    /// The size of the original (uncompressed) data, in bytes.
    uncompressed_size: vuint,

    /// The size of the compressed data, in bytes.
    compressed_size: vuint,

    /// Optional method-specific parameters passed to the decompressor.
    /// The interpretation of this field depends on the value of `method`.
    options: Vec<u8>,
}
```

The supported compression methods are:

```rust
#[repr(vuint)]
enum CompressMethod {
    /// No compression. The data is stored as-is.
    NONE = 0,

    /// Composite compression: combines multiple compression algorithms applied in sequence.
    ///
    /// The `options` field contains a `Vec<CompressInfo>` that describes each layer of compression.
    /// To decompress, the algorithms are applied in reverse order (innermost first).
    COMPOSITE = 1,

    /// A class-file-aware compression algorithm.
    ///
    /// This method extracts frequently occurring strings from Java class file constant pools
    /// and places them into a shared `StringPool`, enabling cross-file string deduplication.
    /// 
    /// The modified class files are typically then compressed further using other compression algorithms,
    /// such as Zstandard.
    CLASSFILE = 2,

    /// Zstandard (zstd) compression.
    ///
    /// See https://github.com/facebook/zstd for details.
    ZSTD = 3,
}
```

The `CompressedData<T>` structure pairs a `CompressInfo` header with its corresponding compressed byte
payload. The type parameter `T` describes the logical type of the data after decompression:

```rust
struct CompressedData<T> {
    /// The compression metadata describing the method and sizes.
    info: CompressInfo,
    
    /// The compressed bytes. After decompression, the result has type `T`.
    data: [u8; info.compressed_size],
}
```

#### Class File Compression

Janex typically extracts frequently occurring strings (such as common package names, type descriptors,
and method signatures) from class file constant pools into a shared `StringPool`, then independently
compresses both the modified class files and the string pool using Zstandard. This approach allows
strings that appear across many class files to be stored only once, significantly reducing the total
compressed size.

The `CLASSFILE` compression algorithm largely preserves the standard class file format, but introduces
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

### `Checksum`

```rust
type Checksum = String;
```

All `Checksum` values are either an empty string (representing no checksum)
or in the form `<Algorithm>:<Checksum>` (e.g., `sha256:a13180315dfd3bff164967b64a726b98c69249970dab2f5a642c733582345885`).

Currently supported cryptographically secure hash algorithms:

- `sha256`: The SHA-256 hash algorithm.
- `sha512`: The SHA-512 hash algorithm.

Currently supported non-cryptographically secure hash algorithms (for integrity verification only):

- `xxh64`: The [XXH64](https://github.com/Cyan4973/xxHash) hash algorithm.

## Janex File Structure

The Janex file is the binary format produced by the Janex build tool for packaging and distributing
Java programs as self-contained executables. Its overall layout is as follows:

```rust
struct JanexFile {
    /// The magic number identifying this as a Janex file.
    ///
    /// Always `0x5050_4158_454e_414a` ("JANEXAPP").
    magic_number: u64, // 0x50504158454e414a  ("JANEXAPP")

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
    /// Readers should accept files with a higher minor version within the same major version,
    /// ignoring any unknown fields or entries.
    ///
    /// The only exception is when `major_version` is `0` (indicating an early development stage),
    /// where we will update the `minor_version` for every breaking change,
    /// and Janex tools should reject files with mismatched `minor_version`.
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

    /// The total length in bytes of the file.
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
    options: Vec<u8>,
    
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
    
    /// Stores the raw resource data after compression.
    DataPool = 0x4c4f_4f50_4154_4144, // "DATAPOOL"
    
    /// The `RootConfigGroup` section.
    RootConfigGroup = 0x5055_4f52_4747_4643, // "CFGGROUP"

    /// The `ResourceGroups` section. Contains all embedded resource groups.
    ResourceGroups = 0x0053_5052_4753_4552, // "RESGRPS\0"

    /// The `StringPool` section. A shared string pool used by class file compression
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
    verification_type: String,
    data: Vec<u8>,
}
```

The supported verification types are:

- `Checksum`: `data` is actually a `Checksum`, which is calculated based on the bytes from the start of the `FileMetadataSection` structure 
  up to the `verification_info` field (i.e., ignoring the `verification_info`, `end_mark`, `metadata_length`, and `file_length` fields).
- `OpenPGP`: OpenPGP signature for the `FileMetadata` section (ignoring the `verification_info`, `end_mark`, `metadata_length`, and `file_length` fields).
- `CMS`: CMS signature for the `FileMetadata` section (ignoring the `verification_info`, `end_mark`, `metadata_length`, and `file_length` fields).

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

        /// The resource group references for Java agent JARs, in the order they are attached.
        items: Vec<ResourceGroupReference>,
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

#### `ResourceGroupReference`

A `ResourceGroupReference` identifies a resource group to be placed on the class path, module path,
or agent list.

It is either a reference to a resource group embedded in the Janex file itself or a reference
to an external Maven artifact that is resolved and downloaded at launch time.

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

    /// A reference to a Maven artifact hosted in a remote repository.
    ///
    /// The artifact is not embedded in the Janex file. The Janex Launcher resolves and downloads
    /// it at launch time (if not already present in a local cache) before starting the JVM.
    ///
    /// At runtime, only the specified artifact is downloaded, and dependencies are not resolved. 
    /// If this artifact depends on other artifacts, those dependencies should also be recorded.
    Maven {
        /// The reference type tag for this variant.
        ref_type: u32, // 0x00564147 ("GAV\0")

        /// The Maven coordinates of the artifact in `groupId:artifactId:version` format
        /// (e.g., `"org.slf4j:slf4j-api:2.0.9"`).
        gav: String,

        /// The base URL of the Maven repository from which to download the artifact
        /// (e.g., `"https://repo1.maven.org/maven2"`).
        /// 
        /// Defaults to `https://repo1.maven.org/maven2`.
        repository: String,

        /// The expected checksum of the artifact JAR, used to verify the integrity of the download.
        checksum: Checksum,
    }
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

    /// The compressed array of resource metadata entries for this group.
    compressed_resources: CompressedData<[Resource; resources_count]>
}
```

#### `Resource`

A `Resource` represents a single entry (regular file, directory, or symbolic link) within a resource
group.

Resources contain only metadata; the actual file content bytes are stored elsewhere in the
`JanexFile` and referenced by byte offset via the `content_offset` field.

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

        /// Compression metadata for this resource's content.
        ///
        /// The `uncompressed_size` field within this structure gives the original file size in bytes.
        compress_info: CompressInfo,

        /// The byte offset of this resource's (compressed) content within the `bytes` field of the `DataPool` section.
        content_offset: vuint,

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

#### `ResourcePath`

`ResourcePath` represents the path of a `Resource`, for example `java/lang/Object.class`.

`ResourcePath` is composed of several parts separated by `/`. The `/` cannot be the first or last character, and cannot be empty.
Each part of `ResourcePath` cannot be empty, cannot be `.` or `..`, and cannot contain `/`.

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
    /// XXH64 checksum of the uncompressed resource content.
    ///
    /// Can be used by the extractor to verify data integrity after decompression.
    Checksum {
        /// The field ID for this variant.
        id: u8, // 0x01

        /// The number of bytes of the checksum payload.
        /// 
        /// Always `8` (the size of the XXH64 checksum).
        payload_bytes: vuint,

        /// The XXH64 hash of the uncompressed resource content.
        checksum: u64,
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

A shared string pool used by the class file compression algorithm and `RefBody` resource paths.

The size of the `StringPool` is at least 1, and the first string (at index 0) is always an empty string.

Each Janex file may contain at most one `StringPool` section.
When present, it must appear before the `ResourceGroups` section.

```rust
struct StringPoolSection {
    /// The magic number identifying this section as a string pool.
    /// 
    /// Always `0x004c_4f4f_5052_5453` ("STRPOOL\0")
    magic_number: u64, // 0x004c_4f4f_5052_5453 ("STRPOOL\0")

    /// The total number of strings stored in this pool.
    count: vuint,

    /// The uncompressed byte length of each string, in pool index order.
    /// Used to locate individual strings within the decompressed byte buffer.
    sizes: [vuint; count],

    /// The concatenated UTF-8 bytes of all pool strings, stored as compressed data.
    /// After decompression, individual strings are extracted sequentially using the `sizes` array.
    bytes: CompressedData<[u8]>,
}
```

All strings stored in the `StringPool` are encoded in standard UTF-8. 
When used to decompress class files, they need to be converted back to Modified UTF-8 encoding.

### `DataPool` Section

Stores the raw resource data after compression.

Currently, each file can only have one `DataPool` section.

```rust
struct DataPoolSection {
    magic_number: u64, // 0x4c4f_4f50_4154_4144 ("DATAPOOL")
    bytes: [u8; ...],
}
```

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
- If it evaluates to `int` (only for the root group), the value represents a priority score.
  The launcher selects the Java installation with the highest score.

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
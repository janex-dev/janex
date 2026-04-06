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

### String

Strings in Janex are length-prefixed and encoded in UTF-8. The `length` field stores the number of
bytes, followed by the raw UTF-8 bytes:

```rust
struct String {
    /// The number of bytes in the string.
    length: vuint,
    /// The raw UTF-8 encoded content of the string.
    data: [u8; length],
}
```

### Dynamic Array

Janex uses the following structure to store dynamically sized arrays. The `length` field specifies the
number of elements, followed by the elements themselves serialized in sequence:

```rust
struct Vec<T> {
    /// The number of elements in the array.
    length: vuint,
    /// The array elements, each serialized according to the type `T`.
    data: [T; length],
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

(TODO: More details about the class file compression algorithm)

## Janex File Structure

The Janex file is the binary format produced by the Janex build tool for packaging and distributing
Java programs as self-contained executables. Its overall layout is as follows:

```rust
struct JanexFile {
    /// The magic number identifying this as a Janex file.
    ///
    /// Always `0x0000_0058_454e_414a` ("JANEX\0\0\0").
    magic_number: u64, // 0x0000_0058_454e_414a ("JANEX\0\0\0")

    /// Raw byte storage pool for all class file and resource file contents.
    /// Individual resources reference their data within this pool by byte offset.
    data_pool: [u8; ...],

    /// Boot metadata describing the resource groups and the optional shared string pool.
    /// Consumed by the Janex Boot ClassLoader at startup.
    boot_metadata: BootMetadata,

    /// Launcher metadata describing how to locate a suitable Java runtime and construct
    /// the JVM launch command. Consumed by the Janex Launcher.
    launcher_metadata: LauncherMetadata,

    // TODO: Support for signature verification.
    signature: Option<Signature>,

    /// End-of-file marker used to locate other sections.
    file_end: FileEnd,
}
```

### `FileEnd` Structure

Because the Janex format supports prepending arbitrary data to the file (such as PE/ELF executables or
shell scripts), the file maybe cannot be parsed from the beginning.

Instead, a fixed-size `FileEnd` structure is written at the end of the file.
Readers locate it by seeking backwards from the end of the file,
then use the offsets and lengths it records to find all other sections.

```rust
struct FileEnd {
    /// The magic number of the end-of-file marker.
    ///
    /// Always `0x444e45` ("END\0").
    magic_number: u32, // 0x444e45 ("END\0")

    /// The major version number of the Janex file format.
    /// 
    /// Readers must reject files with an unsupported major version.
    major_version: u32,

    /// The minor version number of the Janex file format.
    /// 
    /// Readers should accept files with a higher minor version within the same major version,
    /// ignoring any unknown fields or entries.
    minor_version: u32,

    /// File-level flags. Currently unused and must be `0`.
    flags: u32,

    /// The total size of the `JanexFile` structure.
    /// 
    /// The reader uses this value together with the actual file size to determine
    /// the byte offset at which the Janex content begins.
    file_size: u64,

    /// The byte length of the `data_pool` section.
    /// 
    /// The `data_pool` begins immediately after the `magic_number` field of `JanexFile`.
    data_pool_length: u64,

    /// The byte offset of the `boot_metadata` section from the start of the `JanexFile` structure.
    boot_metadata_offset: u64,

    /// The byte length of the `boot_metadata` section.
    boot_metadata_length: u64,

    /// The byte offset of the `launcher_metadata` section from the start of the `JanexFile` structure.
    launcher_metadata_offset: u64,

    /// The byte length of the `launcher_metadata` section.
    launcher_metadata_length: u64,

    /// The byte offset of the `signature` section from the start of the `JanexFile` structure.
    /// 
    /// If no signature is present, this field is `0`.
    signature_offset: u64,

    /// The byte length of the `signature` section.
    /// 
    /// If no signature is present, this field is `0`.
    signature_length: u64,

    /// Reserved for future use. All bytes must be `0`.
    reserved: [u8; 80],
}
```

### `BootMetadata`

`BootMetadata` is the section read and interpreted by the Janex Boot component — a JAR that is
placed on the class path and provides the custom ClassLoader used to load classes and resources from
the Janex file.

It declares all resource groups contained in the file and, optionally, a shared string
pool used by the class file compression algorithm.

```rust
struct BootMetadata {
    /// The magic number identifying this as a boot metadata section.
    ///
    /// Always `0x544f4f42` ("BOOT").
    magic_number: u32, // 0x544f4f42 ("BOOT")

    /// The list of boot metadata entries. Each entry carries a 4-byte type tag followed by its payload.
    /// Unknown entry types must be skipped by readers.
    entries: Vec<BootMetadataEntry>,
}
```

#### `BootMetadataEntry`

Each entry in `BootMetadata` begins with a 4-byte type tag (`entry_type`) that identifies the kind of
entry, followed by a variable-length payload. Unknown entry types must be skipped by readers to allow
forward compatibility.

The raw (untyped) form of an entry is:

```rust
struct BootMetadataEntry {
    /// A 4-byte tag identifying the type of this entry.
    entry_type: u32,

    /// The payload bytes of this entry. Its interpretation depends on `entry_type`.
    content: Vec<u8>
}
```

Supported entries:

```rust
enum BootMetadataEntry {
    /// Declares all resource groups contained in this Janex file.
    ///
    /// Each `BootMetadata` may contain at most one `ResourceGroups` entry.
    /// If any resource path within a group uses `RefBody` encoding, a `StringPool` entry
    /// must appear before this entry in the `entries` list.
    ResourceGroups {
        /// The entry type tag for this variant.
        ///
        /// Always `0x53505247` ("GRPS").
        entry_type: u32, // 0x53505247 ("GRPS")

        /// The byte length of the payload that follows.
        length: vuint,

        /// The list of resource groups contained in this Janex file.
        groups: Vec<ResourceGroup>,
    },

    /// A shared string pool used by the class file compression algorithm and `RefBody` resource paths.
    ///
    /// The size of the `StringPool` is at least 1, and the first string (at index 0) is always an empty string.
    /// 
    /// Each `BootMetadata` may contain at most one `StringPool` entry.
    /// When present, it must appear before the `ResourceGroups` entry.
    StringPool {
        /// The entry type tag for this variant.
        ///
        /// Always `0x4c4f4f50` ("POOL").
        entry_type: u32, // 0x4c4f4f50 ("POOL")

        /// The byte length of the payload that follows.
        length: vuint,

        /// Reserved for future use. All bytes must be `0`.
        reserved: [u8; 8],

        /// The total number of strings stored in this pool.
        count: vuint,

        /// The uncompressed byte length of each string, in pool index order.
        /// Used to locate individual strings within the decompressed byte buffer.
        sizes: [vuint; count],

        /// The concatenated UTF-8 bytes of all pool strings, stored as compressed data.
        /// After decompression, individual strings are extracted sequentially using the `sizes` array.
        bytes: CompressedData<[u8]>,
    },
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

    /// Reserved for future use. All bytes must be `0`.
    reserved: [u8; 48],

    /// The number of `Resource` entries stored in this group.
    resources_count: vuint,

    /// The compressed array of resource metadata entries for this group.
    compressed_resources: CompressedData<[Resource; resources_count]>
}
```

#### `Resource`

A `Resource` represents a single entry (regular file, directory, or symbolic link) within a resource
group.

Resources contain only metadata; the actual file content bytes are stored in the top-level `data_pool`
and referenced by offset.

```rust
enum Resource {
    /// Represents a regular file.
    File {
        /// The resource type tag for this variant.
        /// 
        /// Always `0x534552` ("RES\0")
        resource_type: u32, // 0x534552 ("RES\0")

        /// The path of this resource within its resource group.
        path: ResourcePath,

        /// Compression metadata for this resource's content.
        ///
        /// The `uncompressed_size` field within this structure gives the original file size in bytes.
        compress_info: CompressInfo,

        /// The byte offset of this resource's (compressed) content within the `JanexFile`.
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

`ResourcePath` stores a `/`-separated resource path using one of two encodings selected by the value
of `length`:

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
    /// Requires a `StringPool` entry to be present in the enclosing `BootMetadata`.
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
enum ResourceField {
    /// XXH64 checksum of the uncompressed resource content.
    ///
    /// Can be used by the extractor to verify data integrity after decompression.
    Checksum {
        id: u8, // 0x01

        /// The XXH64 hash of the uncompressed resource content.
        checksum: u64,
    },

    /// File creation timestamp.
    FileCreateTime {
        id: u8, // 0x02
        timestamp: Timestamp,
    },

    /// File last-modification timestamp.
    FileModifyTime {
        id: u8, // 0x03
        timestamp: Timestamp,
    },

    /// File last-access timestamp.
    FileAccessTime {
        id: u8, // 0x04
        timestamp: Timestamp,
    },

    /// POSIX file permission bits (e.g., `0o755`).
    PosixFilePermissions {
        id: u8, // 0x05
        /// The POSIX permission bits for this resource.
        permissions: u16,
    },

    /// A custom, application-defined metadata field.
    ///
    /// Custom fields are not interpreted by Janex and are ignored during normal processing.
    /// They can be used to attach arbitrary metadata for tooling or third-party extensions.
    Custom {
        id: u8, // 0x7F
        /// The name of the custom field, used to identify its purpose.
        name: String,
        /// The raw content bytes of the custom field.
        content: Vec<u8>,
    }
}
```

### `LauncherMetadata`

`LauncherMetadata` is the section read by the **Janex Launcher**.

It contains the configuration needed to locate a suitable Java runtime, build the module path and class path,
resolve JVM options, and launch the application.

The configuration is organized as a tree of `ConfigGroup` entries, which allows conditional selection
of settings based on the runtime environment.

```rust
struct LauncherMetadata {
    /// The magic number identifying this as a launcher metadata section.
    ///
    /// Always `0x4e55414c` ("LAUN").
    magic_number: u32, // `0x4e55414c` ("LAUN")
    
    /// The list of launcher metadata entries. Each entry carries a 4-byte type tag followed by its payload.
    /// 
    /// Unknown entry types must be skipped by readers.
    entries: Vec<LauncherMetadataEntry>
}
```

#### `LauncherMetadataEntry`

Each entry in `LauncherMetadata` follows the same type-tagged layout as `BootMetadataEntry`. 

Readers must skip unknown entry types to allow forward compatibility.

The raw (untyped) form of an entry is:

```rust
struct LauncherMetadataEntry {
    /// A 4-byte tag identifying the type of this entry.
    entry_type: u32,

    /// The payload bytes of this entry. Its interpretation depends on `entry_type`.
    content: Vec<u8>
}
```

Supported entries:

```rust
enum LauncherMetadataEntry {
    /// The root configuration group of the launcher metadata.
    ///
    /// Each `LauncherMetadata` must contain exactly one `ROOT_GROUP` entry.
    /// The root group and all its nested subgroups together define the complete launch configuration.
    ROOT_GROUP {
        /// The entry type tag for this variant.
        ///
        /// Always `0x50524752` ("RGRP").
        entry_type: u32, // 0x50524752 ("RGRP")

        /// The byte length of the payload that follows.
        length: vuint,

        /// The root configuration group.
        root_group: ConfigGroup,
    }
}
```

#### `ConfigGroup`

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

The raw (untyped) form of a field is:

```rust
struct ConfigField {
    /// A 4-byte tag identifying the type of this field.
    field_type: u32,
    /// The payload bytes of this field. Its interpretation depends on `field_type`.
    content: Vec<u8>
}
```

Supported fields:

```rust
enum ConfigField {
    /// A CEL condition expression that guards the enclosing `ConfigGroup`.
    ///
    /// See the [Conditions](#conditions) section for details.
    Condition {
        field_type: u32, // 0x444e4f43 ("COND")

        /// The byte length of the payload that follows.
        length: vuint,

        /// The CEL expression string. Must evaluate to `bool` or `int`.
        condition: String,
    },

    /// The fully qualified binary name of the application's main class.
    MainClass {
        field_type: u32, // 0x534c434d ("MCLS")

        /// The byte length of the payload that follows.
        length: vuint,

        /// The fully qualified binary name of the main class (e.g., `"com.example.Main"`).
        value: String,
    },

    /// The name of the application's main module.
    ///
    /// Passed to the JVM via `--module` when launching with the Java module system.
    MainModule {
        field_type: u32, // 0x444f4d4d ("MMOD")

        /// The byte length of the payload that follows.
        length: vuint,

        /// The main module name.
        value: String,
    },

    /// The ordered list of resource groups to place on the module path (`--module-path`).
    ModulePath {
        field_type: u32, // 0x50444f4d ("MODP")

        /// The byte length of the payload that follows.
        length: vuint,

        /// The resource group references to add to the module path, in order.
        items: Vec<ResourceGroupReference>,
    },

    /// The ordered list of resource groups to place on the class path (`-classpath`).
    ClassPath {
        field_type: u32, // 0x50534c43 ("CLSP")

        /// The byte length of the payload that follows.
        length: vuint,

        /// The resource group references to add to the class path, in order.
        items: Vec<ResourceGroupReference>,
    },

    /// The list of resource groups to load as Java agents (`-javaagent`).
    Agents {
        field_type: u32, // 0x544e4741 ("AGNT")

        /// The byte length of the payload that follows.
        length: vuint,

        /// The resource group references for Java agent JARs, in the order they are attached.
        items: Vec<ResourceGroupReference>,
    },

    /// A list of additional JVM options to pass when launching the application.
    ///
    /// Each element is a single JVM option string
    /// (e.g., `"--add-exports=java.base/sun.nio.ch=ALL-UNNAMED"` or `"-Xmx512m"`).
    JvmOptions {
        field_type: u32, // 0x54504f4a ("JOPT")

        /// The byte length of the payload that follows.
        length: vuint,

        /// The list of JVM option strings, each passed as a separate argument to the JVM.
        options: Vec<String>
    },

    /// A list of nested `ConfigGroup` entries within the enclosing group.
    ///
    /// Each subgroup may carry its own `Condition`, enabling fine-grained conditional configuration.
    /// The launcher evaluates each subgroup independently and applies those whose conditions are satisfied.
    SubGroups {
        field_type: u32, // 0x50524753 ("SGRP")

        /// The byte length of the payload that follows.
        length: vuint,

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
        /// declared in the `ResourceGroups` boot metadata entry.
        group_name: String,
    },

    /// A reference to a Maven artifact hosted in a remote repository.
    ///
    /// The artifact is not embedded in the Janex file. The Janex Launcher resolves and downloads
    /// it at launch time (if not already present in a local cache) before starting the JVM.
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
        /// The format is `<algorithm>:<hex-digest>` (e.g., `"sha256:abcdef0123456789..."`).
        checksum: String,
    }
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
    ///
    /// If the version cannot be determined, this field contains an empty string.
    version: OperatingSystemVersion,
}

/// The parsed version of an operating system.
struct OperatingSystemVersion {
    /// The full, unparsed version string.
    full: String,

    /// The major version number.
    major: uint,
}

/// Information about the host CPU.
struct CPU {
    /// The CPU architecture name (e.g., `"x86-64"`, `"aarch64"`, `"x86"`).
    arch: String,
}
```
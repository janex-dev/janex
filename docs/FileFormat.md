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
- **Conditional configuration**: Janex supports conditionally adding classpath entries, module path entries,
  and JVM arguments based on runtime environment conditions using
  [Common Expression Language (CEL)](https://cel.dev/overview/cel-overview) expressions.
- **Prepended data support**: The Janex format allows arbitrary custom data (such as PE/ELF executables or shell
  scripts) to be prepended to the file, enabling shebang support for direct execution on Unix-like systems and
  self-contained distribution with an embedded launcher.

When launching a Janex program, the **Janex Launcher** reads the launcher metadata to find a suitable Java runtime based
on the metadata,
acquires appropriate JVM arguments according to the conditions, and starts a Java process using these JVM arguments.

In the Java process, Janex needs to load a **Janex Boot** provided as a JAR, which provides a custom ClassLoader for
`Janex` to load classes from the Janex file.

Before this, we already had a prototype, whose documentation includes design concepts and other content about this file. 
For more information, please refer to [janex-dev/japp](https://github.com/janex-dev/japp).

This document is an improvement based on the prototype, and we hope to make it safer, more stable, and easier to extend.

## Data Types

### Basic Data Types

Janex uses little-endian encoding for all integer and floating-point numbers.

This document uses `u8`/`u16`/`u32`/`u64` to represent 8/16/32/64-bit unsigned integers,
uses `i8`/`i16`/`i32`/`i64` to represent 8/16/32/64-bit signed integers,
and uses `f32`/`f64` to represent 32/64-bit floating-point numbers.

`bool` is represented by `u8`, `true` is non-zero, and `false` is zero.

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

Janex uses `vuint` to efficiently store integers in some structures.

```rust
type vuint = u64;
```

In the file, `vuint` is stored as one or more byte sequences.

The lower seven bits of every byte contain integer data, and the highest bit in every byte is the continuation flag.

If the highest bit is `0`, means that the current byte is the last byte of the integer;
If the highest bit is `1`, means that the current byte is not the last byte of the integer, and the next byte is the
continuation of the integer.

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

Janex uses UTF-8 encoding for strings:

```rust
struct String {
    length: vuint,
    data: [u8; length],
}
```

### Dynamic Array

Janex uses the following structure to store arrays of variable length:

```rust
struct Vec<T> {
    length: vuint,
    data: [T; length],
}
```

### Timestamp

Janex uses a 96-bit high-precision timestamp:

```rust
struct Timestamp {
    /// The number of seconds from 1970-01-01T00:00:00Z.
    epoch_second: i64,
    
    /// The number of nanoseconds after the epoch_second.
    /// 
    /// In the range of `[0, 1_000_000_000)`.
    nanos: u32, 
}
```

### Compression

Janex uses the following structure to represent compression metadata:

```rust
struct CompressInfo {
    /// The compress method.
    method: CompressMethod,

    /// The uncompressed size of the data.
    uncompressed_size: vuint,
    
    /// The compressed size of the data.
    compressed_size: vuint,
    
    /// Optional options will be passed to the decompressor during decompression.
    options: Vec<u8>,
}
```

The supported compression methods are:

```rust
#[repr(vuint)]
enum CompressMethod {
    /// No compression.
    NONE = 0,

    /// Combine multiple compression algorithms.
    /// 
    /// Its `options` field is a `Vec<CompressInfo>` that contains the compression metadata of the combined algorithms. 
    COMPOSITE = 1,

    /// A compression algorithm developed specifically for class files,
    /// It will put some strings from the constant pool into the shared constant pool.
    CLASSFILE = 2,

    /// Zstandard compression.
    ZSTD = 3,
}
```

We use the following structure to represent compressed data:

```rust
struct CompressedData<T> {
    info: CompressInfo,
    data: [u8; info.compressed_size],
}
```

#### Class File Compression

Janex typically extracts some strings from class files into a shared string pool and then compresses them using Zstandard.

The `CLASSFILE` compression algorithm largely preserves the class file format, but:

1. The magic number of the compressed class file will be rewritten to `0x70CAFECA` (0xCA 0xFE 0xCA 0x70) to distinguish it from the original class file;
2. The compressed class file will contain some new types of constants.

(TODO: More details about the class file compression algorithm)

## Janex File Structure

The Janex file is the format for executable files generated by the Janex tool. Its overall structure is as follows:

```rust
struct JanexFile {
    /// The magic number of the Janex file.
    ///
    /// Always 0x0000_0058_454e_414a ("JANEX\0\0\0").
    magic_number: u64, // 0x0000_0058_454e_414a ("JANEX\0\0\0")

    /// Data pool used to store all information such as class files and resource files.
    data_pool: [u8; ...],

    /// Boot metadata used to describe the classes and modules that need to be loaded at startup.
    boot_metadata: BootMetadata,

    /// Launcher metadata used to describe the configuration information of the launcher.
    launcher_metadata: LauncherMetadata,

    /// File end metadata used to describe the size, offset, etc. of the file.
    file_end: FileEnd,
}
```

### `FileEnd` Structure

The Janex file is designed to allow arbitrary custom data to be attached at the head of the file (e.g., executable files
like PE/ELF, shell scripts, etc.),
so the core metadata of the Janex file is located in the `file_end` field at the end of the file, its structure is as
follows:

```rust
struct FileEnd {
    /// The magic number of the file end.
    ///
    /// Always 0x444e45 ("END\0").
    magic_number: u32, // 0x444e45 ("END\0")

    /// The major version number of the Janex file format.
    major_version: u32,

    /// The minor version number of the Janex file format.
    minor_version: u32,

    /// File flags, currently unused, reserved as `0`.
    flags: u32,

    /// The total size of the Janex file (including header and tail), the Janex tool uses it to locate the file header.
    file_size: u64,

    /// The length of the `data_pool` field in the file.
    data_pool_length: u64,
    
    /// The offset of the `boot_metadata` field in the file.
    boot_metadata_offset: u64,

    /// The length of the `boot_metadata` field in the file.
    boot_metadata_length: u64,

    /// The offset of the `launcher_metadata` field in the file.
    launcher_metadata_offset: u64,

    /// The length of the `launcher_metadata` field in the file.
    launcher_metadata_length: u64,

    /// The offset of the `signature` field in the file.
    signature_offset: u64,
    
    /// The length of the `signature` field in the file.
    signature_length: u64,

    /// Reserved field. All bytes must be `0`.
    reserved: [u8; 80],
}
```

### `BootMetadata`

Boot metadata is read by the Janex Boot.

Its structure is as follows:

```rust
struct BootMetadata {
    /// The magic number of the boot metadata.
    ///
    /// Always 0x544f4f42 ("BOOT").
    magic_number: u32, // 0x544f4f42 ("BOOT")

    /// The entries of the boot metadata.
    entries: Vec<BootMetadataEntry>,
}
```

#### `BootMetadataEntry`

Basic structure of a entry:

```rust
struct BootMetadataEntry {
    /// The entry type of the entry.
    ///
    /// The entry type is a 32-bit unsigned integer that identifies the type of the entry.
    entry_type: u32,

    /// The payload of the entry.
    content: Vec<u8>
}
```

Supported entries:

```rust
enum BootMetadataEntry {
    /// Each `BootMetadata` can only have one `ResourceGroups`.
    ///
    /// If `ResourceGroups` uses `ResourcePathContent.RefBody`, then a `StringPool` must exist before this entry.
    ResourceGroups {
        /// The entry type of the string pool entry.
        ///
        /// Always 0x53505247 ("GRPS")
        entry_type: u32, // 0x53505247 ("GRPS")

        /// The bytes size of the payload.
        length: vuint,

        /// The resource groups of the boot metadata.
        groups: Vec<ResourceGroup>,
    },

    /// A shared string pool used for class file compression algorithms and resource paths.
    ///
    /// Each `BootMetadata` can only have one `StringPool`.
    StringPool {
        /// The entry type of the string pool entry.
        ///
        /// Always 0x4c4f4f50 ("POOL").
        entry_type: u32, // 0x4c4f4f50 ("POOL")

        /// The bytes size of the payload.
        length: vuint,

        /// Reserved field, currently unused.
        ///
        /// All bytes must be `0`.
        reserved: [u8; 8],

        /// The number of strings in the string pool.
        count: vuint,

        /// The bytes size of each string in the string pool.
        sizes: [vuint; count],

        /// The compressed string pool bytes.
        bytes: CompressedData<[u8]>,
    },
}
```

#### `ResourceGroup`

`ResourceGroup` is a group of metadata of class files or resource files.

```rust
struct ResourceGroup {
    /// The magic number of the resource group.
    ///
    /// Always 0x47534552 ("RESG").
    magic_number: u32, // 0x47534552 ("RESG")
    
    /// The name of the resource group.
    /// 
    /// The name must be unique within the `ResourceGroups`.
    name: String,

    /// Reserved field, all bytes must be `0`.
    reserved: [u8; 48],

    /// The number of resources in the resource group.
    resources_count: vuint,

    /// The compressed resource group data.
    compressed_resources: CompressedData<[Resource; resources_count]>
}
```

#### `Resource`

`Resource` is used to represent a file or directory in the resource group. 

`Resource` only contains metadata, and the actual file content is in `data_pool`.

```rust
enum Resource {
    /// Respresents a regular file.
    File {
        resource_type: u32, // 0x534552 ("RES\0")

        /// The path of the resource.
        path: ResourcePath,
        
        /// The compression info of the resource.
        /// 
        /// The original size of the data is also stored there.
        compress_info: CompressInfo,

        /// The offset of the resource content in the `JanexFile`.
        content_offset: vuint,

        /// Optional fields of the resource.
        fields: Vec<ResourceField>,
    },
    
    /// Respresents a directory.
    Directory {
        resource_type: u32, // 0x00524944 ("DIR\0")

        /// The path of the resource.
        path: ResourcePath,

        /// Optional fields of the resource.
        fields: Vec<ResourceField>,
    },

    /// Respresents a symbolic link.
    SymbolicLink {
        resource_type: u32, // 0x4c4d5953 ("SYML")

        /// The path of the resource.
        path: ResourcePath,
        
        /// The target of the symbolic link.
        target: ResourcePath,

        /// Optional fields of the resource.
        fields: Vec<ResourceField>,
    }
}
```

#### `ResourcePath`

`ResourcePath` stores resource paths separated by `/` in an optimized way. Its structure is as follows:

```rust
struct ResourcePath {
    length: vuint,
    content: ResourcePathContent,
}
```

`ResourcePathContent` has two layouts:

When `length` is not `0`, use `StringBody`, storing the path directly in the `ResourcePath` structure;
When `length` is `0`, use `RefBody`, storing two indices of character names in the `StringPool`.

```rust
enum ResourcePathContent {
    /// When `length` is not `0`
    StringBody {
        body: [u8; length],
    },

    /// When `length` is `0`
    RefBody {
        directory_index: vuint,
        file_name_index: vuint,
    }
}
```

#### `ResourceField`

The supported optional fields of a resource are:

```rust
enum ResourceField {
    /// XXH64 checksum of the resource body.
    Checksum {
        id: u8, // 0x01Compress
        
        /// The XXH64 checksum of the resource body.
        checksum: u64,
    },

    /// File creation time.
    FileCreateTime {
        id: u8, // 0x02
        timestamp: Timestamp,
    },

    /// File modification time.
    FileModifyTime {
        id: u8, // 0x03
        timestamp: Timestamp,
    },

    /// File access time.
    FileAccessTime {
        id: u8, // 0x04
        timestamp: Timestamp,
    },
    
    /// Unix file permissions.
    PosixFilePermissions {
        id: u8, // 0x05
        permissions: u16,
    },

    /// Custom field. They will be ignored by Janex. 
    Custom {
        id: u8, // 0x7F
        name: String,
        content: Vec<u8>,
    }
}
```

### `LauncherMetadata`

```rust
struct LauncherMetadata {
    /// The entries of the launcher metadata.
    entries: Vec<LauncherMetadataEntry>
}
```

#### `LauncherMetadataEntry`

Basic structure of a launcher metadata entry:

```rust
struct LauncherMetadataEntry {
    /// The entry type of the entry.
    ///
    /// The entry type is a 32-bit unsigned integer that identifies the type of the configuration field.
    entry_type: u32,

    /// The payload of the configuration field.
    content: Vec<u8>
}
```

Supported entries:

```rust
enum LauncherMetadataEntry {
    ROOT_GROUP {
        /// The entry type of the entry.
        /// 
        /// Always 0x50524752 ("RGRP")
        entry_type: u32, // 0x50524752 ("RGRP")
        
        /// The bytes size of the payload.
        length: vuint,
        
        /// The root group of the launcher metadata.
        root_group: ConfigGroup,
    }
}
```

#### `ConfigGroup`

```rust
struct ConfigGroup {
    magic_number: u32, // 0x50524743 ("CGRP")
    fields: Vec<ConfigField>,
}
```

#### `ConfigField`

Basic structure of a configuration field:

```rust
struct ConfigField {
    field_type: u32,
    content: Vec<u8>
}
```

Supported fields:

```rust
enum ConfigField {
    Condition {
        field_type: u32, // 0x444e4f43 ("COND")
       
        /// The bytes size of the payload.
        length: vuint,
       
        /// The CEL expression that determines whether the program can run.
        /// 
        /// See the Conditions section for more information.
        condition: String,
    },

    MainClass {
        field_type: u32, // 0x534c434d ("MCLS") 
        
        /// The bytes size of the payload.
        length: vuint,
        value: String,
    },

    MainModule {
        field_type: u32, // 0x444f4d4d ("MMOD")

        /// The bytes size of the payload.
        length: vuint,
        value: String,
    },

    ModulePath {
        field_type: u32, // 0x50444f4d ("MODP")

        /// The bytes size of the payload.
        length: vuint,
        items: Vec<ResourceGroupReference>,
    },

    ClassPath {
        field_type: u32, // 0x50534c43 ("CLSP")

        /// The bytes size of the payload.
        length: vuint,
        items: Vec<ResourceGroupReference>,
    },

    Agents {
        field_type: u32, // 0x544e4741 ("AGNT")

        /// The bytes size of the payload.
        length: vuint,
        items: Vec<ResourceGroupReference>,
    },

    JvmOptions {
        field_type: u32, // 0x54504f4a ("JOPT")

        /// The bytes size of the payload.
        length: vuint,
        options: Vec<String>
    },

    SubGroups {
        field_type: u32, // 0x50524753 ("SGRP")

        /// The bytes size of the payload.
        length: vuint,
        subgroups: Vec<ConfigGroup>
    }
}
```

#### `ResourceGroupReference`

```rust
enum ResourceGroupReference {
    /// A reference to a local resource group.
    Local {
        ref_type: u32, // 0x00434f4c ("LOC\0")
        
        /// The name of the local resource group.
        group_name: String,
    },
    
    /// A reference to a Maven artifact.
    Maven {
        ref_type: u32, // 0x00564147 ("GAV\0")
        
        /// The GAV of the Maven artifact.
        gav: String,
        
        /// The repository of the Maven artifact.
        repository: String,
        
        /// The checksum of the Maven artifact.
        checksum: String,
    }
}
```

## Conditions

Janex allows users to declare runtime environment requirements for a program,
such as the Java version, operating system, and CPU architecture.
Janex uses these conditions to determine whether the program can run,
and selects the most suitable Java environment accordingly.

Janex also supports adding classpath entries, module path entries, JVM arguments,
and other information based on conditions.

Users can use [Common Expression Language (CEL)](https://cel.dev/overview/cel-overview) to declare these requirements when building a Janex file.

The expression provided by the user should return `bool` or `int`.
If it returns `int`, it represents the priority; the higher the value, the higher the priority.
Janex will select the Java environment with the highest priority to launch the program.

### Environment

At runtime, Janex provides some values for assertions in conditions.

```rust
let java: Java = ...;
let platform: Platform = ...;
```

The `Java` struct contains the Java version information:

```rust
/// The Java information.
struct Java {
    /// The Java version.
    version: JavaVersion,
   
    /// The vendor of the Java runtime environment.
    vendor: String,   

    /// The operating system of the Java runtime environment.
    os: OperatingSystem,
   
    /// The architecture of the Java runtime environment.
    arch: String,
}

/// The Java version information.
struct JavaVersion {
    /// The full version string.
    full: String,
   
    /// The feature version of the Java version.
    feature: uint,
    
    /// The interim version of the Java version.
    interim: uint,
    
    /// The update version of the Java version.
    update: uint,
    
    /// The patch version of the Java version.
    patch: uint,
    
    /// The optional pre-release information.
    pre: String,
    
    /// The build number.
    build: uint,
    
    /// The optional additional identifying build information.
    optional: String,
}
```

The `Platform` struct contains the operating system and CPU architecture information:

```rust
/// The platform information.
struct Platform {
    /// The operating system information.
    os: OperatingSystem,
   
    /// The CPU architecture information.
    cpu: CPU,
}

/// The operating system information.
struct OperatingSystem {
   /// The name of the operating system.
   name: String,
   
   /// The version of the operating system.
   /// 
   /// If the operating system does not have a version, this field will be empty.
   version: OperatingSystemVersion,
}

struct OperatingSystemVersion {
    /// The full version string.
    full: String,
   
    /// The major version number.
    major: uint,
}

struct CPU {
    /// The name of the CPU architecture.
    arch: String,
}
```
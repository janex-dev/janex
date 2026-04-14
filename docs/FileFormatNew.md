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

### String

String is a special `Vec<u8>` where the bytes are UTF-8 encoded string data:

```rust
type String = Vec<u8>;
```

### Tagged Payload

```rust
struct TaggedPayload<const TAG: u32, T> {
    // Always equal to `TAG`.
    tag: u32,
    
    /// The number of bytes in the payload.
    length: vuint,
    
    /// The payload bytes.
    payload: [u8; length],
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

### `Checksum`

## Janex File Structure

The Janex file is the binary format produced by the Janex build tool for packaging and distributing
Java programs as self-contained executables. Its overall layout is as follows:

```rust
struct JanexFile {
    /// The magic number identifying this as a Janex file.
    ///
    /// Always `0x50504158454e414a` ("JANEXAPP").
    magic_number: u64, // 0x50504158454e414a  ("JANEXAPP")

    sections: [Section; ...],
    
    metadata: Metadata,
}
```

### `Metadata` Section

```rust
struct Metadata {
    magic_number: u64,

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
    flags: [u8; 8],

    /// The total size in bytes of the file.
    ///
    /// The reader uses this value together with the actual file size to determine
    /// the byte offset at which the Janex content begins.
    file_size: vuint,

    /// Records the length and other information of each section.
    section_table: Vec<SectionInfo>,
    
    /// The verification information.
    verification_info: VerificationInfo,
    
    end_mark: u64,  // 0x444e_4558_454e_414a ("JANEXEND")
    
    /// The length in bytes of the metadata section.
    metadata_length: u64,
}
```

#### `Checksum` Structure

The structure of the `Checksum` is as follows:

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

#### `SectionInfo` Structure

The structure of the `SectionInfo` is as follows:

```rust
struct SectionInfo {
    /// The type of a section
    ///
    /// Generally, `section_type` is the same as the `magic_number` of the section content (if the section has a `magic_number`).
    section_type: u64,
    
    /// The length in bytes of the section content.
    length: vuint,
    
    /// The checksum of the section content.
    checksum: Checksum,
}
```

#### `VerificationInfo` Structure

The structure of the `VerificationInfo` is as follows:

```rust
struct VerificationInfo {
    verification_type: String,
    data: Vec<u8>,
}
```

The supported verification types are:

- `Checksum`: `data` is actually a `Checksum`, which is calculated based on the bytes from the start of the `Metadata` structure 
  up to the `verification` field (i.e., ignoring the `verification`, `end_mark`, and `metadata_length` fields).
- `OpenPGP`: OpenGPG signature for the `Metadata` section (ignoring the `verification`, `end_mark`, and `metadata_length` fields).
- `CMS`: CMS signature for the `Metadata` section (ignoring the `verification`, `end_mark`, and `metadata_length` fields).


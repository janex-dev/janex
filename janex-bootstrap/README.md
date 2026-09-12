# Java Bootstrap

This Gradle subproject provides entry invocation, snapshot resource loading, and a read-only NIO file system. `janex-java`
embeds its reproducible `bootstrap.jar`; ordinary Cargo builds require neither Java nor downloads.
The root Gradle build uses JDK 25, compiling the base with `--release 8` and module support with
`--release 9` in a multi-release JAR.
Use the root Wrapper (`gradlew.bat` on Windows):

```text
./gradlew :janex-bootstrap:updateEmbeddedBootstrap
./gradlew check
```

`./gradlew build` compiles Java sources and test fixtures, tests Zstandard format boundaries,
builds the JAR, and verifies the recorded artifact. Cross-language codec and launch tests run
through `cargo test --workspace`.

The Host authenticates an owned snapshot once, evaluates conditions and layers, resolves links,
and produces a bounded resource index. The Java system loader reads ordinary Stored blobs and
Extents from the private snapshot, reverses Zstandard and CLASSFILE encodings on demand, and
retains up to 64 MiB of decoded blob data within the configured byte limit. Root and file override
string pools retain their identities. Resource checksums are not rehashed in Java; authentication
comes from the Host's verified snapshot. Manifests are sanitized in the Host.

The loader preserves parent-first delegation, classpath root order, duplicate resource enumeration,
package metadata and sealing, code-source URLs, and service lookup through the context loader.
Serialized resource URLs use Java's protocol-handler package lookup; the bootstrap appends its
handler prefix without replacing existing prefixes. Resource loading uses Java APIs without JNI,
Unsafe, or JDK-internal class-loader access. Custom system loaders
prevent HotSpot from using archived application classes; expected CDS notices are suppressed for
recognized HotSpot VMs without disabling system-class sharing or overriding explicit user logging.

`Paths.get(resourceUrl.toURI())` uses the installed `janex` provider. It supports directory traversal,
glob/regex matching, read-only seekable channels, basic attributes, and copying to native files.
The `janex` attribute view also exposes nullable exact nanosecond timestamps and POSIX permission
bits. Missing basic timestamps use the epoch; timestamps beyond `FileTime` range saturate.
Closing a filesystem view invalidates its channels but leaves class loading and resource URLs
usable. `FileSystems.newFileSystem` can remount the active snapshot; it does not open arbitrary packages.

Java 9+ uses indexed `ModuleFinder`, `ModuleReference`, and independently closeable `ModuleReader`
implementations. Named and automatic modules share the system resource loader in a child of the
native boot layer. Automatic-module names and versions use original JAR filenames and manifests;
service declarations and module resource encapsulation are retained. Preparation resolves the
module graph without running application entry points or agents. Only required system modules are
added to the actual JVM invocation.

Module access options are applied after layer definition. `--add-reads`, `--add-exports`, and
`--add-opens` use an explicitly exported `jdk.internal.module.Modules` bridge, retaining
`ALL-UNNAMED` behavior. Named `--enable-native-access` also requires access to `java.lang.Module`
internals on the selected JDK. `--patch-module` and custom system loaders require direct mode.
Agents retain the JVM's native JAR protocol and execute after resource-layer initialization.
External-dictionary blobs are decoded by the Host. Direct mode independently retains native
classpath/module-path launching with materialized JARs.

Each launch embeds `launch.bin`: module name, class name, the modern-main boolean, and program
arguments. Strings use a big-endian nonnegative 32-bit UTF-16 unit count followed by those units;
arrays use a 32-bit count. This preserves empty strings, NUL, supplementary characters, and Windows
surrogates without native-launcher charset conversion. JVM options and agent options remain native.
The main method runs on the main thread; exceptions and process exits propagate.

`resources.bin` is private Host/bootstrap communication, not part of the Janex file format. It
contains a magic, limits, snapshot path, topologically ordered sources, shared pools, and ordered
roots, module requirements, and resource metadata. `options.bin` carries the module entry and JVM
options needed by the module bridge. The Host and embedded Java artifact are built together. The snapshot and index must remain
private and alive until the child exits. The Java reader does not reopen the original package.

The Java Zstandard decoder is implemented in this project under MPL-2.0, using
[RFC 8878](https://www.rfc-editor.org/rfc/rfc8878.html), the
[Zstandard format description](https://github.com/facebook/zstd/blob/dev/doc/zstd_compression_format.md),
and the [XXH64 algorithm description](https://github.com/Cyan4973/xxHash/blob/dev/doc/xxhash_spec.md).
It supports dictionary-free frames, raw/RLE/compressed blocks, Huffman literals, FSE sequences,
frame checksums, and skippable frames. Codec tests compare it with native libzstd output.

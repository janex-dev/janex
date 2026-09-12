# Java Bootstrap

This Java 8 Gradle subproject provides entry invocation and direct classpath resource loading. `janex-java`
embeds its reproducible `bootstrap.jar`; ordinary Cargo builds require neither Java nor downloads.
The root Gradle build selects a JDK 25 toolchain and compiles with `--release 8`.
Use the root Wrapper (`gradlew.bat` on Windows):

```text
./gradlew :janex-bootstrap:updateEmbeddedBootstrap
./gradlew check
```

`./gradlew build` compiles Java sources and test fixtures, builds the JAR, and verifies the recorded
artifact. Cross-language codec and launch tests run through `cargo test --workspace`.

The Host authenticates an owned snapshot once, evaluates conditions and layers, resolves links,
and produces a bounded resource index. The Java system loader reads ordinary Stored blobs and
Extents from the private snapshot, reverses Zstandard and CLASSFILE encodings on demand, and
retains up to 64 MiB of decoded blob data within the configured byte limit. Root and file override
string pools retain their identities. Resource checksums are not rehashed in Java; authentication
comes from the Host's verified snapshot. Manifests are sanitized in the Host.

The loader preserves parent-first delegation, classpath root order, duplicate resource enumeration,
package metadata and sealing, code-source URLs, and service lookup through the context loader.
Serialized resource URLs use Java's protocol-handler package lookup; the bootstrap appends its
handler prefix without replacing existing prefixes. It uses Java APIs without JNI, Unsafe, or
JDK-internal class-loader access. Custom system loaders
prevent HotSpot from using archived application classes; expected CDS notices are suppressed for
recognized HotSpot VMs without disabling system-class sharing or overriding explicit user logging.

External-dictionary blobs are decoded by the Host into the private index. Module paths and agents
still use materialized JARs. A module entry point retains native module resolution and patches the
entry bridge into the main module. Direct mode retains ordinary JAR launching independently.
Direct module loading and a read-only NIO file-system provider are planned separately.

Each launch embeds `launch.bin`: module name, class name, the modern-main boolean, and program
arguments. Strings use a big-endian nonnegative 32-bit UTF-16 unit count followed by those units;
arrays use a 32-bit count. This preserves empty strings, NUL, supplementary characters, and Windows
surrogates without native-launcher charset conversion. JVM options and agent options remain native.
The main method runs on the main thread; exceptions and process exits propagate.

`resources.bin` is private Host/bootstrap communication, not part of the Janex file format. It
contains a magic, limits, snapshot path, topologically ordered sources, shared pools, and ordered
roots. The Host and embedded Java artifact are built together. The snapshot and index must remain
private and alive until the child exits. The Java reader does not reopen the original package.

The vendored Zstandard decoder derives from japp/Airlift. See [NOTICE](NOTICE) and
[LICENSE-APACHE-2.0](LICENSE-APACHE-2.0) for provenance and modifications.

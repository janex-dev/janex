# Java Reader

`janex-reader` provides Java 8 APIs for reading Janex 0.1 independently of the native Host.
The file format is defined only by [FileFormat.md](spec/FileFormat.md).

Public reading APIs live in `org.glavo.janex.reader`. Binary and archive helpers live in
`org.glavo.janex.reader.internal`; portable codecs live in `org.glavo.janex.reader.internal.codec`.
The reader has no dependency on the bootstrap module. `new JanexReader(path)` uses its portable
Zstandard decoder; constructor overloads accept custom decoding and acquisition policies.
`ClassFile.restore` restores CLASSFILE content independently of a running launcher.

`JanexReader.launch()` returns selected metadata and immutable `ResourcePlan` descriptions.
The bootstrap's `org.glavo.janex.bootstrap.loader.ResourceIndexes` serializes these descriptions into
the private resource index and enforces its encoded-size limit. The reader does not encode that
protocol. Snapshot ranges remain lazy; returned arrays are copies and collection views are immutable.

The bootstrap keeps entry coordination in `org.glavo.janex.bootstrap`, resource and module loading in
`.loader`, NIO views in `.fs`, and acquisition policy in `.dependency`. NIO accesses decoded data
through read-only buffers. Gradle still combines both modules into one distributable bootstrap JAR.

`ReadLimits` configures buffered byte lengths, collection sizes, and nesting depth. The defaults
match Rust: 256 MiB per encoded or decoded value, 1,000,000 elements, and depth 64. These are
reader policy; the format version remains 0.1. Java buffered lengths are limited to nonnegative
`int` values. Container, application, and launch-reader constructor overloads accept the policy.
Nested decoding inherits it, and launch preparation carries byte and element limits into the
resource index. Integrity scans stream content and do not require the whole file to fit this limit.
Dependency resolvers and compression callbacks remain responsible for their own acquisition and
working-memory limits.
Zstandard frame checks enforce the native reader's advertised-window cap before decompression;
the portable indexed resource reader applies the same check to lazily decoded blobs.

Blob pools open on first reference. Opening validates the page directory and every checksum
descriptor; reading a page validates its checksum and entry framing. Logical indices are not
collection counts and may exceed Java's array-index range. Unselected Stored lengths remain
unsigned metadata; buffered byte limits apply when their sources are prepared or decoded.

`JanexReader.applications()` exposes validated application metadata in section order. Construction
checks every supported application descriptor, including inactive configuration branches. Unknown
application types retain their descriptors for inspection. `Application` provides localized titles
and comments, version and launch mode, and copies of the original section and type-info bytes.
Title lookup falls back to the integration command and then the application ID.

Java version conditions use the same alias grammar, numeric bounds, prerelease ordering, and VERS
timelines as the native reader. Overlay selection visits matching branches in depth-first order,
prunes unmatched subtrees, and bounds both accumulated lists and pending branches. Inactive
configuration remains subject to schema validation. Preset arguments retain their exact order and
boundaries through list append and clear operations.

`PackageUrl` validates canonical package references and exposes their decoded components without
acquiring dependencies. Application validation checks references in inactive branches as well.
Maven acquisition uses these same components; transport and repository policy remain in the bootstrap.
Canonical lowercase checks use a generated Unicode table, independent of the running JDK. Regenerate
it with `cargo run -p janex-format --example generate_java_case_checks` when updating the native Unicode
tables; the cross-language test compares every Unicode scalar value.

Resource layers validate names, metadata, and conflicts even when inactive. File BlobRefs and
explicit transform pools are resolved only for selected resources. Names and paths use UTF-8
ordering and byte limits. Symbolic links resolve relative to their containing directory with a
cumulative link limit; directory aliases use bounded traversal. The resulting resource index uses
native traversal order and directory markers. Launch preparation omits stale JAR signature files
and verifies recorded Manifest checksums before rewriting them at their expanded paths, without
changing other aliases of the same file. Resource timestamps retain all signed 128-bit nanoseconds;
permission zero remains distinct from unspecified permissions.

`ClassFile.validate` checks ordinary class framing, constant-pool references and Modified UTF-8,
code and attribute boundaries, and module descriptors under the supplied reader limits. The
bootstrap applies the same validation after reversing CLASSFILE transforms, including the indexed
constant-pool limit. Bytecode type verification remains the runtime's responsibility.

`ContainerReader` owns a seekable channel or opens a local file. It supports automatic standalone
and JAR/ZIP64 boundary discovery, or an explicit external-tail length. Opening validates framing
and common metadata without interpreting section bodies or verifying checksums and signatures.
Metadata accessors return copies of the original CBOR bytes, preserving unknown fields.
`verificationInput()` retains the exact original Sized encoding and verification type byte.

```java
try (ContainerReader reader = new ContainerReader(path)) {
    byte[] originalInput = reader.verificationInput();
    ContainerReader.Verification verification = reader.verification();
    ContainerReader.IntegrityReport integrity = reader.verifyChecksums();
}
```

`verifyChecksums()` supports all five checksum algorithms and streams section and external-region
bytes. Its report counts content checksums and indicates complete secure coverage. Missing digests
are reflected in coverage; no report asserts signer trust. The operation does not cache results.
The caller must keep the source unchanged while using the reader or retaining a report.

`JanexReader` prepares resources and launch data. It verifies recorded container integrity once
before interpreting section bodies. By default it permits None and Checksum verification and
rejects signed packages. Its constructor overload accepts an `AuthenticationPolicy`, called for
all four verification types before content verification, section-body interpretation, or dependency
acquisition. Policies may require signing and must validate the Janex OpenPGP/CMS profile,
cryptographic signature, required signers, and applicable trust, algorithm, time, and revocation
rules. The reader supplies the exact input and payload; it does not implement signature cryptography.

A policy rejection aborts preparation without an unsigned fallback. Signed preparation additionally
requires secure checksums for every section and nonempty external region, with both external-region
lengths declared. Passing authentication cannot waive these checks. Policies and dependency resolvers
remain caller-owned; construction failure closes the reader's source. Default `java -jar` launching
does not provide an authentication policy; use the native Host for its built-in signature support.

Standalone and native bootstrap launches defer module access options until indexed modules exist.
Both operand forms retain `ALL-UNNAMED` semantics, including unnamed modules created later.
Invalid module options fail during preparation before application execution. Direct mode retains
the Java launcher's own option handling; see [CLI.md](spec/CLI.md) for standalone restrictions.

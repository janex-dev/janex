# Janex Implementation: Packaging, Dependencies, Signing, and Java Launching

This document tracks implementation tasks and acceptance criteria. See [ROADMAP.md](ROADMAP.md)
for the project's long-term direction and delivery priorities.

## SDK Management

- Add shared `available`, `install`, `list`, `update`, `uninstall`, `default`, `current`, `home`,
  `use`, `pin`, `unpin`, `exec`, and `env` commands, initially backed by Java distributions from Foojay Disco.
- Preserve all installed versions. Record requirements separately from concrete releases, platform
  variants, and SHA-256 identities. Support fixed builds and pinned request bindings.
- Download over HTTPS and require a secure archive checksum; obtain GitHub asset SHA-256 metadata
  when a Disco record only advertises SHA-1. Stream downloads and bound extraction.
- Commit SDK trees before atomically replacing the CBOR registry. Serialize registry writes and
  protect selected SDKs with shared leases during prepared and running Janex executions.
- Support read-only external registration, global defaults, declarative project selection, shell
  environment output, child execution, and managed-Java discovery in both application launch modes.
- Validate archive traversal, links, limits, corrupt metadata, multiple versions, pinned updates,
  default protection, active leases, external ownership, and Windows/Linux command integration.
- Support Gradle and Apache Maven distributions through the same installation, pinning, update,
  ownership, and lease machinery. Query official metadata and require SHA-256/SHA-512 sidecars.
- Keep defaults and project selections independent for Java, Gradle, and Maven. Combine selected
  homes during shell activation and execution; preserve Windows batch launcher behavior.
- Keep Maven application installation and additional SDK providers as subsequent work.
- Provide thin Bash, Zsh, sh, Fish, and PowerShell integration around native environment rendering.
  Make `use` shell-local and `use --project` explicit. Retain the activation-time environment,
  remove stale SDK PATH entries, apply project selections without stale generated home overrides,
  and evaluate only successful native results. Validate real interpreters and failure atomicity.
  Keep automatic directory hooks as subsequent work.
- Distribute executables independently of `JANEX_HOME`, with embedded shell templates.
  `janex init` writes all supported loaders into `JANEX_HOME/shell`, binding the current executable
  and user directory. Keep shell rendering internal and retain integration after `deactivate`.
  Reinitialization updates only managed scripts. Root Gradle installation accepts an explicit
  executable directory; platform archives contain executables and documentation at the root.
- Keep SDK switching based on shell environment updates. Future application installation exposes
  stable command entry points in `JANEX_HOME/bin`, independently of the active SDK environment.

## Goals and Boundaries

Implement the complete directory or JAR -> `.janex` -> Java process workflow, supporting classpath
and module-path applications, together with OpenPGP and CMS signature generation and verification.

Support existing and managed Java runtimes. Keep global trust stores and persistent verification
caches separate from SDK management. Keep the file format at version **0.1**.

## Specification and Module Boundaries

Update [FileFormat.md](docs/spec/FileFormat.md) before implementing the affected behavior:

- Skip an overlay's entire subtree when its condition does not match.
- Continue ignoring malformed language tags in `LocalizedText`, but require at least one well-formed
  tag and reject tags that are duplicates under case-insensitive comparison.
- Define the optional resource-root `name` at integer metadata key `0` as a suggested export filename.
  Java consumers append a missing `.jar` suffix and use `resources.jar` for unnamed roots. JAR importers
  retain the original filename. Materialize each root in a separate directory to avoid collisions.

Use five Rust crates and two Java projects:

- Use `janex-format` for container reading and writing, compression, resource trees, CLASSFILE
  transforms, format conditions, version comparison, and checksums. It does not depend on the
  Java runtime or signature libraries, CLI, networking, or installation management.
- Use `janex-signature` for CMS and OpenPGP signatures, key decoding and unlocking, certificate
  validation, and caller-supplied authentication policy. It owns its parsing limits and errors.
- Use `janex-java` for runtime discovery and probes, bounded JAR reading, manifests, and native
  or bootstrap launch argument preparation. Its inputs do not contain Janex format types.
- Use `janex-bootstrap` for portable Java resource loading and entry invocation. The Host supplies
  an evaluated resource index over its verified snapshot on the default launch path.
- Use `janex-reader` for independent Java parsing, integrity checks, portable codecs, and launch/resource
  selection in `org.glavo.janex.reader`. Internal helpers and codecs occupy its `internal` subpackages.
  Return immutable resource descriptions; bootstrap `.loader` code owns private-index serialization.
  Keep NIO views in bootstrap `.fs` and acquisition policy in `.dependency`. A child JVM
  from the current Java installation receives the selected startup options and existing loader.
- Use `janex-host` to orchestrate local packaging, load trust material, apply execution policy,
  prepare resources, select compatible runtimes, and own temporary files and process lifetimes.
  It converts format limits, conditions, and application descriptors into capability inputs.
- Use `janex-cli` for argument parsing, terminal interaction, and result presentation. Update
  [CLI.md](docs/spec/CLI.md) alongside the command implementation.

## Implementation Order and Interfaces

### Java Reader Parity

Align independent Java reading with the implemented Rust format and launch behavior. Keep version
0.1, Java 8 classpath support, and both native launch modes. Parsing and checksum support must not
silently imply signature authentication. The format specification remains authoritative.

- [x] Share all five checksum algorithms across container, blob-page, resource, and dependency
  validation. Use bounded streaming state, canonical digest bytes, and independent Rust/C vectors.
- [x] Decode raw and trained external Zstandard dictionaries, including page restrictions and
  reference/recursion constraints, through the existing resource index.
- [x] Align ordinary and ZIP64 wrapper discovery and external JAR reading, including bounds,
  ambiguous end records, entry metadata, and malformed archive rejection.
- [x] Support selected Java agents with preserved arguments, ordering, manifests, and lifecycle.
- [x] Resolve virtual Java module requirements against the current runtime and physical module
  path, preserving exact version constraints and selected overlay behavior.
- [x] Separate container parsing, recorded-integrity verification, and caller authentication policy;
  support the format's verification variants without bypassing signed-package authentication.
- [x] Expose validated application metadata and localized presentation, checking every application
  and inactive configuration branch before launch selection.
- [x] Provide immutable reader-limit configuration with Rust defaults and propagate it through
  nested decoding, imported JARs, Blob preparation, evaluated lists, and private-index output.
- [x] Align resource-name encodings, UTF-8 order and byte limits, implicit directories, layer
  conflicts, cumulative symbolic-link limits, lazy file references, and expanded index paths.
  Preserve original bytes through other aliases when rewriting manifests; skip stale JAR signatures.
  Enforce the native Zstandard advertised-window policy before eager and indexed decoding.
- [x] Compare deterministic CBOR and binary framing with shared vectors covering every half-float
  encoding, float-width boundaries, unknown values, map ordering and original bytes, UTF-8,
  sized empty maps, nesting, and ULEB128 overflow on Java 8 and the current JDK.
- [x] Match CLASSFILE restoration and structural validation: constant-pool slots and references,
  Modified UTF-8, class versions, Code and attribute boundaries, module descriptors, exact output
  sizes, and inherited byte/element limits. Compare valid classes and systematic mutations on
  Java 8 and the current JDK.
- [x] Compare resource metadata kinds, all timestamp fields and signed i128 boundaries, permission
  absence versus zero, unknown fields, and inactive-layer validation. Check original Manifest
  checksums before rewriting, including aliases and all five algorithms.
- [x] Compare Java version grammar, VERS boundaries, condition selectors and runtime requirements
  across Java 8 and current Java. Validate candidates even for wildcard ranges and parse long
  numeric sequences without recursive regular expressions.
- [x] Match selected overlay order, subtree pruning, entry-point replacement, argument append/clear
  behavior, inactive-branch validation, and the aggregate pending-overlay limit.
- [x] Match lazy BlobPool opening, page-descriptor validation, selected-page entry framing,
  wide logical indices, and deferred Stored lengths with shared acceptance/rejection fixtures.
  Compare module access options in standalone, native bootstrap, and direct launches, including
  separate operands, ALL-UNNAMED, native access, and rejection before application execution.
- [x] Validate canonical external PURLs independently of selected dependency acquisition, retaining
  exact decoded components and registered type constraints. Use shared acceptance/rejection vectors
  and compare every Unicode scalar value on Java 8 and the current JDK.
- [x] Run the full workspace suite, Java 8/current-JDK integration tests, Gradle checks, Clippy,
  and reproducible embedded-JAR verification after integration. Record any platform evidence gaps.

Acceptance on Windows, 2026-09-13: `cargo test --workspace --locked` passed all 161 tests with no
failures or ignored tests, with `JANEX_TEST_JAVA8_HOME` selecting Corretto 8u452 alongside OpenJDK 25.
`gradlew.bat check --rerun-tasks --no-build-cache --console=plain`, workspace Clippy with
`-D warnings`, `cargo fmt --all -- --check`, and `git diff --check` passed. The rebuilt bootstrap
was verified byte for byte across repeated builds. The BlobPool fixture also passed after its Clippy cleanup.

The acceptance evidence is maintained in `janex-host/tests`:

- `reader_container`, `reader_authentication`, and `reader_checksums`: original wire bytes,
  verification boundaries, coverage, all algorithms, and bounded streaming.
- `reader_cbor`, `reader_blobs`, `reader_dictionaries`, `reader_archives`, and `reader_limits`:
  binary framing, paging, Extents, compression, wrappers, malformed input, and inherited limits.
- `reader_applications`, `reader_conditions`, `reader_launch_selection`, `reader_purls`,
  `reader_resources`, and `reader_classfiles`: metadata, selection, names, links, and transforms.
- `standalone`, `standalone_agents`, `standalone_modules`, and `standalone_dependency`: launch
  modes, arguments, access options, module requirements, agents, acquisition, and shared caches.

Linux and macOS have not run this uncommitted tree; the existing three-platform CI matrix remains
the platform acceptance gate after submission. Signature cryptography remains a caller/Host
responsibility, and direct `java -jar` ZIP64 support remains bounded by the initial JVM, as documented.

### Standalone Java Launch

`pack --with-launcher` appends the bootstrap JAR using the existing JAR Tail Wrapper. Metadata binds
the tail's exact size and SHA-256 digest. Keep native bootstrap and direct launching available.
The initial standalone profile supports embedded classpath/module roots, resource layers, shared
data pools, Stored/Extents blobs, Zstandard with raw and trained external dictionaries, and
CLASSFILE transforms on Java 8+.
It accepts None or Checksum metadata verification and checks every recorded container digest using
the five specified algorithms. It rejects signed packages and unsupported startup options
explicitly. Publisher authentication remains a Host capability.
The reusable `ContainerReader` parses all four verification declarations without authenticating
them, retains exact metadata and signature-input bytes, and verifies recorded integrity separately.
`JanexReader` accepts a caller authentication policy for signed preparation, requires complete
secure content coverage, and verifies integrity once before interpreting section bodies or acquiring
dependencies. Default standalone launching does not supply a signature policy.
Resolve virtual module requirements against system modules and indexed physical module roots,
checking exact descriptor versions and conflicting constraints after overlay selection. Validate
module resolution in a preparation JVM before executing agents or application entry points.
Reject selected module paths on Java 8, including paths containing only virtual requirements.
Selected local and remote agents are materialized from a separate resource index after all
dependencies resolve. Preserve their order, unsplit options, manifests, and native premain lifecycle;
prepare all agent JARs before the child starts and remove them after it exits or preparation fails.
Verify agent-file checksums over restored original bytes before rewriting manifests.
The reader supports ordinary and ZIP64 tails and dependency JARs. `Standalone.launch(Path, String[])`
also supports executable ZIP64 tails by extracting the validated tail before bridge construction.
Direct `java -jar` depends on the initial JVM's archive support: local OpenJDK 25 rejects the
prefixed ZIP64 fixture before Janex starts. Keep the generated launcher tail in ordinary ZIP form.
Resolve selected HTTP(S) JARs and exact Maven PURLs through a reader callback implemented by the
bootstrap module. Share the Host's cache keys and records, verify all five checksum algorithms, and support
offline, refresh, and repository overrides. Import remote archives through the same resource index,
retaining multi-release layers, automatic-module filenames, permissions, and symbolic links.
Verify cache interchange in both directions, corruption repair, failed-download isolation, and
offline launches after the server stops. Keep format parsing independent of networking policy.
Verify Java 8 and current-JDK launches, module resources, JVM options, exact preset arguments,
exit status, malformed encoding, corruption rejection, and continued native launching.

### 1. Format Reading and Writing

- Implement binary primitives, strict deterministic CBOR, checksums, metadata, sections, and the
  footer. Preserve unknown CBOR fields. Verify signatures against the original signed bytes;
  re-encoding must not substitute for those bytes.
- Provide a container reader based on `Read + Seek`, a sequential writer, and structured errors
  carrying location and context. Reader options control parsing limits. Check integer arithmetic
  and bounds before allocating memory.
- Support paged blob tables, Stored and Extents entries, multiple Zstandard frames and external
  dictionaries, standalone files, caller-supplied external-tail lengths, and JAR/ZIP64 tail discovery.
- Implement resource-layer merging, all three string-reference representations, symbolic-link
  resolution, and both CLASSFILE external-string entry types. CLASSFILE decoding must restore the
  exact original bytes. Apply the transform only when it saves space.
- Use `minicbor` with a specification-validation layer, `zstd`, and established checksum libraries.
  Lock resolved dependency versions in `Cargo.lock`.

### 2. Local Packaging

Add the following command interface:

```text
janex pack <SOURCE> --output <FILE>
    [--class-path <PATH>]...
    [--module-path <PATH>]...
    [--main-class <NAME>]
    [--main-module <NAME>]
    [--application <ID>]
    [--jvm-option <ARG>]...
    [--argument <ARG>]...
    [--java-version <VERS>]
```

- Accept directories and JARs. Each input becomes a separate resource root, preserving path order.
  Place the primary input on the module path when `--main-module` is specified, otherwise on the
  classpath.
- Prefer an explicitly supplied main class. Otherwise, obtain the entry point from the JAR manifest
  or module descriptor. Report an error if no entry point can be determined. Default the application
  ID to `main`.
- Parse manifest continuation lines and Multi-Release JARs correctly, generating resource layers
  according to the specification. Manifest `Class-Path` entries must not trigger dependency
  acquisition; dependencies must be supplied explicitly through command arguments.
- Preserve the original JAR filename and resource contents. Do not follow symbolic links while
  importing directories. Report unrepresentable paths, duplicate entries, and conflicts as errors.
- Write shared data pools and blob data with XXH3-64 checksums for file entries. Use SHA-256 for
  sections, blob-table pages, and the default Checksum verification type. Write to a temporary file
  and publish the result only after successful completion.
  Report an error if the destination already exists.

### 3. Signatures and Authentication

- Retain one `VerificationInfo` per file, supporting Checksum, OpenPGP, and CMS. Do not introduce a
  signature list or per-blob authentication.
- Add mutually exclusive signing options to `pack`: an OpenPGP secret key, or a CMS signer certificate
  and private key. Unlock encrypted keys using hidden terminal input or a password file; do not
  accept plaintext passwords as command-line arguments.
- Use [rPGP](https://docs.rs/pgp/latest/pgp/) for OpenPGP. Use
  [RustCrypto CMS](https://docs.rs/cms/latest/cms/signed_data/struct.SignerInfo.html), DER/X.509, and
  the corresponding cryptographic libraries for CMS. Janex must enforce its own packet, signed
  attribute, algorithm-binding, and authenticated-scope requirements.
- Initially support RSA/SHA-256, RSA/SHA-512, and ECDSA P-256/P-384 signatures. Additionally support
  Ed25519 for OpenPGP. Return explicit unsupported-algorithm errors for other algorithms.
- Establish trust from explicitly supplied material: an OpenPGP public key and its valid signing
  subkeys, or directly specified CMS signer certificates. Check validity periods, permitted usage,
  and revocation information available in the supplied material. Do not establish trust through
  network requests.
- Allow multiple required CMS signer certificates; verification fails if any required signer is
  missing. Library interfaces must distinguish structural validation, checksum validation,
  cryptographic signature validity, and trust results.
- Allow explicitly requested execution of unsigned local files. Signed files must pass the
  corresponding authentication checks. Missing trust material or failed authentication must not
  downgrade execution to an unsigned mode.

### 4. Java Launching

Implement the local-file form of:

```text
janex run [OPTIONS] <TARGET> [ARGS...]
```

- Support `--application`, `--java-home`, `--java`, and trust-material options. Janex options must
  precede the target; forward arguments after the target unchanged.
- Honor explicit runtime selection; otherwise prefer native-architecture candidates from
  `JAVA_HOME` and `PATH`, retaining other runnable architectures as fallbacks. Report an error when
  an explicitly selected runtime does not meet requirements; otherwise try the next candidate.
  Support classpath launching on Java 8; module launching requires Java 9 or later.
- Obtain a stable input snapshot and verify it completely once. Reuse the result during parsing
  and materialization for the current launch. Do not persist publisher-authentication results.
- Evaluate conditions, overlays, and resource layers against each candidate runtime. Check local
  modules and modules supplied by the runtime. Report missing dependencies without downloading
  arbitrary module providers. Resolve explicit HTTP(S) JARs and exact Maven PURLs after package
  authentication, verify declared checksums, and support an atomic local cache and offline mode.
- For bootstrap entry points, pass original package and verified dependency-cache paths without
  copying files or repeating whole-file verification. Require these files to remain unchanged until
  application exit; create temporary launch files only for agents and direct-mode resources. Read ordinary
  Stored blobs, Extents, Zstandard frames, and CLASSFILE transforms on demand in Java. Preserve root
  order, resource enumeration, manifest package attributes, sealing, and service discovery.
- Load named and automatic modules directly through indexed module finders and readers, retaining
  original filenames, services, and access options. Keep system modules in the native boot layer.
- Expose resource URLs through a read-only NIO provider with path operations, directory traversal,
  seekable channels, metadata, and explicit view lifetimes. All readers share the verified snapshot.
- Materialize agents and direct-mode paths as JARs. Preserve the native agent lifecycle and keep
  direct classpath and module-path launching independently usable.
- Decode external-dictionary blobs during preparation in both the Host and the standalone reader.
  Bound index size, logical expansion, and Java blob-cache retention. Resolve symbolic links before
  launching, rejecting dangling links, cycles, and root escapes.
- Produce a structured execution plan, then start Java directly without a shell. Preserve JVM and
  program argument boundaries. Place preset program arguments before user-supplied arguments.
  Inherit the working directory and standard streams, and propagate the process exit status.
- Retain explicit `bootstrap` and `direct` launch modes. The default bootstrap transports Unicode
  program arguments independently of the native launcher's code page; direct mode uses the native
  application entry point. Keep this choice in the runtime implementation, not the file format.
  Do not make future runtime backends, including translated Java on Windows on Arm or IKVM,
  depend on the bootstrap being available.

## Validation and Acceptance

- **Encoding:** fixed byte vectors, ULEB128 boundaries, deterministic CBOR, the empty-map special
  encoding, digest byte order, truncation, and length overflow.
- **Containers:** independently constructed reader fixtures and writer round trips; pagination,
  Extents, dictionaries, corrupted checksums, unknown types, and JAR/ZIP64 tails.
- **Resources:** mixed string-reference forms, exact CLASSFILE reconstruction, cross-layer conflicts,
  tombstones, symbolic-link resolution, overlay subtrees, and Java version comparison.
- **Signatures:** generation and verification of both formats; tampering, incorrect keys, expired or
  revoked material, missing required signers, and incorrect protected fields. Include fixtures
  generated by independent tools rather than relying solely on self-generated round trips.
- **Execution:** compile small fixtures with the available JDK and test directories, JARs, local
  dependencies, explicit modules, filename-derived automatic modules, Multi-Release resources,
  empty arguments, Unicode, and exit codes.
- Run `cargo fmt --check`, workspace tests, and Clippy. Validate platform behavior with Windows,
  Linux, and macOS CI.

Acceptance requires working packaging, signing, verification, and launching. Type skeletons and
placeholder interfaces do not constitute completion.

## Native Launcher

- Build `janex-launcher` as a separate Rust executable reusing Host preparation and execution.
- Package PE or ELF prefixes with bounded private launch configuration and authenticated header
  coverage. Preserve optional JAR tails and both Java invocation modes.
- Keep launcher, native system, and JVM architectures distinct. Prefer native JVM candidates while
  evaluating application and resource architecture conditions against the selected JVM.
- Validate x86 Windows wrappers launching x64 Java under WOW64; retain Windows ARM64 emulation
  testing as a hardware acceptance requirement. Keep generated binaries out of the repository.
- Cover application arguments, signatures and tampering, resource lifetime, Java 8, direct mode,
  JAR tails, and Unix process termination with executable integration tests.

## Gradle Plugin

- Provide `org.glavo.janex` as a Java Gradle plugin that can also be included as a standalone build.
- Package the project JAR and ordered runtime dependencies through `janex-writer`, with lazy
  application conventions, explicit classpath/module-path overrides, and both launcher forms.
- Keep ordinary packaging independent of native executables; native launcher prefixes remain explicit inputs.
- Generate Maven publications and plugin markers for local or configured CNB repositories.
- Verify real packaged applications with TestKit, configuration-cache reuse, dependency changes,
  and preservation of the previous output when the writer fails. Keep a runnable composite-build demo.

## Java Writer

- Provide `janex-writer` alongside `janex-reader`, targeting Java 17 without native libraries.
- Import bounded JARs and directory trees with deterministic UTF-8 ordering, Multi-Release layers,
  symbolic links, permissions, shared file blobs, and external dependency declarations.
- Write Janex 0.1 with XXH3-64 file checksums and SHA-256 section, page, metadata, and wrapper coverage.
- Bundle the generated bootstrap JAR as a build resource; never commit generated binaries.
- Preserve JAR-tail and native-prefix packaging, with both bootstrap and direct invocation modes.
- Validate Java output with the Rust Host and Rust output with the Java reader and launcher.
- Compress blobs and table pages with portable Zstandard when their encoded representation shrinks;
  expose a default-enabled Gradle compression switch and verify both modes across readers and launchers.
- Encode both CLASSFILE external-string forms, preserve Modified UTF-8 exactly, and share class-name
  components with resource names. Compare complete compressed pools before selecting transforms.
- Sign with CMS or OpenPGP after selecting the final representation, including encrypted private-key
  loading and native-wrapper public signer pins. Preserve failure atomicity and explicit signing time.
- Expose transforms and signing through Gradle, loading secrets only during task execution and excluding
  signed tasks from build-cache and up-to-date reuse. Verify all algorithms through the Rust Host.

## Java Bootstrap Validation

The root Gradle build uses Java 8 base classes and Java 9 module classes in a reproducible
multi-release JAR. The portable Zstandard decoder is implemented under MPL-2.0. `./gradlew build`
assembles Java and Rust and runs both languages' checks; `assembleRelease` produces optimized native
binaries. Cargo compilation, Clippy, and tests depend on the bootstrap JAR task, with incremental
compilation managed by Cargo. `clean` removes both languages' build outputs.
Rust embeds `janex-bootstrap/build/libs/janex-bootstrap.jar`; generated JARs remain
in ignored build directories and are not stored in the source repository.

Cover NIO URI round trips, full directory traversal, channel positioning, write rejection,
metadata precision, closure and remounting on Java 8 and current Java. Cover direct module reads,
automatic modules, service discovery, access options, module-reader lifetimes, and agents on Java 9+.
The module bridge uses explicitly enabled JDK module-access APIs. Custom system loaders and
`--patch-module` use direct mode. Host authentication covers the complete snapshot once;
the Java readers do not add per-resource publisher authentication.

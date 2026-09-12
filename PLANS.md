# Initial Janex Implementation: Local Packaging, Signing, and Java Launching

## Goals and Boundaries

Implement the complete directory or JAR -> `.janex` -> Java process workflow, supporting classpath
and module-path applications, together with OpenPGP and CMS signature generation and verification.

Use an existing Java runtime. Defer installation, SDK management, remote dependency acquisition,
global trust stores, and persistent verification caches. Keep the file format at version **0.1**.

## Specification and Module Boundaries

Update [FileFormat.md](docs/spec/FileFormat.md) before implementing the affected behavior:

- Skip an overlay's entire subtree when its condition does not match.
- Continue ignoring malformed language tags in `LocalizedText`, but require at least one well-formed
  tag and reject tags that are duplicates under case-insensitive comparison.
- Define the optional resource-root text attribute `janex.java.jar_name` to retain the original JAR
  filename for Java path materialization. It must be a single filename. Materialize each resource
  root in a separate directory to avoid filename collisions. Use `resources.jar` when absent.

Use five Rust crates and one Java bootstrap project:

- Use `janex-format` for container reading and writing, compression, resource trees, CLASSFILE
  transforms, format conditions, version comparison, and checksums. It does not depend on the
  Java runtime or signature libraries, CLI, networking, or installation management.
- Use `janex-signature` for CMS and OpenPGP signatures, key decoding and unlocking, certificate
  validation, and caller-supplied authentication policy. It owns its parsing limits and errors.
- Use `janex-java` for runtime discovery and probes, bounded JAR reading, manifests, and native
  or bootstrap launch argument preparation. Its inputs do not contain Janex format types.
- Use `janex-bootstrap` for portable Java resource loading and entry invocation. The Host supplies
  an evaluated resource index over its verified snapshot; Java does not repeat launch policy.
- Use `janex-host` to orchestrate local packaging, load trust material, apply execution policy,
  prepare resources, select compatible runtimes, and own temporary files and process lifetimes.
  It converts format limits, conditions, and application descriptors into capability inputs.
- Use `janex-cli` for argument parsing, terminal interaction, and result presentation. Update
  [CLI.md](docs/spec/CLI.md) alongside the command implementation.

## Implementation Order and Interfaces

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
- Write shared string pools and blob data with XXH3-64 checksums for file entries. Use SHA-256 for
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
- Select runtimes in this order: explicit selection, `JAVA_HOME`, then `PATH`. Report an error when
  an explicitly selected runtime does not meet requirements; otherwise try the next candidate.
  Support classpath launching on Java 8; module launching requires Java 9 or later.
- Obtain a stable input snapshot and verify it completely once. Reuse the result during parsing
  and materialization for the current launch. Do not implement a cache shared across launches.
- Evaluate conditions, overlays, and resource layers against each candidate runtime. Check local
  modules and modules supplied by the runtime. Report missing dependencies without downloading
  content or invoking remote providers.
- For bootstrap entry points, write a private snapshot and resource index. Read ordinary
  Stored blobs, Extents, Zstandard frames, and CLASSFILE transforms on demand in Java. Preserve root
  order, resource enumeration, manifest package attributes, sealing, and service discovery.
- Load named and automatic modules directly through indexed module finders and readers, retaining
  original filenames, services, and access options. Keep system modules in the native boot layer.
- Expose resource URLs through a read-only NIO provider with path operations, directory traversal,
  seekable channels, metadata, and explicit view lifetimes. All readers share the verified snapshot.
- Materialize agents and direct-mode paths as JARs. Preserve the native agent lifecycle and keep
  direct classpath and module-path launching independently usable.
- Decode external-dictionary blobs in the Host until the portable Java decoder supports dictionaries.
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

## Java Bootstrap Validation

The root Gradle build uses Java 8 base classes and Java 9 module classes in a reproducible
multi-release JAR. The portable Zstandard decoder is implemented under MPL-2.0. `check` validates
the embedded artifact, and `:janex-bootstrap:updateEmbeddedBootstrap` updates it.

Cover NIO URI round trips, full directory traversal, channel positioning, write rejection,
metadata precision, closure and remounting on Java 8 and current Java. Cover direct module reads,
automatic modules, service discovery, access options, module-reader lifetimes, and agents on Java 9+.
The module bridge uses explicitly enabled JDK module-access APIs. Custom system loaders and
`--patch-module` use direct mode. Host authentication covers the complete snapshot once;
the Java readers do not add per-resource publisher authentication.

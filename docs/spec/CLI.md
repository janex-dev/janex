# Janex CLI Tool

This document is a draft of the Janex command-line interface.

The file format itself is specified in `docs/spec/FileFormat.md`. This document only describes the user-facing CLI behavior.

Application commands require a supported application descriptor. A generic Janex container is not
directly executable.

## Command Design

The CLI should separate software acquisition from software execution:

- `janex pack`: build a Janex application from local directories and JARs.
- `janex inspect`: inspect a local container without launching it or acquiring dependencies.
- `janex install`: acquire and register software; SDK installation is implemented, while Janex application installation remains planned.
- `janex run`: start an installed application or a local Janex file without implicitly treating remote content as trusted software.
- `janex available`, `list`, `update`, `uninstall`, `default`, `use`, `exec`, and `env`: manage SDK versions and execution environments.

This split keeps trust decisions at acquisition time and keeps the run path simpler and safer.

`janex run` may use a managed Java installation installed by `janex install`, but it must not download or install a Java runtime implicitly.
If no suitable runtime is available, `janex run` should report the missing requirement and point the user to the appropriate
`janex install` command.

## `janex inspect`

```text
janex inspect <FILE>
    [--sections] [--blobs] [--resources] [--data-pools]
    [--json] [--verify] [--external-tail-length <BYTES>]
```

The default summary reports container and external-region sizes, section and pool counts,
application identities, and verification status. Standalone files, native prefixes, and appended
JAR/ZIP64 launchers are detected automatically. An explicit trailing length supports other wrappers.
Inspection never launches code, selects a runtime, or acquires external dependencies.

- `--sections` includes physical ranges, section types, recorded checksums, and metadata.
- `--blobs` includes every pool, table page, Stored entry, and Extents entry, including unused blobs.
  It decodes and validates table pages but does not decompress ordinary blob payloads.
- `--resources` includes local roots declared anywhere in supported application configurations,
  including agents and inactive or later-cleared path lists. All resource layers, conditions,
  directories, files, links, and tombstones remain unmerged. External references remain in application
  metadata. Unknown application descriptors are preserved but do not contribute inferred roots.
- `--data-pools` implies `--resources` and includes the root data pools and explicitly referenced
  CLASSFILE override pools. Reading resource structure decodes roots, directory-entry blobs, and
  default data pools; it does not restore ordinary file contents.
- `--verify` checks recorded metadata, section, and external-region checksums. Missing checksums
  reduce reported coverage; they are not failures. This does not authenticate CMS/OpenPGP signatures
  or check every restored file checksum. Table-page checksums are always checked when pages are read.

Malformed requested structures, unsupported required features, limit violations, and checksum
failures exit nonzero. There is no damaged-file recovery mode. No report is emitted before requested
parsing and verification succeed. Opening and inspecting a signed container does not require a
trusted signer; the report explicitly leaves signature authentication unchecked.

### Machine Output

`--json` emits one document with `schema_version: 1`. It uses the same detail selectors as text output.
Unrequested `sections`, `blob_pools`, `resource_roots`, and `data_pools` members are omitted.
Applications and original file metadata are included in the summary. Consumers should tolerate
additional members.

Each `data_pools` record contains a blob `reference` and an `entries` array in pool-index order.
Entries use `{"bytes_hex":"..."}` without assuming a text encoding.

Opaque IDs, byte sizes, physical offsets, and logical blob indices are decimal strings, preserving
unsigned 64-bit values in JavaScript. Ordinary bounded counts and method IDs are JSON numbers.
Physical ranges use absolute file offsets and byte lengths; `decoded_offset` in an extent is relative
to its decoded Stored blob and never identifies a physical file position.

CBOR metadata preserves key types and unknown fields: integers use `{"integer":"123"}`, byte strings
use `{"bytes_hex":"..."}`, and maps use `{"map":[[key,value],...]}`. Text, arrays, booleans, and null
use their JSON counterparts. Tags, floats, and other simple values use their exact original encoding
as `{"cbor_hex":"..."}`. `not_checked` is distinct from `passed`; successful checksum verification
does not imply complete secure coverage or trusted signatures.

### Library Access

Tools can use `janex-format` directly: `Reader::section_range` returns absolute section ranges;
`BlobStore::pool_info` exposes the page directory without loading its pages; `BlobStore::entry`
returns Stored or Extents descriptions. `Application::resource_references` enumerates unique local
roots across all Java configuration branches without evaluating conditions. Existing
`ResourceRoot::decode` and `DataPool` APIs expose unmerged resources and indexed byte sequences.
These operations use the normal format parser and reader limits; the CLI does not maintain a
separate binary parser. They do not establish publisher trust.

## `janex pack`

```text
janex pack <SOURCE> --output <FILE>
    [--class-path <PATH>]...
    [--module-path <PATH>]...
    [--main-class <NAME>]
    [--main-module <NAME>]
    [--application <ID>]
    [--jvm-option <ARG>]...
    [--external-class-path <URI> <CHECKSUM>]...
    [--external-module-path <URI> <CHECKSUM>]...
    [--argument <ARG>]...
    [--java-version <VERS>]
    [--with-launcher]
    [--cms-certificate <FILE> --cms-key <FILE>]
    [--cms-algorithm <ALGORITHM>]
    [--openpgp-key <FILE>]
    [--openpgp-signing-key <FINGERPRINT>]
    [--openpgp-algorithm <ALGORITHM>]
    [--key-password-file <FILE>]
```

`SOURCE`, `--class-path`, and `--module-path` take local directories or JARs. Each input becomes a separate
resource root. The primary input comes first on the module path when `--main-module` is present,
otherwise first on the classpath. Repeated path options retain their order. JAR filenames are
preserved; directory inputs use `resources.jar`.

`--external-class-path <URI> <CHECKSUM>` and `--external-module-path <URI> <CHECKSUM>` append
external declarations after the corresponding local inputs, without downloading them. Repeat an
option to preserve declaration order. `CHECKSUM` is `none` or `algorithm:hex`, where the algorithm
is `xxh3-64`, `xxh3-128`, `sha256`, `sha512`, or `sm3`. The digest covers the complete original JAR.
For example, `--external-class-path pkg:maven/org.example/library@1.2.3 sha256:<64-hex-digits>`.

`--main-class` overrides entry-point inference. Otherwise the main manifest supplies `Main-Class`;
for classpath launching, a module descriptor's main class is used when the manifest supplies none.
Different main classes in versioned descriptors require an explicit entry point. With
`--main-module` and no main class, the selected module supplies its main class at launch.
An input without a determinable entry point is an error. `--application` defaults to `main`.

Each `--jvm-option` and `--argument` contributes one complete argument, including empty strings.
No shell splitting, variable substitution, or wildcard expansion is performed. Use forms such as
`--jvm-option=-ea` and `--argument=--verbose` for values starting with a hyphen.
`--java-version` takes a `vers:jep322` range, such as `vers:jep322/>=21|<26`.

The packer preserves file bytes and available POSIX permission bits, imports symbolic links without
following them, and maps Multi-Release JAR versions to conditional layers. Timestamps are omitted.
Manifest `Class-Path` entries do not cause dependency acquisition; supply dependencies explicitly.
Malformed paths, duplicate entries, unsupported filesystem nodes, and corrupt archives are errors.

File entries use XXH3-64 checksums; sections, blob-table pages, and Checksum verification use SHA-256.
The output constrains both external regions to be absent. A shared string
pool and identical-file blob reuse reduce repetition within each root. Zstandard is used where it
reduces storage. CLASSFILE transforms are selected only when their complete candidate package is
smaller, including data-pool and index costs. Unrecognized class files remain ordinary resources.
Unsigned output bytes are reproducible given unchanged inputs, permission bits, options, and encoder
versions. Signatures may include the current time or randomness.

### Signing Keys

Choose either OpenPGP or CMS signing; the two options are mutually exclusive. Without a signing
key, the output uses Checksum verification.

Encrypted keys prompt for a hidden password when standard input is a terminal. Otherwise,
`--key-password-file` is required. One final LF or CRLF is removed from that file; other bytes
are retained. Passwords are never accepted as command-line values. Excessive encoded key-derivation
costs fail before decryption. Key and trust files are individually limited to 4 MiB.

### Standalone Java Launcher

`--with-launcher` appends an executable bootstrap JAR using the existing JAR Tail Wrapper. The
package metadata records its length and SHA-256 checksum. The format remains version 0.1.

```shell
janex pack app.jar --output app.janex --with-launcher
java -jar app.janex arg1 arg2
```

The Java reader selects the application and resources, then starts a child JVM from the same Java
installation with the package's JVM options and existing resource loader. Preset arguments precede
user arguments; exit status is propagated and temporary files are removed after exit. Java 8+
supports classpath applications; modules require Java 9+. `-Djanex.application=ID` selects a target.
Options supplied before `-jar` are forwarded to the child before the package's options. Arguments
already altered by the platform's initial Java launcher cannot be recovered.

External classpath and module-path declarations support HTTP(S) JARs and exact Maven PURLs, using
the same cache directory, keys, and checked records as `janex run`. Multi-release resources and
original JAR filenames are retained. Supply these properties before `-jar`:

| Property | Default | Meaning |
| --- | --- | --- |
| `janex.dependencyCache` | `<JANEX_HOME>/cache/dependencies` | Override the shared dependency cache directory. |
| `janex.offline` | `false` | Use verified cache entries without network access. |
| `janex.refreshDependencies` | `false` | Replace cached entries after a successful download; incompatible with offline mode. |
| `janex.mavenRepository` | `https://repo.maven.apache.org/maven2/` | Default Maven repository. |

These settings govern preparation in the initial JVM; package JVM options cannot override them.
Downloads allow five redirects, prohibit HTTPS downgrade, and are bounded to 60 seconds and 512 MiB
per dependency. HTTP requires a SHA-256, SHA-512, or SM3 checksum. HTTPS may omit it; declared
checksums support all five Janex algorithms and are verified on downloads and cache hits. No POMs or
transitive dependencies are resolved. Offline cache misses and corruption fail; online corruption
triggers reacquisition. Cache publication is atomic.

The standalone reader accepts None or Checksum verification and checks all recorded container and
blob-page digests using their declared algorithms. Missing checksums do not establish integrity.
Raw and trained external Zstd dictionaries are supported, including Stored and Extents dictionary
sources. Dictionary-backed resource data is decoded during preparation, matching the native Host.
ZIP64 dependency JARs and tail discovery are supported. Direct `java -jar` still depends on the initial
JVM accepting the prefixed archive; use `janex run` or the Java `Standalone.launch(Path, String[])`
API for ZIP64 tails rejected by that JVM. `--with-launcher` produces an ordinary ZIP tail.
Selected local and remote Java agents are prepared as JARs before the child starts. Their order,
unsplit options, manifest capabilities, and native `premain` behavior are retained; temporary agent
files are removed after the child exits or preparation fails. Recorded agent-file checksums are
verified before manifest rewriting and before any selected agent executes.
Virtual Java module requirements are checked against the current runtime and indexed module path,
including exact descriptor versions. Module resolution is validated before any selected agent or
application entry point executes. Virtual requirements do not download module providers; selected
module paths require Java 9 or later.
Signing options cannot be combined with `--with-launcher`. Default standalone launching rejects
signed packages and startup options requiring direct mode.
The reusable Java reader APIs separate container parsing, recorded-integrity
checks, and caller authentication policy; parsing a signature does not authorize execution.
The runtime resource index does not repeat per-file checksums, matching native bootstrap launching.
This entry point does not establish publisher trust:
the appended launcher executes before it can check the package. Existing `janex run` bootstrap and
direct modes remain available, including their signature and remote-dependency capabilities.

### OpenPGP Signing

`--openpgp-key` accepts one binary or ASCII-armored transferable secret key. The newest eligible
secret signing subkey is selected by default, falling back to the primary key. Equal creation times
are ordered by ascending fingerprint. `--openpgp-signing-key` selects a primary key or subkey by its
complete hexadecimal fingerprint, ignoring letter case. Selection checks certifications, signing
usage, expiration, and supplied revocations before requesting a password for the selected key.

`--openpgp-algorithm` accepts `rsa-sha256`, `rsa-sha512`, `ecdsa-p256-sha256`, `ecdsa-p384-sha384`,
`ed25519-sha256`, or `ed25519-sha512`. The default is SHA-256 except P-384 uses SHA-384. RSA keys must
contain 2048–8192 bits. The output contains one binary detached document signature, with protected
creation time and signing-key fingerprint. Its packet version follows the selected key, either 4 or 6.

Encrypted keys may use AES with CFB/SHA-1 integrity or AEAD protection. Salted and iterated password
derivation accept SHA-256/384/512, plus SHA-1 for version-4 keys. Argon2 requires AEAD and is limited
to 256 MiB, 10 passes, 16 lanes, and 1 GiB of memory times passes. Other protection schemes are
rejected. Unlocking checks that the private parameters match the selected public key.

### CMS Signing

`--cms-certificate` and `--cms-key` must be supplied together. The certificate may be DER or PEM;
the matching private key must be PKCS#8, in DER or PEM, optionally encrypted with PBES2.
`--cms-algorithm` accepts `rsa-sha256`, `rsa-sha512`, `ecdsa-p256-sha256`, or `ecdsa-p384-sha384`.
The default is SHA-256 for RSA and P-256, or SHA-384 for P-384. RSA keys must contain 2048–8192 bits.

The output contains one detached CMS envelope with protected content type, digest, and algorithms.
The embedded certificate identifies its signer; it does not establish trust. Certificate validity and
code-signing usage are checked before signing.

### Output

`--output` is required and must not exist. The packer writes temporary files beside the destination,
then publishes the completed result without replacing an existing file.

```text
janex pack app.jar --output app.janex --class-path lib/dependency.jar
janex pack classes --output app.janex --main-class example.Main --jvm-option=-ea
janex pack app.jar --output app.janex --main-module example.app --argument=--verbose
janex pack app.jar --output app.janex --cms-certificate signer.pem --cms-key key.pem
janex pack app.jar --output app.janex --openpgp-key signer.asc
```

## Application Installation (Planned)

`janex install` installs a Janex application package from a URI or a local file.

### Synopsis

```text
janex install [OPTIONS] <TARGET>
```

### Description

The `install` subcommand is responsible for acquisition, validation, and trust establishment.

At a high level, the command should:

1. Resolve the target from a URI or local file.
2. Download or read the Janex file.
3. Validate the file structure, integrity information, and signatures when available.
4. Require a `package_name` and use it to identify a new installation or an update within its
   publisher or source scope.
5. Inspect every supported `Application` section for launch-sensitive features such as remote
   dependencies, Java agents, and embedded JVM options.
6. Require explicit user consent when policy-sensitive actions are involved.
7. Record a local installed copy together with its source and trust metadata.
8. Register each supported `Application` section by (`package_name`, `application_id`) within that
   scope and honor its `ApplicationIntegrationObject` requests when policy allows. Each generated
   command or launcher resolves to that pair.

### Arguments

#### `<TARGET>`

The Janex application package to install.

This value may be either:

- A Janex file URI.
- A local file name or path.

### Options

The exact option set may evolve, but the `install` subcommand is expected to support the following categories:

- Installation location selection.
- Update or replacement behavior for an installed package with the same `package_name`.
- Trust and policy overrides, such as non-interactive approval flags.
- Diagnostics output for validation and signature results.

### Examples

Install from a remote URI:

```text
janex install https://example.com/app.janex
```

Install from a local file:

```text
janex install app.janex
```

### Exit Status

- `0`: the package was installed successfully.
- Non-zero: installation failed or was rejected by policy.

## `janex run`

```text
janex run [OPTIONS] <TARGET> [ARGS...]
```

`TARGET` is a local Janex file path or a local `file:` URI without a query or fragment.
Downloading the target itself and installed-package lookup are outside this execution interface.
All arguments after the target are forwarded to the application, including empty strings,
`--`, and arguments resembling Janex options. Janex options must precede the target.

### Options

- `--application <ID>` selects an application section; otherwise the file must contain exactly one.
- `--java <PATH>` selects a Java executable or a bare executable name on `PATH`.
- `--java-home <PATH>` selects the Java executable under that home. It conflicts with `--java`.
- `--launch-mode <bootstrap|direct>` selects entry-point invocation. The default is `bootstrap`.
- `--offline` resolves external dependencies only from the local cache.
- `--dependency-cache <DIRECTORY>` overrides the platform user cache directory.
- `--refresh-dependencies` downloads dependencies again and replaces cache entries after verification;
  it conflicts with `--offline`.
- `--maven-repository <URL>` overrides Maven Central for PURLs without a `repository_url` qualifier.
- `--allow-unsigned` permits local files using None or Checksum verification. It never bypasses
  authentication for a signed file or an explicit signer requirement.
- `--trust-cms-certificate <FILE>` requires that signer certificate. Repeat it to require every
  listed signer. Embedded certificates and system trust stores do not add trusted signers.
- `--cms-issuer <FILE>` supplies an issuer certificate for offline CRL authentication without
  adding a trusted application signer. It requires `--trust-cms-certificate`.
- `--cms-crl <FILE>` supplies a complete direct X.509 v2 CRL in DER or PEM. Repeat for additional
  lists. It requires `--trust-cms-certificate`.
- `--trust-openpgp-key <FILE>` pins one binary or armored OpenPGP public-key certificate and its
  valid signing subkeys. It conflicts with `--trust-cms-certificate` and rejects other verification types.

CMS authentication checks each required signature, certificate validity, signing usage, and
supported key strength. Unknown critical certificate extensions fail. Supplied or embedded CRLs
matching a required certificate's issuer must be current and authenticated; revoked signers fail.
Issuer keys must verify both the pinned certificate and its CRL. Delta, partitioned, and indirect
CRLs are unsupported. No CRL means no revocation-status assertion, and no revocation data is fetched.

OpenPGP authentication checks the document signature, self-certifications, signing flags, and
key/signature validity both when the signature was created and at the current time. Signing subkeys
need bindings in both directions. The newest applicable self-signature supplies policy; expiration
does not reactivate older policy. Supplied primary-key and subkey revocations apply permanently and
retroactively. Certification revocations suppress older certifications. Third-party certifications
do not establish trust. Designated revokers and mixed primary/subkey packet versions are unsupported.
Only supplied revocation information is checked; no network or global keyring is consulted.

### Execution

The launcher reads a bounded, owned snapshot and verifies container and external-region checksums
once before selecting a runtime. The result is reused for this launch. Later changes to the source file do not
change the prepared invocation; there is no verification cache shared across launches.
Signed files additionally require secure checksum coverage of every section and both external regions.

Runtime selection tries an explicit override, `JAVA_HOME`, then Java executables on `PATH`.
An explicit override fails without fallback. Otherwise, failed probes, mismatched conditions,
and unsatisfied local module requirements cause the launcher to try the next candidate.
Classpath launching supports Java 8; module launching requires Java 9 or later.

Bootstrap mode restores program arguments from private UTF-16 launch data before invoking the
application, preserving characters that the Windows Java launcher's ANSI conversion would lose.
It supports classpath and module entry points, including Java 25 instance and no-argument main
methods. Direct mode invokes the native application entry point without this layer and retains
the runtime's argument-conversion limits. Both modes preserve argument order and boundaries.
On Unix, non-Unicode native argument bytes require direct mode. JVM options, Java paths, and
agent options use the native launcher in both modes. A custom `java.system.class.loader` or
`--patch-module` requires direct mode.

Conditions, overlays, and resource layers use the selected runtime and invocation `run`.
Bootstrap entry points load classpath and module resources on demand from the verified snapshot,
using a Janex system class loader. Resource URLs support `Paths.get(uri)` and read-only NIO access.
The application JVM prepares the resource index from selected root references before agents or main
run. The fixed bootstrap JAR is reused from `JANEX_HOME/cache/bootstrap`; launch data is passed through
a private JVM property, with environment chunks for larger payloads. Native process limits still apply.
On Java 9+, application modules occupy a child of the native boot layer; module access options are
applied to that layer through the JDK module-access bridge. Agents and direct launches use
temporary JARs. Original filenames are retained for automatic-module naming. Module requirements use
the selected Java runtime and resolved module-path entries; virtual module requirements do not trigger
provider discovery or dependency downloads by themselves.
Symbolic links expand into resource contents; dangling links, cycles, and root escapes fail.

### External dependencies

After package authentication and runtime condition evaluation, the Host resolves explicit HTTP(S)
JAR URLs and canonical Maven PURLs for classpath, module-path, and agent entries. HTTP URLs must
end in a `.jar` filename; query strings are allowed. Maven PURLs require a group, artifact, and
exact release or timestamped snapshot version. `classifier`, JAR-producing `type`, and
`repository_url` qualifiers are supported. Floating versions, `-SNAPSHOT` metadata resolution,
PURL subpaths, and transitive POM resolution are not implemented.

Declared checksums are verified before importing or caching the raw JAR. Signed packages require
a secure checksum for each external dependency; plain HTTP also requires one. HTTPS verifies
server certificates. Up to five redirects are followed, with no HTTPS downgrade, URL credentials,
or non-HTTP(S) redirects. Downloads have size and time limits; failed downloads never publish partial
cache entries. Manifest `Class-Path` does not trigger additional downloads.
Per-entry operating-system locks serialize concurrent acquisition and are released when the process
closes the lock or exits; offline readers use shared locks.

Request metadata binds the original URI, resolved URL, and declared checksum. Original JARs are stored
under `files/sha256/<prefix>/<remaining-hash>/<filename>`, separately from CBOR metadata and locks.
Securely pinned Maven dependencies may reuse verified files from `~/.m2/repository` without modifying
that repository. Cache hits are copied into owned memory
and checked against both their stored SHA-256 digest and the declared checksum. A corrupt entry is
recovered online and rejected offline. Cache entries remain reusable until refreshed or removed;
their stored digest does not establish publisher identity. Java reads launch-owned data, so later
changes to the cache cannot change a prepared invocation. The cache defaults to
`<JANEX_HOME>/cache/dependencies`. `JANEX_HOME` defaults to `%USERPROFILE%/.janex` on Windows
and `$HOME/.janex` elsewhere. An explicit `JANEX_HOME` must be a nonempty absolute path;
the selected user home must also be absolute. Invalid values are errors when locating the cache.
Explicit dependency cache options take precedence. Local-only launches do not require this directory.

Runtime resources omit manifest `Class-Path`, JAR signature files, and signature-only manifest attributes.
Other manifest attributes, including sealing information, are retained. The original resources in
the Janex file remain unchanged. Temporary launch files are removed after the child process exits.

Java starts directly without a shell, inheriting the working directory and standard streams.
JVM options and agent options retain their argument boundaries. Preset program arguments precede
user arguments; `@` arguments are passed literally. Windowed applications suppress a console
window on Windows. The CLI propagates the child's exit code; Unix signal termination maps to
`128 + signal`. Preparation or process-launch failure returns `1`; invalid CLI syntax returns `2`.

### Examples

```text
janex run --allow-unsigned ./app.janex
janex run --allow-unsigned --application javac ./jdk-tools.janex --version
janex run --allow-unsigned --java-home /opt/jdk ./app.janex --config=config.toml
janex run --allow-unsigned --launch-mode direct ./app.janex
janex run --trust-cms-certificate signer.pem ./app.janex
```

## SDK Management

SDKs use the shared command namespace. They do not have a separate `java` or `sdk` command group.
Providers support Java distributions, Gradle, and Apache Maven; application installation from Maven is planned
separately.

```text
janex available bellsoft@21 [--refresh] [--offline] [--json]
janex install bellsoft@21 bellsoft@25 [--pin] [--offline] [--json]
janex install bellsoft@21 --path <JAVA_HOME>
janex install gradle@8 maven@3.9 [--pin]
janex install gradle@8 --path <GRADLE_HOME>
janex list [--json]
janex update bellsoft@21 [--json]
janex update --all [--json]
janex uninstall <EXACT_VERSION_OR_ID>
janex default <TARGET_OR_ID>
janex default --clear [--family java|gradle|maven]
janex current [--json]
janex home <TARGET_OR_ID>
janex init
janex activate
janex deactivate
janex use [TARGET_OR_ID...]
janex use --project <TARGET_OR_ID> [--pin]
janex pin <SAVED_REQUIREMENT>
janex unpin <SAVED_REQUIREMENT>
janex exec [--java <TARGET_OR_ID>] [--gradle <TARGET_OR_ID>] [--maven <TARGET_OR_ID>] -- <COMMAND> [ARGS...]
janex env [--java <TARGET_OR_ID>] [--gradle <TARGET_OR_ID>] [--maven <TARGET_OR_ID>] --shell <sh|powershell|fish>
```

`available`, `install`, `pin`, `unpin`, and explicit `update` requests support `--arch`, `--kind jdk|jre`,
`--javafx`, and `--libc glibc|musl`. `java:<vendor>@<version>` is the full target syntax;
recognized vendors such as `bellsoft`, `temurin`, and `zulu` may omit `java:`.
Installation IDs select exact platform variants without repeating variant options.
Gradle and Maven requests use `gradle@<version>` and `maven@<version>` and reject Java variant options.
One or two numeric components select a tool series; three select an exact release. `latest` selects
the newest stable release. Pinning freezes the resolved release for any requirement.

Installing retains every previous version and does not change any family's global default. A Java feature
request such as `21` may advance within that series on `update`; a numeric release such as
`21.0.8` does not advance to another release. `--pin` prevents subsequent updates of that saved
request. Fixed build requests are also supported. `update --all` updates requests independently;
if a later request fails, earlier successful updates remain committed.

`install --path` registers one existing SDK without transferring directory ownership. Uninstall
rejects the current global default and SDKs protected by live Janex execution leases. External
unregistration leaves files untouched. Project files outside Janex's registry are not tracked as
persistent roots; an explicit project reference to an uninstalled SDK subsequently fails.

`use --project` updates one of `java`, `gradle`, or `maven` in `.janex-toolchains.toml`, preserving other families.
`--pin` stores an exact local installation ID. `exec`, `current`, and `env` resolve each family's
explicit selection, shell home variable, project selection, and global default, in that order.
Java additionally falls back to system discovery. `env` prints assignments for
the caller to evaluate; it cannot modify its parent shell.

`init` writes `shell/init.sh`, `shell/init.fish`, and `shell/init.ps1` into `JANEX_HOME` and prints
loading commands. These scripts bind the current executable path and use the initialized user
directory when `JANEX_HOME` is unset. Repeated initialization updates only those scripts, preserving
SDKs, caches, state, and other files; it never edits shell profiles. Each script is replaced atomically,
but a failure may leave earlier scripts updated. Loading a script adds the executable's directory to
PATH and defines a shell function without selecting SDKs. With integration loaded, `activate` applies SDK selections and
`use <targets...>` selects SDKs in the current terminal; `use` without targets
clears manual selections and applies the current project's configuration and defaults. Native
`use` without integration reports an error instead of silently writing project configuration.
Shell selection order is manual selection, project, global default, then the original home variable.
`deactivate` restores the activation-time environment and retains the function for later activation.
`activate` retains existing manual selections; `use` can activate an inactive environment. Failed native
resolution emits no environment changes. Directory changes do not automatically trigger selection.

`run` and native application launchers ignore project toolchain files. They include managed SDKs
in Java discovery and retain a lease for the selected installation while the execution plan exists.
Explicit Java executable/home overrides prevent fallback. Automatic candidates include `JAVA_HOME`,
the global default, other managed SDKs, and PATH. Without a global default, native-architecture
runtimes are preferred over emulated ones. Application conditions still determine compatibility.
No execution command downloads an SDK implicitly.

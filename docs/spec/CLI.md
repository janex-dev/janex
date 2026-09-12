# Janex CLI Tool

This document is a draft of the Janex command-line interface.

The file format itself is specified in `docs/spec/FileFormat.md`. This document only describes the user-facing CLI behavior.

Application commands require a supported application descriptor. A generic Janex container is not
directly executable.

## Command Design

The CLI should separate software acquisition from software execution:

- `janex pack`: build a Janex application from local directories and JARs.
- `janex install`: acquire a Janex application package, validate it, present trust and policy decisions, and record a local installed copy.
- `janex run`: start an installed application or a local Janex file without implicitly treating remote content as trusted software.
- `janex java`: discover, install, select, and remove Java installations used by Janex.

This split keeps trust decisions at acquisition time and keeps the run path simpler and safer.

`janex run` may use a managed Java installation installed by `janex java`, but it must not download or install a Java runtime implicitly.
If no suitable runtime is available, `janex run` should report the missing requirement and point the user to the appropriate
`janex java` command.

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
smaller, including string-pool and index costs. Unrecognized class files remain ordinary resources.
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
| `janex.dependencyCache` | Platform user cache | Override the shared dependency cache directory. |
| `janex.offline` | `false` | Use verified cache entries without network access. |
| `janex.refreshDependencies` | `false` | Replace cached entries after a successful download; incompatible with offline mode. |
| `janex.mavenRepository` | `https://repo.maven.apache.org/maven2/` | Default Maven repository. |

These settings govern preparation in the initial JVM; package JVM options cannot override them.
Downloads allow five redirects, prohibit HTTPS downgrade, and are bounded to 60 seconds and 512 MiB
per dependency. HTTP requires a SHA-256 or SHA-512 checksum. HTTPS may omit it; any declared checksum
must use one of those two algorithms and is verified on downloads and cache hits. No POMs or
transitive dependencies are resolved. Offline cache misses and corruption fail; online corruption
triggers reacquisition. Cache publication is atomic. See [Dependency Cache](../DependencyCache.md).

The standalone profile requires SHA-256/SHA-512 integrity coverage. Signing options cannot be
combined with `--with-launcher`. The Java reader rejects signed packages, agents, virtual module
requirements, external Zstd dictionaries, ZIP64 JARs, and startup options requiring direct mode.
Per-file checksums are not repeated after the complete encoded sections have been verified.
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

## `janex install`

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

The cache key binds the resolved URL and declared checksum. Cache hits are copied into owned memory
and checked against both their stored SHA-256 digest and the declared checksum. A corrupt entry is
refetched online and rejected offline. Cache entries remain reusable until refreshed or removed;
their stored digest does not establish publisher identity. Java reads launch-owned data, so later
changes to the cache cannot change a prepared invocation. Cache defaults are
`%LOCALAPPDATA%/Janex/Cache/dependencies` on Windows, `~/Library/Caches/janex/dependencies` on macOS,
and `$XDG_CACHE_HOME/janex/dependencies` (or `~/.cache/janex/dependencies`) elsewhere.

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

## `janex java`

`janex java` manages Java installations available to Janex.

The command group is similar in purpose to SDKMAN-style Java management, but it is scoped to Janex runtime selection:

- It discovers locally installed Java installations.
- It lists installable Java distributions, versions, and runtime kinds from configured indexes.
- It installs, verifies, and removes managed JDK and JRE distributions.
- It records per-shell and default runtime choices.
- It provides runtime selection data for `janex run`.

Managed Java installations are stored in a Janex-controlled directory. The exact default location is platform-specific, but
implementations should allow it to be overridden by configuration or environment.

### Java Installation Identity

A Java installation should be identified by a stable runtime spec plus a runtime kind.

```text
<distribution>@<version>
```

The runtime kind is either `jdk` or `jre`.

Examples:

```text
temurin@21
zulu@17.0.12
graalvm-ce@21.0.2
```

When the distribution is omitted, the CLI should use the configured default distribution.

When the version is incomplete, the CLI may resolve it to the latest matching patch release according to the configured
Java index and local policy.

When a command accepts a runtime kind and the kind is omitted, the CLI should use the configured default kind.
If more than one installed runtime matches the same distribution and version, the command should reject the ambiguous
selection and ask the user to pass `--kind <jdk|jre>`.

### `janex java list`

Lists Java installations known to Janex.

#### Synopsis

```text
janex java list [OPTIONS]
```

#### Description

The command should show both managed Java installations installed by Janex and external runtimes discovered from the host system.

The output should indicate:

- Runtime spec.
- Runtime kind (`jdk` or `jre`).
- Java home.
- Distribution and version.
- Operating system and architecture.
- Whether the runtime is managed or external.
- Whether the runtime is the active default.

#### Options

- `--managed`: show only Janex-managed runtimes.
- `--external`: show only externally discovered runtimes.
- `--json`: print machine-readable JSON.

#### Examples

```text
janex java list
janex java list --managed
```

### `janex java available`

Lists Java versions available for installation.

#### Synopsis

```text
janex java available [OPTIONS] [VERSION]
```

#### Description

The command queries configured Java indexes and prints matching runtimes that can be installed for the current platform
unless overridden by options.

#### Arguments

##### `[VERSION]`

Optional Java feature version or full version filter, such as `17`, `21`, or `21.0.2`.

#### Options

- `--distribution <NAME>`: filter by distribution.
- `--kind <jdk|jre>`: filter by runtime kind.
- `--os <OS>`: filter by target operating system.
- `--arch <ARCH>`: filter by target CPU architecture.
- `--json`: print machine-readable JSON.

#### Examples

```text
janex java available
janex java available 21 --distribution temurin --kind jre
```

### `janex java install`

Installs a managed Java runtime.

#### Synopsis

```text
janex java install [OPTIONS] <JAVA>
```

#### Description

The command resolves a Java runtime spec, downloads the selected archive, verifies integrity metadata, unpacks it into the
managed runtime directory, and records the installed runtime.

Java runtime installation is a software acquisition operation. It should require explicit user consent when trust-sensitive
metadata is missing, when checksums cannot be verified, or when policy would otherwise reject the selected runtime.

#### Arguments

##### `<JAVA>`

The Java runtime spec to install, such as `temurin@21`, `zulu@17.0.12`, or `21`.

#### Options

- `--kind <jdk|jre>`: install a JDK or JRE runtime.
- `--set-default`: set the installed runtime as the default runtime after installation.
- `--force`: reinstall even if a matching runtime is already installed.
- `--name <ALIAS>`: assign a local alias to the installed runtime.
- `--json`: print machine-readable JSON.

#### Examples

```text
janex java install temurin@21 --kind jdk
janex java install 17 --kind jre --set-default
```

### `janex java uninstall`

Removes a managed Java runtime.

#### Synopsis

```text
janex java uninstall [OPTIONS] <JAVA>
```

#### Description

The command removes a Janex-managed Java runtime. It must not remove external runtimes discovered from the host system.

#### Arguments

##### `<JAVA>`

The managed Java runtime spec or alias to remove.

#### Options

- `--kind <jdk|jre>`: select the runtime kind when the spec is ambiguous.
- `--yes`: skip interactive confirmation.

#### Examples

```text
janex java uninstall temurin@17.0.12 --kind jre
```

### `janex java use`

Selects a Java runtime for the current shell or command context.

#### Synopsis

```text
janex java use [OPTIONS] <JAVA>
```

#### Description

The command prints shell commands that activate the selected runtime by setting `JAVA_HOME` and updating `PATH`.

Because a child process cannot directly mutate its parent shell environment, users should evaluate the printed script
using their shell-specific mechanism.

#### Arguments

##### `<JAVA>`

The installed Java runtime spec, external runtime identifier, or alias to activate.

#### Options

- `--kind <jdk|jre>`: select the runtime kind when the spec is ambiguous.
- `--shell <SHELL>`: output activation commands for a specific shell.

#### Examples

```text
janex java use temurin@21
janex java use --kind jre --shell powershell temurin@21
```

### `janex java default`

Gets or sets the default Java runtime used by Janex.

#### Synopsis

```text
janex java default [OPTIONS] [JAVA]
```

#### Description

Without an argument, the command prints the currently configured default runtime.
With an argument, it sets the default runtime used by Janex commands when no more specific runtime is selected.

#### Options

- `--kind <jdk|jre>`: select the runtime kind when the spec is ambiguous.

#### Examples

```text
janex java default
janex java default temurin@21 --kind jre
```

### `janex java current`

Prints the runtime that Janex would currently use.

#### Synopsis

```text
janex java current [OPTIONS]
```

#### Description

The command resolves the active Java runtime using the same precedence rules as `janex run`, without launching an
application.

#### Options

- `--json`: print machine-readable JSON.

#### Examples

```text
janex java current
```

### `janex java home`

Prints the Java home path for a selected runtime.

#### Synopsis

```text
janex java home [OPTIONS] <JAVA>
```

#### Options

- `--kind <jdk|jre>`: select the runtime kind when the spec is ambiguous.

#### Examples

```text
janex java home temurin@21 --kind jdk
```

### `janex java refresh`

Refreshes local Java indexes.

#### Synopsis

```text
janex java refresh [OPTIONS]
```

#### Description

The command updates local metadata used by `janex java available` and `janex java install`.

#### Options

- `--force`: ignore cache freshness and refresh all configured indexes.

#### Examples

```text
janex java refresh
```

### Runtime Selection Precedence

When a command needs a Java runtime, Janex should resolve it in the following order:

1. Explicit command-line override, such as `janex run --java-home <PATH>` or `--java <PATH>`.
2. Runtime selected by the current shell environment, such as `JAVA_HOME`.
3. Janex default runtime configured by `janex java default`.
4. Compatible managed runtime installed by `janex java install`.
5. Compatible external runtime discovered from the host system.

For a `janex.java` descriptor, evaluate [conditions](FileFormat.md#conditions) with the current host,
invocation `run`, and each candidate runtime. On mismatch, try lower-precedence candidates unless
the user explicitly selected the runtime. Resource layers use the selected runtime and launch context.

### Java Indexes

The CLI may support one or more Java indexes. An index describes installable distributions, versions, runtime kinds,
download URLs, platform support, checksums, and optional signatures.

The index format is outside the scope of this draft, but `janex java install` must not treat an unverifiable download as
trusted without policy approval.

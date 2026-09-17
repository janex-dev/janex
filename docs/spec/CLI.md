# Using Janex

Janex manages Java, Gradle, and Maven installations, installs Maven applications, and packages and runs Java applications.
Put `janex` (`janex.exe` on Windows) on your `PATH` to get started.

Use `janex --help` to list commands, or add `--help` to any command for its full option list:

```shell
janex pack --help
janex integration register --help
```

- [Manage SDKs](#manage-sdks)
- [Install applications](#install-applications)
- [Run an application](#run-an-application)
- [Package an application](#package-an-application)
- [Inspect a package](#inspect-a-package)
- [Register file-opening support](#register-file-opening-support)
- [Files and caches](#files-and-caches)

## Manage SDKs

### Install and choose a default

List Java products, then install a JDK and make it your default:

```shell
janex available java
janex available bellsoft/liberica-jdk@21
janex install bellsoft/liberica-jdk@21
janex default bellsoft/liberica-jdk@21
```

You can keep several versions installed, including different Java distributions:

```shell
janex install bellsoft/liberica-jdk@21 adoptium/temurin-jdk@25
janex install gradle@8 maven@3.9
janex list
janex current
janex home bellsoft/liberica-jdk@21
```

`list` shows installed versions and their IDs. `current` shows the SDKs selected for the current
environment. `home` prints an installation's directory.

Products use `publisher/product` names. `bellsoft/liberica-jdk`, `bellsoft/liberica-jre`, and
`bellsoft/liberica-nik` are separate products. `gradle` and `maven` are short aliases for
`gradle/gradle` and `apache/maven`.

Java has a separate default for each target platform. Gradle and Maven each have one portable
default. Installing another SDK keeps existing installations. Updating a request used as a default
moves that default to the new build, unless pinned. To clear the native Java default, use
`janex default --clear`. To clear the family/platform default of an installed SDK, use
`janex default --clear "bellsoft/liberica-jdk@21[arch=aarch64]"`. Its version and variant identify
an installation; the cleared default is shared by that installation's family and platform.
For Gradle or Maven, use `--family gradle` or `--family maven`.

### Select versions

The part after `@` determines which versions an install or update can choose:

| Request | Meaning |
| --- | --- |
| `bellsoft/liberica-jdk@21` | The latest available release of Java 21. |
| `bellsoft/liberica-jdk@21.0.8` | Java 21.0.8; updates stay on this release. |
| `bellsoft/liberica-jdk@21.0.8+12` | One specific Java build. |
| `adoptium/temurin-jdk@latest` | The latest stable release from that distribution. |
| `gradle@8` | A release in the Gradle 8 series. |
| `maven@3.9` | A release in the Maven 3.9 series. |
| `maven@3.9.9` | Exactly Maven 3.9.9. |

Omitting `@version` means `@latest`. Updates are explicit: run `janex update <PRODUCT>` to
advance that saved request. NIK versions identify NIK itself; `list --json` records its bundled
Java version separately as `java_version`.

### Keep several architectures or variants

Write variant and platform choices inside the target: `product@version[variant=...,arch=...]`.
Quote selectors containing brackets to prevent shell expansion. Each target carries its own
choices, whether installing one SDK or several:

```shell
janex install "bellsoft/liberica-jdk@21[variant=full,arch=aarch64]" "gradle@9[variant=all]"
janex default "bellsoft/liberica-jdk@21[variant=full,arch=aarch64]"
janex use "bellsoft/liberica-jdk@21[variant=full,arch=aarch64]"
janex exec --java "bellsoft/liberica-jdk@21[variant=full,arch=x86-64]" -- java -version
```

`available java` and `available gradle` list products and their variants. Liberica JDK offers
`standard`, `full`, and `lite`. Gradle offers `bin` (the default) and `all` (also includes sources
and documentation). Gradle variants can coexist and update independently, but share one default.
A registered Gradle `all` directory must contain both `docs` and `src`.

Java defaults for different platforms coexist. Omitted platform qualifiers select the operating
system's native platform, even when Janex runs under emulation. Explicit selections do not fall
back to another architecture; running a non-native architecture requires OS support.
Use `os=windows|linux|macos|freebsd` and, on Linux, `libc=glibc|musl`. Foreign-OS SDKs can be stored
and registered but cannot be activated on the current OS. Portable products such as Gradle and
Maven do not accept platform qualifiers.

The same selectors work with `available`, `home`, `use`, `default`, `update`, `pin`, `unpin`, and
`uninstall`. `update --all` updates each saved request independently. Operation options such as
`--offline` and `--timeout` apply to the whole command. Install validates all selectors before
starting, then installs them sequentially; a failure stops the command and retains completed installs.

`list` displays complete selectors and installation IDs. An ID selects one exact installation
and cannot carry additional qualifiers.

### Use SDKs in your shell

Run `janex init` once, then load the script for your shell. With the default Janex directory:

| Shell | Load command |
| --- | --- |
| Bash or Zsh | `. "$HOME/.janex/shell/init.sh"` |
| Fish | `source "$HOME/.janex/shell/init.fish"` |
| PowerShell | `. "$HOME/.janex/shell/init.ps1"` |

`init` prints the correct paths if you use a custom `JANEX_HOME`. Add the load command to your shell
profile if you want it in every session; Janex does not edit the profile for you.

Once loaded, activate your SDK selections or switch versions in the current terminal:

```shell
janex activate
java -version
janex use adoptium/temurin-jdk@25 gradle@8
janex deactivate
```

`activate` applies the project's selections and your defaults. An explicit `use` selection takes
priority. Run `janex use` without arguments to clear manual selections and apply the current
project's configuration again. Changing directories does not switch SDKs automatically.

`deactivate` restores the environment from before activation. You can activate it again later.
If you move the Janex executable, run `janex init` again and reload the script.

### Choose SDKs for a project or a single command

In your project directory, save the versions it needs:

```shell
janex use --project bellsoft/liberica-jdk@21
janex use --project gradle@8
```

These commands update `.janex-toolchains.toml`, one tool family at a time. Add `--pin` to save an
exact local installation ID instead of a version request. The selected SDK must already be installed.
Unspecified platform values remain unspecified in the project file, so another machine uses its own
native platform. Explicit `arch`, `os`, `libc`, and `variant` qualifiers are retained.

To run one command without changing your shell, use `exec`:

```shell
janex exec --java bellsoft/liberica-jdk@21 -- java -version
janex exec --java bellsoft/liberica-jdk@21 --gradle gradle@8 -- gradle build
```

For `exec`, explicit options take priority over SDK home variables, project selections, and global
defaults, in that order. Java can also fall back to a system installation. For scripts,
`janex env --shell sh` prints environment assignments; it also accepts `powershell` or `fish`
and the same `--java`, `--gradle`, and `--maven` selections as `exec`.

### Update, pin, and remove

```shell
janex update bellsoft/liberica-jdk@21
janex update --all
janex pin bellsoft/liberica-jdk@21
janex unpin bellsoft/liberica-jdk@21
```

Updates follow the saved version request and keep previous installations. Pinning holds a request
at its current build until you unpin it. You can also pass `--pin` when installing.

To remove an installation, use its exact version or ID from `janex list`:

```shell
janex uninstall <INSTALLATION_ID>
```

Choose another default or clear the current one before removing it. Janex also refuses to remove
an SDK in use by a Janex-managed process. If a project refers to a removed SDK, install it again or
change the project's selection.

To register an SDK you already have, use `janex install bellsoft/liberica-jdk@21 --path /opt/jdk-21`.
This also works for Gradle and Maven. Janex leaves that directory in place when you unregister it
with `uninstall`.

`available --offline` uses a cached catalog; `install --offline` reuses installed SDKs.
Use `available --refresh` to refresh the catalog. For slow downloads, `install` and `update` accept
`--timeout <SECONDS>`; the default is 1800 seconds per archive.

## Install applications

Install an application using a Package URL (PURL), or its Maven shorthand:

```shell
janex install maven:org.benf:cfr
cfr --help
janex install pkg:maven/org.benf/cfr@0.152
```

Load the scripts written by `janex init`, or add `JANEX_HOME/bin` to your `PATH`, to use installed
commands. Native entries forward arguments without a shell script. They use the same `JANEX_HOME`
as Janex; keep that variable set when using a custom home.

Both forms use the same canonical PURL in installation records. Maven Central is the default
repository. Without a version, Janex follows the repository's `<release>` metadata. An explicit
`@version` means that exact release, including a literal `@latest`; omit the version to track updates.
Updates are explicit, and old versions remain:

```shell
janex install maven:org.benf:cfr@0.152
janex update maven:org.benf:cfr
janex pin maven:org.benf:cfr
janex unpin maven:org.benf:cfr
janex default maven:org.benf:cfr@0.152
janex run maven:org.benf:cfr@0.152 --help
janex list
```

The first installation creates the command. Installing another version keeps its current selection;
`default` changes it. A default chosen by request follows that request's updates; an `app-...` ID fixes
one installation. `default --clear <TARGET>` removes the command while retaining the installed files.
`uninstall` accepts an exact version or installation ID. Removing the command's selected release
removes its entry; it never silently switches to an older version. Running applications cannot be removed.

SDK and application targets can share one command:

```shell
janex install bellsoft/liberica-jdk@21 maven:org.benf:cfr pkg:maven/org.example/tool@1.0
```

Maven targets default to `.jar`. Select `.janex` explicitly with `type=janex`:

```shell
janex install 'pkg:maven/org.example/tool@1.0?type=janex'
janex install 'pkg:maven/org.example/tool@1.0?classifier=all&type=janex'
```

The name remains the Maven artifact ID. Janex uses the requested type and never tries a different
extension when a file is missing. PURL qualifiers belong to each target:

| Qualifier | Meaning |
| --- | --- |
| `classifier=all` | Select a classifier, such as a self-contained JAR. |
| `type=janex` | Download a `.janex` artifact instead of the default `.jar`. |
| `repository_url=https:%2F%2Fexample.org%2Fmaven%2F` | Use another HTTPS Maven repository. An absolute `file:` URL selects a local repository. |

Quote PURLs containing qualifiers to protect `&` and `?` from the shell. Qualifier values use URL
percent encoding; `+` is never decoded as a space. Output sorts qualifier keys, normalizes encoding,
and omits the default `type=jar` and Maven Central repository.

Maven shorthands also accept query qualifiers or per-target options, for example
`"maven:org.example:tool@1.0[classifier=all,command=my-tool]"`. The shorthand's `repository` option
maps to `repository_url`. A custom `command` is a local installation option stored separately from
the PURL. It defaults to the lowercase artifact ID. Command names use lowercase ASCII letters,
digits, hyphens, and underscores, starting with a letter. Existing unmanaged commands and names
owned by other products are not overwritten.

Application installation currently supports Maven PURLs with `type=jar` or `type=janex`, without
subpaths. Unsupported types and qualifiers are rejected before installation.

JARs are kept unchanged and must contain `Main-Class` and its class. POM dependency resolution and
snapshots are not implemented. JAR manifest `Class-Path`, `Launcher-Agent-Class`, `Add-Exports`,
`Add-Opens`, and `Enable-Native-Access` are not yet supported by this installation path.
Janex packages must contain one application. Signed packages still require explicit signer pins
through `janex run`; command entries do not store trust configuration. Installing a package never runs it.

Application downloads use the shared cache, but installations own separate files under
`JANEX_HOME/apps`. Clearing download caches does not remove installed artifacts. A package's remote
dependencies may still need their cache or network access at launch. `list --json` includes application
records under `applications`; `update --all` updates saved SDK and application requests, respecting pins.

## Run an application

For an unsigned package you trust:

```shell
janex run --allow-unsigned app.janex
janex run --allow-unsigned app.janex --config settings.toml
```

Put Janex options **before** the file name. Everything after it goes to the application, even
arguments such as `--help` or `--java`. Arguments stored in the package come first.
The application inherits your working directory and standard streams, and Janex returns its exit code.

`run` accepts a local path or a local `file:` URI. It does not download the application itself.
It finds a compatible installed Java runtime, including SDKs managed by Janex, but does not install
one automatically. Project toolchain files do not control application runtime selection.

To choose a particular runtime or an application inside a package:

```shell
janex run --allow-unsigned --java-home /opt/jdk-21 app.janex
janex run --allow-unsigned --java /opt/jdk-21/bin/java app.janex
janex run --allow-unsigned --application javac tools.janex --version
```

`--java` and `--java-home` are alternatives. An explicit choice disables fallback to another runtime.
`--application` is required when a package contains more than one application.
Classpath applications need Java 8 or later; modules need Java 9 or later.

### Signed packages

Supply the certificate or public key you trust:

```shell
janex run --trust-cms-certificate signer.pem app.janex
janex run --trust-openpgp-key signer.asc app.janex
```

Choose one signature type. Repeating `--trust-cms-certificate` requires every listed signer.
Janex does not automatically trust embedded certificates, system certificate stores, or global
OpenPGP keyrings. `--allow-unsigned` does not bypass signature checks on a signed package.

For CMS revocation checks, supply `--cms-crl <FILE>` and, if needed, `--cms-issuer <FILE>`.
Janex checks supplied revocation information; it does not fetch it from the network.

### Dependencies and launch modes

A package can refer to external JARs using HTTP(S) URLs or exact Maven coordinates. Janex downloads
those dependencies on first use and caches them. It does not resolve POMs or transitive dependencies.

| Option | Use |
| --- | --- |
| `--offline` | Use cached dependencies only. Missing or corrupt entries cause an error. |
| `--refresh-dependencies` | Download dependencies again. Cannot be combined with `--offline`. |
| `--dependency-cache <DIRECTORY>` | Choose a different dependency cache. |
| `--maven-repository <URL>` | Replace Maven Central as the default repository. |

The default `bootstrap` launch mode loads resources from the package and preserves Unicode program
arguments on Windows. Use `--launch-mode direct` for applications that need their own system class
loader or `--patch-module`, or runtimes that cannot use the bootstrap. Direct mode uses Java's native
launcher and is subject to its argument-encoding limits. On Unix, non-Unicode arguments need direct mode.

## Package an application

Package a JAR with a `Main-Class` manifest entry:

```shell
janex pack app.jar --output app.janex
```

For a directory of compiled classes, specify the main class:

```shell
janex pack build/classes --output app.janex --main-class example.Main
```

The output file must not already exist. Packages are unsigned unless you supply a signing key;
run them with `janex run --allow-unsigned app.janex`.

Add dependencies, JVM options, or preset program arguments as needed:

```shell
janex pack app.jar --output app.janex --class-path lib/library.jar
janex pack app.jar --output app.janex --jvm-option=-Xmx1g --argument=--verbose
janex pack app.jar --output app.janex --java-version 'vers:jep322/>=21|<26'
```

Repeat `--class-path`, `--module-path`, `--jvm-option`, or `--argument` to add more values.
Each argument is passed as one value; use `=` for values beginning with `-`.
Dependencies listed in a JAR's manifest `Class-Path` are not included automatically.

For a modular application, use `--main-module example.app`. This puts the primary input on the
module path. Add `--main-class` if the module does not declare one. `--application <ID>` sets the
application's ID inside the package; the default is `main`.

### Leave a dependency external

Use `--external-class-path` or `--external-module-path` with a URI and checksum:

```text
janex pack app.jar --output app.janex --external-class-path pkg:maven/org.example/library@1.2.3 sha256:<64-hex-digits>
```

Replace the example coordinates and digest with your dependency's values. The checksum covers the
original JAR. Supported names are `xxh3-64`, `xxh3-128`, `sha256`, `sha512`, and `sm3`;
`none` omits the checksum. Signed packages and plain HTTP dependencies require a secure checksum
(`sha256`, `sha512`, or `sm3`).

Maven references need an exact release or timestamped snapshot version. Floating versions and
`-SNAPSHOT` lookup are not supported. Use the PURL's `repository_url` qualifier for a repository
specific to that dependency. Packaging records the reference without downloading the JAR.

### Sign a package

For CMS, provide a certificate and its matching PKCS#8 private key, in PEM or DER format:

```shell
janex pack app.jar --output app.janex --cms-certificate signer.pem --cms-key key.pem
```

For OpenPGP, provide a binary or ASCII-armored secret key:

```shell
janex pack app.jar --output app.janex --openpgp-key signer.asc
```

Janex chooses an algorithm for the key. Use `--cms-algorithm` or `--openpgp-algorithm` to choose one
explicitly; `pack --help` lists the supported combinations. For OpenPGP, `--openpgp-signing-key`
selects a key by its full fingerprint.

Encrypted keys prompt for a password in the terminal. In a script, supply
`--key-password-file <FILE>` instead. CMS and OpenPGP signing cannot be combined.

### Add a launcher

With `--with-launcher`, the package can also be started by Java without an installed Janex executable:

```shell
janex pack app.jar --output app.janex --with-launcher
java -jar app.janex
```

This starts the application in a child JVM from the same Java installation; it does not search for
another JDK. Signing options cannot be combined with `--with-launcher`, and the Java launcher does
not support packages requiring direct launch mode.

Pass Java launcher settings before `-jar`, for example:

```shell
java -Djanex.offline=true -jar app.janex
```

The other settings are `janex.application`, `janex.dependencyCache`, `janex.refreshDependencies`, and
`janex.mavenRepository`. They select the application, cache directory, refresh behavior, and default
Maven repository respectively.

To make a native executable, supply a `janex-launcher` binary for the target platform:

```shell
janex pack app.jar --output app.exe --native-launcher janex-launcher.exe
```

Windows PE and Linux/FreeBSD ELF launchers are supported. They still need an installed Java runtime.
See [Native Launcher](../../janex-launcher/README.md) for build instructions and runtime overrides.

## Inspect a package

```shell
janex inspect app.janex
janex inspect app.janex --resources
janex inspect app.janex --sections --blobs --json
janex inspect app.janex --verify
```

`inspect` shows package sizes, sections, applications, and recorded verification information without
running the application or downloading dependencies.

| Option | Additional detail |
| --- | --- |
| `--sections` | Section types, file ranges, and metadata. |
| `--blobs` | Blob pools, table pages, and data references. |
| `--resources` | Declared files, links, and resource layers, including inactive conditions. |
| `--data-pools` | Shared data-pool entries; also enables `--resources`. |
| `--json` | Structured output for scripts and other tools. |
| `--verify` | Check recorded container checksums. This does not authenticate signatures or check every file. |

Native prefixes and appended JAR launchers are detected automatically. For other trailing data,
use `--external-tail-length <BYTES>`.

JSON output has `schema_version: 1`. Large IDs, offsets, and sizes are decimal strings to avoid losing
integer precision. A verification result of `not_checked` means the check was not performed.
For the underlying container structure, see the [file format specification](FileFormat.md).

## Register file-opening support

Add Janex to your desktop's file-opening options:

```shell
janex integration register --allow-unsigned
janex integration status
```

This example allows unsigned local packages. To require a signer, use the same
`--trust-cms-certificate` or `--trust-openpgp-key` options as `run` instead.

Registration applies to the current user and leaves your default application choice alone.
Windows gets an Open With entry; Linux gets MIME and desktop entries; macOS gets a handler app in
`~/Applications`. Use `--system` for a machine-wide registration, with the necessary permissions.

After moving Janex, register it again to update the executable path. To remove the registration:

```shell
janex integration unregister
```

Use `--system` again to remove a system registration. Janex reports conflicts if its registered
files or values have been edited elsewhere.

File associations use `janex open`, which accepts the same options as `run`. Packages can give
desktop opening different settings through `invocation=open`. If opening fails, run
`janex open` in a terminal with the same trust options and file to see the error; graphical error
dialogs are not yet provided. Linux and macOS handlers do not open a terminal for console applications.

### Direct execution on Linux

With `binfmt_misc` mounted, register a system handler to execute ordinary `.janex` files directly:

```shell
sudo janex integration register --system --binfmt --allow-unsigned
chmod +x app.janex
./app.janex
```

Janex also installs a `binfmt.d` rule for systemd to load at boot. Removing the system registration
removes this rule. Packages with native executable prefixes do not need binfmt support.

### Export files for an installer

```shell
janex integration export --output integration-files --system --executable /usr/bin/janex
```

`export` creates a new directory containing registration files and installation instructions for
the current platform. It does not register them. Set `--executable` to the final installed path;
on Windows, use an absolute Windows path. The installer is responsible for installing and removing
these files. macOS export uses the system AppleScript compiler.

## Files and caches

Janex stores SDKs, applications, command entries, state, shell scripts, and caches in `~/.janex` by default
(`%USERPROFILE%\.janex` on Windows). Set `JANEX_HOME` to an absolute path to use another directory.
The Janex executable itself can live elsewhere.

Dependencies are cached in `JANEX_HOME/cache/dependencies`. `--dependency-cache` for `run`, or
`-Djanex.dependencyCache` for `java -jar`, overrides that location. Janex can reuse checksum-verified
Maven artifacts from `~/.m2/repository` without modifying them.

Do not modify a running application's package or dependencies before it exits.

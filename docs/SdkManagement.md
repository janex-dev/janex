# SDK Management

SDK installation runs in the native Host and does not require an existing JVM. Java distributions
use the Foojay Disco API, with `bellsoft` mapped to `liberica`. Gradle uses the
[official release service](https://services.gradle.org/versions/all); Apache Maven uses its
binary distributions in [Maven Central](https://repo.maven.apache.org/maven2/org/apache/maven/apache-maven/).
It accepts ZIP and gzip-compressed tar archives. This is independent of the Janex file format.

## Commands

The shared command namespace is `available`, `install`, `list`, `update`, `uninstall`, `default`,
`current`, `home`, `use`, `pin`, `unpin`, `exec`, and `env`. Java requests use `java:<vendor>@<version>`; the `java:`
prefix is optional for recognized Java vendors. Portable tools use `gradle@<version>` and
`maven@<version>`. Installing the Maven SDK is separate from installing applications from Maven repositories.

`21` selects a Java feature series; `21.0.8` selects that numeric release, excluding later
patches; a version containing a build number selects that build. `--pin` freezes the saved
request's installation binding. GA releases are selected; early-access versions are not inferred.
`latest` selects across GA feature series. `pin` and `unpin` change the update policy of a saved request.
Vendor, architecture, JDK/JRE kind, JavaFX variant, and libc are part of selection identity.
For Gradle and Maven, one or two numeric components select a series (`gradle@8`, `maven@3.9`);
three select an exact release (`maven@3.9.9`). `latest` selects the newest stable release.
Use `--pin` to freeze any resolved release, including Gradle releases named with two components.
Java variant options are rejected for portable tools.
The default architecture is the operating system's native architecture, including when Janex
itself runs under emulation. Linux libc is selected independently of the Janex build target.
The Linux default is musl on Alpine and glibc elsewhere; `--libc` overrides that choice.

Installation retains every installed version and does not set a default. Updates re-resolve a
saved requirement and retain the previous installation. Exact pinned requests do not advance.
Each SDK family has an independent default, referring to a requirement or an exact installation ID.
`default --clear --family gradle` clears only Gradle's default; the omitted family is `java`.
Uninstall rejects active defaults and running Janex-managed uses. Cache cleanup never removes
installed SDKs. Registering an external Java home does not transfer ownership of its directory.
External contents remain under their original owner's control and are not frozen by a Janex pin.

## Storage and Execution

SDK trees reside in `JANEX_HOME/sdks/<family>/<installation-id>/`. A bounded CBOR registry under
`JANEX_HOME/state/` records installations, requests, and the default selection. Content is
published before the registry, and registry replacement is atomic under an operating-system
lock. A failed commit can leave an unreferenced SDK tree but cannot expose an incomplete install.
Per-installation shared locks protect prepared and running processes against uninstall.

Catalog metadata is disposable and stored under `JANEX_HOME/cache/sdk/`. Downloads require a
SHA-256 or SHA-512 checksum obtained over HTTPS. When Disco only provides SHA-1 for a GitHub
release asset, the provider obtains the asset's SHA-256 from the GitHub release API instead.
Missing secure checksums fail rather than accepting SHA-1 or an unchecked executable archive.
Gradle archives require the official SHA-256 sidecar; Maven archives require the SHA-512 sidecar.
This establishes integrity relative to the configured HTTPS services, not publisher signature
authentication. Archive downloads use bounded byte ranges with up to three attempts per range,
check every Content-Range, and verify the complete archive afterward. A server that ignores Range
may supply one complete response with Content-Length. Size and time limits apply; extraction rejects traversal,
special files, unsafe links, duplicate files, and excessive expansion.
`install` and `update` accept `--timeout` in seconds; the default archive deadline is 1800 seconds.

`exec` and `env` accept independent `--java`, `--gradle`, and `--maven` selections. They set
`JAVA_HOME`, `GRADLE_HOME`, and `MAVEN_HOME` for selected families and prepend their `bin`
directories to `PATH`. Tool launchers use the selected Java; Janex does not infer their supported
Java version range. `env --shell <shell>` prints shell-specific environment assignments;
the caller evaluates them to change its own shell. Ordinary commands never modify the parent shell.
Project selection uses optional `java`, `gradle`, and `maven` strings in `.janex-toolchains.toml`.
`use --project` updates one family and preserves the others. The file does not execute project code.
For each family, explicit selection precedes its shell home variable, project selection, and
global default. Java additionally falls back to system discovery. Installed application execution
does not read project toolchain configuration or select portable tools.

```sh
janex install bellsoft@21
janex install bellsoft@25
janex install gradle@8 maven@3.9
janex default bellsoft@21
janex default gradle@8
janex default maven@3.9
janex exec --java bellsoft@25 -- java -version
janex exec --java bellsoft@21 --gradle gradle@8 -- gradle --version
janex exec --maven maven@3.9 -- mvn --version
janex use --project bellsoft@21
janex use --project gradle@8
janex update bellsoft@21
janex list --json
```

## Shell Integration

Load integration once in the current shell or add the appropriate initialization command to its
startup file. Janex prints code and never edits startup files automatically.

```sh
eval "$(janex activate bash)"
```

```powershell
janex activate powershell | Out-String | Invoke-Expression
```

Zsh accepts `eval "$(janex activate zsh)"`; fish accepts `janex activate fish | source`.
The integration defines a thin `janex` shell function. Other commands pass through to the native
executable. `use` and `deactivate` evaluate environment output only when the native command succeeds.

```sh
janex use bellsoft@21 gradle@8
java -version
gradle --version
janex use
janex deactivate
```

`use <targets...>` changes only the named SDK families in this terminal. `use` without targets
clears manual selections and applies the current project and defaults. Shell selection order is
manual selection, project, global default, then the original home variable. Generated home variables
do not block a later project change. `use --project <target>` only saves project configuration;
`--pin` is valid with `--project` and saves an exact local installation ID.

Repeated switches remove previously inserted SDK bins from PATH. Manual PATH additions are retained
across switches. `deactivate` restores the activation-time PATH and SDK home variables, including
their absent/empty state, and removes the shell function. That explicit restoration replaces later
manual edits to these variables. Reinitialization retains the original snapshot. Internal session
state is carried by `JANEX_SHELL_STATE`; it does not modify global SDK defaults. No existing JVM is
required to initialize integration. Automatic directory hooks are not installed; run `janex use`
after changing projects.

`env --shell <shell>` remains a lower-level environment renderer without session restoration.

Shell activation is optional. It retains no process lease after the environment command exits; applications launched
outside Janex and background daemons surviving the foreground child are not tracked by Janex.
Windows `.cmd` and `.bat` launchers use standard command escaping; their own script semantics apply.
Project files are local selections rather than registered
installation roots. `use --pin` writes an exact local installation ID. Missing project SDKs fail
explicitly instead of silently selecting a different version.

# Janex CLI Tool

This document is a draft of the Janex command-line interface.

The file format itself is specified in `docs/spec/FileFormat.md`. This document only describes the user-facing CLI behavior.

## Command Design

The CLI should separate software acquisition from software execution:

- `janex install`: acquire a Janex application, validate it, present trust and policy decisions, and record a local installed copy.
- `janex run`: start an installed application or a local Janex file without implicitly treating remote content as trusted software.
- `janex java`: discover, install, select, and remove Java installations used by Janex.

This split keeps trust decisions at acquisition time and keeps the run path simpler and safer.

`janex run` may use a managed Java installation installed by `janex java`, but it must not download or install a Java runtime implicitly.
If no suitable runtime is available, `janex run` should report the missing requirement and point the user to the appropriate
`janex java` command.

## `janex install`

`janex install` installs a Janex application from a URI or a local file.

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
4. Inspect launch-sensitive features such as remote dependencies, Java agents, and embedded JVM options.
5. Require explicit user consent when policy-sensitive actions are involved.
6. Record a local installed copy together with its source and trust metadata.

### Arguments

#### `<TARGET>`

The Janex application to install.

This value may be either:

- A Janex file URI.
- A local file name or path.

### Options

The exact option set may evolve, but the `install` subcommand is expected to support the following categories:

- Installation location selection.
- Update or replacement behavior for an already installed application.
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

- `0`: the application was installed successfully.
- Non-zero: installation failed or was rejected by policy.

## `janex run`

`janex run` starts an installed Janex application or a local Janex file.

### Synopsis

```text
janex run [OPTIONS] <TARGET> [ARGS...]
```

### Description

The `run` subcommand is responsible for execution, not software acquisition.

At a high level, the command should:

1. Locate the installed application or open the local Janex file.
2. Read the launcher metadata from the selected target.
3. Detect the current platform and available Java runtimes.
4. Evaluate the root configuration condition to select the best runtime.
5. Build the final JVM invocation, including module path, class path, Java agents, and JVM options.
6. Start the target application and forward its exit code.

Remote URIs should be installed first instead of being executed directly by `janex run`.

### Arguments

#### `<TARGET>`

The Janex application to run.

This value may be either:

- An installed application identifier.
- A local file name or path.
- A local `file:` URI.

After the Janex file target appears on the command line, all remaining arguments are forwarded to the target application as-is.

This means Janex CLI options for the `run` subcommand must appear before `<TARGET>`.

#### `[ARGS...]`

Application arguments passed to the target Java program.

### Options

The exact option set may evolve, but the `run` subcommand is expected to support the following categories:

- Java runtime selection override, such as explicitly providing a Java executable or Java home.
- Offline and policy controls for local-file execution.
- Diagnostics output, such as printing the selected runtime, resolved configuration, or final JVM command.
- Logging verbosity control for troubleshooting launch failures.

### Examples

Run an installed application: 

```text
janex run com.example.app
```

Run a local Janex file:

```text
janex run ./app.janex
```

Pass application arguments:

```text
janex run ./app.janex --server.port=8080 --profile=prod
```

Run a Janex file with additional diagnostics:

```text
janex run --verbose app.janex
```

Arguments after the Janex file target are still forwarded to the application even if they begin with `-` or `--`:

```text
janex run app.janex --enable-feature --config=config.toml
```

### Exit Status

- `0`: the Janex application exited successfully.
- Non-zero: the launch failed, or the launched application exited with a non-zero status.

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

`janex run` must still evaluate the root configuration condition from the Janex file. If the selected runtime is
incompatible, it should continue searching lower-precedence candidates unless the user provided an explicit override.

### Java Indexes

The CLI may support one or more Java indexes. An index describes installable distributions, versions, runtime kinds,
download URLs, platform support, checksums, and optional signatures.

The index format is outside the scope of this draft, but `janex java install` must not treat an unverifiable download as
trusted without policy approval.

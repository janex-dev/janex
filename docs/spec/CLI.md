# Janex CLI Tool

This document is a draft of the Janex command-line interface.

The file format itself is specified in `docs/spec/FileFormat.md`. This document only describes the user-facing CLI behavior.

## Command Design

The CLI should separate software acquisition from software execution:

- `janex install`: acquire a Janex application, validate it, present trust and policy decisions, and record a local installed copy.
- `janex run`: start an installed application or a local Janex file without implicitly treating remote content as trusted software.

This split keeps the trust decision at install time and keeps the run path simpler and safer.

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
janex run app.janex
```

Pass application arguments:

```text
janex run app.janex --server.port=8080 --profile=prod
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

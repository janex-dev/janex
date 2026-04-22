# Janex CLI Tool

This document is a draft of the Janex command-line interface.

The file format itself is specified in `docs/spec/FileFormat.md`. This document only describes the user-facing CLI behavior.

## `janex run`

`janex run` starts a Janex application file.

### Synopsis

```text
janex run [OPTIONS] <TARGET> [ARGS...]
```

### Description

The `run` subcommand reads a Janex file, selects a suitable Java runtime, prepares the launch configuration, and starts the application.

At a high level, the command should:

1. Read the launcher metadata from the input Janex file.
2. Detect the current platform and available Java runtimes.
3. Evaluate the root configuration condition to select the best runtime.
4. Resolve any remote dependencies required at launch time.
5. Build the final JVM invocation, including module path, class path, Java agents, and JVM options.
6. Start the target application and forward its exit code.

### Arguments

#### `<TARGET>`

The Janex file to run.

This value may be either:

- A Janex file URI.
- A local file name or path.

After the Janex file target appears on the command line, all remaining arguments are forwarded to the target application as-is.

This means Janex CLI options for the `run` subcommand must appear before `<TARGET>`.

#### `[ARGS...]`

Application arguments passed to the target Java program.

### Options

The exact option set may evolve, but the `run` subcommand is expected to support the following categories:

- Java runtime selection override, such as explicitly providing a Java executable or Java home.
- Dependency resolution control, such as disabling network downloads or using cached artifacts only.
- Diagnostics output, such as printing the selected runtime, resolved configuration, or final JVM command.
- Logging verbosity control for troubleshooting launch failures.

### Examples

Run a Janex file:

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

### Notes

- `janex run` is responsible for starting an existing Janex file. It does not build or package one.
- Any future subcommands for building or inspecting Janex files should be documented in separate subsections of this document.

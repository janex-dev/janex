# Janex Roadmap

Janex aims to become a cross-platform, cross-language SDK manager, package manager, and build tool.
The `.janex` format will serve as a general application container for managed languages.

Janex currently packages and launches Java applications and manages Java, Gradle, and Maven
installations. The directions below describe future work; concrete implementation tasks and
acceptance criteria belong in [PLANS.md](PLANS.md).

## Long-Term Scope

- **SDK management:** install and update runtimes and toolchains, preserve multiple versions,
  select global and project defaults, and integrate with shells. Extend Java support to Node.js,
  Python, and additional ecosystems, covering workflows served by SDKMAN and nvm.
- **Package and application management:** resolve, lock, install, and update dependencies; install
  command-line applications with isolated environments; support temporary tool execution and
  package publishing. Cover workflows served by npm, uv, and `cargo install`, starting with
  application installation from Maven repositories.
- **Project and build management:** prepare reproducible development environments, support
  workspaces, run tasks, and provide dependency-aware scheduling, incremental builds, and build
  caching. Support projects combining multiple languages and progressively cover build workflows
  currently served by Gradle and Maven.
- **Application containers:** describe applications, runtime requirements, entry points,
  dependencies, and resources, with compression, integrity verification, and signatures. Support
  managed code alongside native libraries and executables selected for the target platform.

## Design Principles

- Share downloads, caches, installation records, version selection, command entry points, shell
  integration, and process execution across ecosystems.
- Keep package coordinates, version semantics, dependency resolution, lockfiles, installation
  layouts, and compilation behavior specific to each ecosystem.
- Keep SDK, package, and build capabilities independently usable. Consume existing ecosystem
  packages and tools directly; conversion to `.janex` is optional.
- Keep the container core generic. Resource trees, conditional layers, data pools, compression,
  and authentication form shared infrastructure; runtime entry points and language-specific
  transforms belong to explicit extensions.
- Distinguish launcher architecture, host architecture, and runtime architecture when selecting
  SDKs and application resources.
- Develop shared interfaces from working implementations in multiple ecosystems. Java is the
  first complete implementation; a second ecosystem will test which abstractions are reusable.

## Delivery Sequence

1. Complete the Java workflow: packaging, distribution, launching, SDK management, and application
   installation from Maven repositories.
2. Extend SDK management to Node.js and Python, including multiple installed versions and
   consistent project and shell selection.
3. Add ecosystem application installation, isolated execution, and project environment preparation.
   Integrate existing package and build tools while establishing cross-language workflows.
4. Implement ecosystem dependency resolution, locking, synchronization, and publishing, together
   with shared build scheduling and caching. Expand managed-language container support alongside
   the corresponding runtime integrations.

These stages express priorities, not release dates or promises of command-line compatibility.
The [file format specification](docs/spec/FileFormat.md) remains authoritative for implemented
container rules; roadmap goals do not introduce format requirements.

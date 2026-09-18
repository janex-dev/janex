# Janex Roadmap

Janex aims to become a cross-platform, cross-language SDK manager, package manager, and build tool.
The `.janex` format will serve as a general application container for managed languages.

Janex currently packages and launches Java applications and manages Java, Gradle, and Maven
installations, plus standalone JAR and Janex applications from Maven repositories. The directions below describe future work; concrete implementation tasks and
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

## Design References

| Reference | Design to study | Application to Janex |
| --- | --- | --- |
| [Docker / OCI](https://github.com/opencontainers/image-spec/blob/main/image-index.md) | Content identities and multi-platform publication indexes. | Separate package coordinates, version selection, platform variants, and content digests. Keep publication indexes in repository metadata. |
| [Nix](https://nix.dev/manual/nix/stable/package-management/profiles) | Independent installations, environment generations, atomic switching, and rollback. | Select environments through references to retained installations; keep dependencies reachable from retained environments during cleanup. |
| [mise](https://mise.jdx.dev/dev-tools/mise-lock.html) | Cross-language tool selection and platform-aware lockfiles. | Separate project SDK requirements from exact versions and platform artifacts selected during resolution. |
| [Coursier](https://get-coursier.io/docs/cli-appdescriptors) | Maven application installation, application descriptors, and channels. | Supply entry points and launch settings missing from ordinary packages while preserving PURLs as authoritative package identities. |
| [pnpm](https://pnpm.io/symlinked-node-modules-structure) | Content-addressed storage and independent dependency layouts. | Share stored dependency content while preserving each application's resolved dependency set. |
| [Bazel](https://bazel.build/remote/caching) | Build actions, action-result caches, and content-addressed storage. | Model build inputs and outputs explicitly for incremental builds and shared local or remote caches. |
| [BuildKit](https://github.com/moby/buildkit#exploring-llb) | Dependency-graph execution with separate build frontends. | Let language-specific build rules share scheduling and caching infrastructure. |
| [Flatpak / OSTree](https://docs.flatpak.org/en/latest/under-the-hood.html) | Separate application and runtime deployments, versioned content, and deduplication. | Inform runtime sharing, application updates, and desktop distribution. |

Prioritize Coursier, mise, and Nix for application installation, SDK selection, and environment
management. Study OCI, BuildKit, and Bazel as distribution and build support expand.

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

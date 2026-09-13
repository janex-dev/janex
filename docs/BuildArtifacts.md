# Build Artifacts

The **Build artifacts** GitHub Actions workflow builds all supported distributions
on pushes to `main`, pull requests, and manual runs. Download each target's artifact
from the workflow run page. Artifacts expire after 14 days and include an archive
and its SHA-256 checksum. Windows uses ZIP; Unix uses tar.gz to preserve executable
permissions.

CI and artifact builds use BellSoft Liberica JDK. Builds use JDK 25; compatibility
tests additionally use Liberica JDK 8.

Both workflows cache Cargo downloads, dependency build outputs, and installed Cargo
tools using `Swatinem/rust-cache`. Artifact caches are separated by target; CI caches
are separated by runner OS. Rust versions and Cargo manifests also contribute to
the cache keys. Zig retains its separate compiler cache.

| System | Architectures | Build tools | Contents |
| --- | --- | --- | --- |
| Linux | x86-64, ARM64 | cargo-zigbuild on Linux x64 | CLI and native launcher |
| Windows | x86, x64, ARM64 | MSVC; ARM64 builds on Windows ARM64 | CLI and native launcher |
| macOS | x64, ARM64 | Xcode on macOS ARM64 | CLI |

Linux binaries statically link musl and use mimalloc for Rust allocations and C
allocation functions. Windows binaries statically link the MSVC CRT; macOS links
the system libraries. Both retain Rust's default system allocator and the native
libraries' default allocation functions. The Java process retains its own allocator.

Mach-O native launcher prefixes are not implemented, so macOS artifacts contain
only the CLI. The CLI supports bootstrap, direct, and standalone `java -jar` packages.

## Local Build

Install JDK 25 and Rust, plus the target system's build tools. Use the same root
Gradle task for every target:

```shell
rustup target add aarch64-unknown-linux-musl
cargo install cargo-zigbuild --version 0.23.4 --locked
./gradlew packageArtifacts -PjanexTarget=aarch64-unknown-linux-musl
```

Linux requires cargo-zigbuild and Zig 0.15.2 on PATH; the workflow installs Zig with
`mlugg/setup-zig` and cargo-zigbuild with Cargo. On Windows, Rust discovers the
installed Visual Studio C++ tools automatically. For macOS, install Xcode's command
line tools. Supported `janexTarget` values are:

- `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`
- `i686-pc-windows-msvc`, `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`
- `x86_64-apple-darwin`, `aarch64-apple-darwin`

`packageArtifacts` builds the bootstrap JAR and Rust binaries, runs the Java
artifact checks, and creates a ZIP or TAR archive plus its checksum using Gradle.
The archives are written to `build/distributions/`. To build only the binaries,
use `assembleArtifacts`; they are written to `target/<target>/release/` unless
`CARGO_TARGET_DIR` is set. No Python installation is required for this workflow.

## Verification

The workflow checks executable architecture and runtime dependencies before upload.
Linux additionally checks that mimalloc overrides the C allocation symbols. Windows
checks that no dynamically linked MSVC runtime is required; macOS checks that all
dynamic dependencies are system libraries.

Launch checks cover packaged resources, bootstrap and direct modes, and `java -jar`.
They exercise Unicode arguments except in Windows direct and `java -jar` modes,
where the Java launcher's code-page limitation still applies. Linux ARM64 runs through QEMU;
macOS x64 runs through Rosetta. These emulated binaries launch the host's Java.
Windows ARM64 builds and runs the complete launch checks on `windows-11-arm`, using
an ARM64 JDK. Windows x86 launches the x64 runner's Java through WOW64.

To build and check the binaries without creating archives, run:

```shell
./gradlew checkArtifacts -PjanexTarget=aarch64-unknown-linux-musl
```

Linux checks require binutils and qemu-user for ARM64. Windows checks locate
`dumpbin` through Visual Studio's `vswhere`; macOS requires `lipo` and `otool`.

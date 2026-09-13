# Build Artifacts

The **Build artifacts** GitHub Actions workflow builds all supported distributions
on pushes to `main`, pull requests, and manual runs. Download each target's artifact
from the workflow run page. Artifacts expire after 14 days and include an archive
and its SHA-256 checksum. Windows uses ZIP; Unix uses tar.gz to preserve executable
permissions.

| System | Architectures | Build tools | Contents |
| --- | --- | --- | --- |
| Linux | x86-64, ARM64 | cargo-zigbuild on Linux x64 | CLI and native launcher |
| Windows | x86, x64, ARM64 | MSVC on Windows x64 | CLI and native launcher |
| macOS | x64, ARM64 | Xcode on macOS ARM64 | CLI |

Linux binaries statically link musl and use mimalloc for Rust allocations and C
allocation functions. Windows binaries statically link the MSVC CRT and use its
default allocator. macOS uses the system libraries and allocator. The Java process
retains its own allocator on every platform.

Mach-O native launcher prefixes are not implemented, so macOS artifacts contain
only the CLI. The CLI supports bootstrap, direct, and standalone `java -jar` packages.

## Local Build

Install JDK 25 and Rust, plus the target system's build tools. Use the same root
Gradle task for every target:

```shell
rustup target add aarch64-unknown-linux-musl
python -m pip install cargo-zigbuild==0.23.4 ziglang==0.15.2
./gradlew assembleArtifacts -PjanexTarget=aarch64-unknown-linux-musl
```

Linux requires cargo-zigbuild and Zig. For Windows, use a Visual Studio developer
environment configured for the target architecture. For macOS, use Xcode's command
line tools. Supported `janexTarget` values are:

- `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`
- `i686-pc-windows-msvc`, `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`
- `x86_64-apple-darwin`, `aarch64-apple-darwin`

Gradle builds the bootstrap JAR before invoking Cargo. Binaries are written to
`target/<target>/release/` unless `CARGO_TARGET_DIR` is set. Package them with:

```shell
python .github/scripts/package-artifacts.py aarch64-unknown-linux-musl
```

## Verification

The workflow checks executable architecture and runtime dependencies before upload.
Linux additionally checks that mimalloc overrides the C allocation symbols. Windows
checks that no dynamically linked MSVC runtime is required; macOS checks that all
dynamic dependencies are system libraries.

Launch checks cover packaged resources, bootstrap and direct modes, and `java -jar`.
They exercise Unicode arguments except in Windows direct and `java -jar` modes,
where the Java launcher's code-page limitation still applies. Linux ARM64 runs through QEMU;
macOS x64 runs through Rosetta. These emulated binaries launch the host's Java.
Windows ARM64 binaries receive structural checks on the x64 build runner; execution
still needs a Windows ARM64 host.

With `JAVA_HOME` and the inspection tools on PATH, run:

```shell
python .github/scripts/check-artifacts.py aarch64-unknown-linux-musl
```

Linux checks require binutils and qemu-user for ARM64. Windows requires `dumpbin`;
macOS requires `lipo` and `otool`.

# Janex

Janex packages and launches Java applications and manages Java, Gradle, and Maven installations.

The long-term goal is a cross-platform, cross-language SDK manager, package manager, and build tool,
with `.janex` as a general application container for managed languages. See the [roadmap](ROADMAP.md)
for planned capabilities and priorities.

## Getting Started

Place `janex` (`janex.exe` on Windows) in a directory on PATH. The optional `janex-launcher`
executable is a prefix for creating self-launching application packages.

```shell
janex install bellsoft@21
janex default bellsoft@21
janex init
```

`janex init` writes shell integration scripts into `JANEX_HOME/shell` and prints loading commands.
`JANEX_HOME` defaults to `~/.janex`; set it to an absolute path to use another directory.
The executable can reside elsewhere. SDK installations are separate from disposable dependency caches.
Shell startup files are not modified automatically.

Load `shell/init.sh` for Bash or Zsh, `shell/init.fish` for Fish, or `shell/init.ps1` for PowerShell.
For example:

```powershell
. "$HOME/.janex/shell/init.ps1"
janex activate
```

After moving the executable, run `janex init` again and reload the script.
See the [CLI specification](docs/spec/CLI.md) for packaging, launching, and SDK commands.

## Building from Source

Install JDK 25, Rust, and the platform's native build tools. Use the root Gradle Wrapper
(`.\gradlew.bat` on Windows):

```shell
./gradlew assembleRelease
./gradlew check
./gradlew installJanex -PjanexInstallDir=<absolute-directory>
```

Release binaries are written to `target/release/` unless Cargo output settings override it.
`installJanex` copies executables into the specified directory; run `janex init` separately.

Use `packageArtifacts -PjanexTarget=<rust-target>` to build, check, and package a distribution
under `build/distributions/`. Windows archives use ZIP; Unix archives use tar.xz and require `xz`.
Linux musl and FreeBSD cross-builds also require Zig and cargo-zigbuild.
Supported targets and tool versions are maintained in the [artifact workflow](.github/workflows/artifacts.yml).
Download prebuilt archives from its GitHub Actions runs.

## Versioning

`workspace.package.version` in `Cargo.toml` is shared by Rust, Java, and the Gradle plugin.
Development versions end in `-SNAPSHOT`. For a release, remove the suffix, refresh `Cargo.lock`
with `cargo check --workspace`, and commit both files before creating the matching `v<version>` tag.
Then set the next development version and refresh the lockfile. Release builds do not change versions.
The file format version is independent.

## Documentation

- [File format](docs/spec/FileFormat.md)
- [Program architecture](docs/Architecture.md)
- [Gradle plugin](janex-gradle-plugin/README.md)
- [Native launcher](janex-launcher/README.md)

See [LICENSE](LICENSE) for license terms.

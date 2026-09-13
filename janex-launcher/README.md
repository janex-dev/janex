# Native Launcher

`janex-launcher` opens its own executable and runs the embedded application through `janex-host`.
It preserves application arguments, packaged resources, dependency resolution, and both bootstrap
and direct Java invocation. Windows PE and Linux/FreeBSD ELF prefixes are supported. Mach-O and
Shell wrappers are separate distribution mechanisms.

Build the CLI and launcher with the root Gradle Wrapper:

```text
./gradlew assembleRelease
```

For static Linux musl builds and downloadable Actions artifacts, see
[Build Artifacts](../docs/BuildArtifacts.md).

Package a Windows executable:

```text
target/release/janex.exe pack application.jar --output application.exe --native-launcher target/release/janex-launcher.exe
```

On Linux, use `target/release/janex` and `target/release/janex-launcher`. Native outputs receive mode
`0755` on Unix. Build artifacts stay in ignored Cargo directories; no executable is stored in source.
`--with-launcher` can additionally append the existing JAR launcher for `java -jar` execution,
subject to its existing unsigned-package restriction.

`--native-launch-mode direct` selects the native Java entry point; the default is `bootstrap`.
At execution time, `JANEX_LAUNCH_MODE=bootstrap|direct` overrides this choice, and `JANEX_JAVA` selects
an explicit Java executable. Application arguments are never parsed as launcher options. The current
directory and standard streams are inherited. The condition invocation channel is `open`.

## Architecture

The launcher's architecture, the native system architecture, and the selected JVM's architecture
are independent. Java runs in a child process; its bitness never needs to match the launcher.
Automatic selection probes JAVA_HOME and PATH candidates and prefers a JVM matching the native
system. Other runnable JVMs remain fallback candidates. Explicit Java overrides retain priority.
Resource and launch conditions use the selected JVM's normalized `os.arch`, preventing an emulated
JVM from receiving native-system libraries it cannot load.

Windows uses `IsWow64Process2` to identify the native machine under WOW64 and ARM emulation, with
`GetNativeSystemInfo` only as a fallback on older Windows lacking that API. The native Windows calls
are confined to `janex-platform`; all parsing and launch orchestration forbid unsafe Rust.
An x86 launcher can therefore launch an x64 or ARM64 Java installation when supported by the OS.
Discovery currently uses JAVA_HOME and PATH, not a registry-wide JDK search.

With the `i686-pc-windows-msvc` Rust target and its native build tools installed:

```text
./gradlew assembleWindowsX86Launcher
./gradlew checkWindowsX86Launcher
```

The release launcher is `target/i686-pc-windows-msvc/release/janex-launcher.exe`. These tasks do not
change the architecture of the normal workspace build. The tests execute the actual x86 wrapper
with the installed Java, including Java 8 when `JANEX_TEST_JAVA8_HOME` is set. ARM64 emulation still
requires validation on a Windows ARM64 machine.

## Prefix and Trust

The physical layout is:

```text
[PE or ELF executable] [configuration] [configuration length: u32 LE] [JNXBOOT1] [JanexFile] [optional JAR]
```

Everything preceding `JanexFile` is its `external_header`, with exact size and SHA-256 coverage.
The configuration is a private deterministic CBOR map shared by this packer and launcher:

| Key | Value |
| --- | --- |
| 0 | Nonempty application ID |
| 1 | Invocation: `0` bootstrap, `1` direct |
| 2 | Trust: `0` unsigned, `2` OpenPGP, `3` CMS |
| 3 | Public trust material as a byte string; empty for unsigned |

Configuration is bounded to 4 MiB, the complete prefix to 256 MiB, and the input snapshot to 512 MiB.
The configuration and authenticated package are read from the same owned snapshot. Signed packages
embed the signing certificate or public OpenPGP certificate, never the private key, and must pass
the Host's existing authentication and complete-content checks. Unsigned packages explicitly record
unsigned execution policy. Runtime overrides cannot disable authentication.

Executing the native file authorizes its embedded policy. Embedded pins establish consistency with
that wrapper, not an independent publisher identity for a file obtained from an untrusted source.
Authenticode signing and its tail-discovery rules are not implemented; already signed PE inputs
are rejected. Unix launchers forward termination signals and retain temporary resources until Java
exits; Windows console events also reach the child while the launcher waits for cleanup.

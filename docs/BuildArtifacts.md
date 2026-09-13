# Linux Build Artifacts

The **Linux artifacts** GitHub Actions workflow builds the CLI (`janex`) and native
launcher (`janex-launcher`) on pushes to `main`, pull requests, and manual runs.
Download each target's artifact from the workflow run page. Artifacts expire after
14 days and contain a `.tar.gz` archive with executable permissions preserved,
plus its SHA-256 checksum.

Supported targets:

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`

Both binaries statically link musl. On Linux musl targets, mimalloc serves as the
Rust global allocator and overrides the C allocation functions used by native
dependencies. This does not change the allocator of the Java process they launch.

## Local Build

Install JDK 25, Rust, and Python, then run:

```shell
python -m pip install cargo-zigbuild==0.23.4 ziglang==0.15.2
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
./gradlew assembleLinuxMusl -PjanexTarget=aarch64-unknown-linux-musl
```

Gradle builds the bootstrap JAR before invoking cargo-zigbuild. The target defaults
to `x86_64-unknown-linux-musl`; binaries are written to `target/<target>/release/`
unless `CARGO_TARGET_DIR` is set.

On an x86-64 Linux host with `JAVA_HOME` set and `binutils` and `qemu-user` installed,
run the same checks used before uploading:

```shell
python .github/scripts/check-linux-artifacts.py aarch64-unknown-linux-musl
```

The checks inspect ELF architecture, static linking, and mimalloc's allocation
symbols, then exercise bootstrap, direct, and `java -jar` launches with resources
and Unicode arguments. ARM64 binaries run through QEMU and launch the host's Java.

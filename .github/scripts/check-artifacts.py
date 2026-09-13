# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

"""Checks distribution binaries and exercises supported Java launch modes."""

import os
import platform
from pathlib import Path
import subprocess
import struct
import sys
import tempfile


def run(*command, **kwargs):
    """Runs a command and returns its UTF-8 output, failing on a nonzero exit."""
    return subprocess.run(
        command, check=True, text=True, encoding="utf-8", stdout=subprocess.PIPE,
        timeout=120, **kwargs
    ).stdout


def main():
    """Validates binaries for the requested target and tests runnable targets."""
    target = sys.argv[1]
    machines = {
        "x86_64-unknown-linux-musl": "Advanced Micro Devices X86-64",
        "aarch64-unknown-linux-musl": "AArch64",
    }
    directory = Path(os.environ.get("CARGO_TARGET_DIR", "target")) / target / "release"
    windows = target.endswith("-windows-msvc")
    macos = target.endswith("-apple-darwin")
    suffix = ".exe" if windows else ""
    cli = (directory / f"janex{suffix}").resolve()
    launcher = None if macos else (directory / f"janex-launcher{suffix}").resolve()
    for binary in (cli,) if macos else (cli, launcher):
        if windows:
            data = binary.read_bytes()
            offset = struct.unpack_from("<I", data, 0x3C)[0]
            machine = {"i686": 0x14C, "x86_64": 0x8664, "aarch64": 0xAA64}[target.split("-")[0]]
            if data[:2] != b"MZ" or data[offset:offset + 4] != b"PE\0\0":
                raise RuntimeError(f"Invalid PE executable: {binary}")
            if struct.unpack_from("<H", data, offset + 4)[0] != machine:
                raise RuntimeError(f"Unexpected PE architecture: {binary}")
            imports = run("dumpbin", "/dependents", binary).lower()
            if any(name in imports for name in ("vcruntime", "msvcp", "ucrtbase", "api-ms-win-crt")):
                raise RuntimeError(f"Dynamic MSVC runtime dependency: {binary}")
            continue
        if macos:
            machine = "arm64" if target.startswith("aarch64-") else "x86_64"
            if run("lipo", "-archs", binary).strip() != machine:
                raise RuntimeError(f"Unexpected Mach-O architecture: {binary}")
            for line in run("otool", "-L", binary).splitlines()[1:]:
                if not line.strip().startswith(("/usr/lib/", "/System/Library/")):
                    raise RuntimeError(f"Non-system library dependency: {line}")
            continue
        header = run("readelf", "-hW", binary)
        if machines[target] not in header:
            raise RuntimeError(f"Unexpected architecture: {binary}")
        if "INTERP" in run("readelf", "-lW", binary):
            raise RuntimeError(f"Dynamic interpreter found: {binary}")
        if "(NEEDED)" in run("readelf", "-dW", binary):
            raise RuntimeError(f"Shared library dependency found: {binary}")
        symbols = {}
        for line in run("nm", "--defined-only", binary).splitlines():
            fields = line.split()
            if len(fields) == 3:
                symbols[fields[2]] = fields[0]
        for name in ("malloc", "calloc", "realloc", "free"):
            if name not in symbols or symbols[name] != symbols.get(f"mi_{name}"):
                raise RuntimeError(f"mimalloc does not override {name}: {binary}")

    if windows and target.startswith("aarch64-") and platform.machine().lower() not in ("arm64", "aarch64"):
        print(f"Verified PE architecture and static CRT: {target}; execution requires Windows ARM64")
        return
    runner = ["qemu-aarch64"] if target == "aarch64-unknown-linux-musl" else []
    java_home = Path(os.environ["JAVA_HOME"])
    java = java_home / f"bin/java{suffix}"
    javac = java_home / f"bin/javac{suffix}"
    env = dict(os.environ, JANEX_JAVA=str(java))
    with tempfile.TemporaryDirectory(prefix="janex-artifacts-") as temporary:
        root = Path(temporary)
        classes = root / "classes"
        classes.mkdir()
        source = root / "Main.java"
        source.write_text('''import java.io.InputStream;
import java.nio.charset.StandardCharsets;

/// Exercises a packaged resource and Unicode arguments.
public class Main {
    /// Prints the resource contents followed by the first argument.
    public static void main(String[] args) throws Exception {
        try (InputStream input = Main.class.getResourceAsStream("/message.txt")) {
            byte[] bytes = new byte[9];
            if (input == null || input.read(bytes) != bytes.length) {
                throw new IllegalStateException("Missing resource");
            }
            System.out.write((new String(bytes, StandardCharsets.UTF_8) + args[0])
                    .getBytes(StandardCharsets.UTF_8));
        }
    }
}
''', encoding="utf-8")
        run(javac, "--release", "8", "-Xlint:-options", "-d", classes, source)
        (classes / "message.txt").write_text("resource:", encoding="utf-8")
        run(*runner, cli, "--version")
        for mode in ("bootstrap", "direct"):
            # The direct Windows Java launcher cannot preserve arbitrary Unicode arguments.
            argument = "ASCII argument" if windows and mode == "direct" else "Unicode 参数 🚀"
            expected = f"resource:{argument}"
            application = root / f"application-{mode}{suffix}"
            native_options = [] if launcher is None else [
                "--native-launcher", launcher, "--native-launch-mode", mode
            ]
            run(*runner, cli, "pack", classes, "--output", application,
                "--main-class", "Main", *native_options, "--with-launcher")
            actual = run(*runner, cli, "run", "--allow-unsigned", "--java", java,
                         "--launch-mode", mode, application, argument)
            if actual != expected:
                raise RuntimeError(f"Unexpected CLI {mode} launch output: {actual!r}")
            if launcher is not None:
                actual = run(*runner, application, argument, env=env)
                if actual != expected:
                    raise RuntimeError(f"Unexpected native {mode} launch output: {actual!r}")
            standalone_argument = "ASCII argument" if windows else argument
            actual = run(java, "-jar", application, standalone_argument)
            if actual != f"resource:{standalone_argument}":
                raise RuntimeError(f"Unexpected standalone launch output: {actual!r}")
    print(f"Verified binary format, dependencies, and launch modes: {target}")


if __name__ == "__main__":
    main()

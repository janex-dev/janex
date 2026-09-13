# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

"""Checks static musl binaries and exercises native and standalone Java launches."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(*command, **kwargs):
    """Runs a command and returns its UTF-8 output, failing on a nonzero exit."""
    return subprocess.run(
        command, check=True, text=True, encoding="utf-8", stdout=subprocess.PIPE, **kwargs
    ).stdout


def main():
    """Validates the two binaries for the requested Linux target."""
    target = sys.argv[1]
    machines = {
        "x86_64-unknown-linux-musl": "Advanced Micro Devices X86-64",
        "aarch64-unknown-linux-musl": "AArch64",
    }
    machine = machines[target]
    directory = Path(os.environ.get("CARGO_TARGET_DIR", "target")) / target / "release"
    cli = (directory / "janex").resolve()
    launcher = (directory / "janex-launcher").resolve()
    for binary in (cli, launcher):
        header = run("readelf", "-hW", binary)
        if machine not in header:
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

    runner = ["qemu-aarch64"] if target.startswith("aarch64-") else []
    java_home = Path(os.environ["JAVA_HOME"])
    java = java_home / "bin/java"
    javac = java_home / "bin/javac"
    env = dict(os.environ, JANEX_JAVA=str(java))
    argument = "Unicode 参数 🚀"
    expected = f"resource:{argument}"
    with tempfile.TemporaryDirectory(prefix="janex-musl-") as temporary:
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
            System.out.print(new String(bytes, StandardCharsets.UTF_8) + args[0]);
        }
    }
}
''', encoding="utf-8")
        run(javac, "--release", "8", "-d", classes, source)
        (classes / "message.txt").write_text("resource:", encoding="utf-8")
        run(*runner, cli, "--version")
        for mode in ("bootstrap", "direct"):
            application = root / f"application-{mode}"
            run(*runner, cli, "pack", classes, "--output", application,
                "--main-class", "Main", "--native-launcher", launcher,
                "--native-launch-mode", mode, "--with-launcher")
            actual = run(*runner, application, argument, env=env)
            if actual != expected:
                raise RuntimeError(f"Unexpected {mode} launch output: {actual!r}")
            actual = run(java, "-jar", application, argument)
            if actual != expected:
                raise RuntimeError(f"Unexpected standalone launch output: {actual!r}")
    print(f"Verified static musl, mimalloc, and launch modes: {target}")


if __name__ == "__main__":
    main()

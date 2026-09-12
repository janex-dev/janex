#!/usr/bin/env python3
# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

"""Rebuild the embedded Java 8 bootstrap with JDK 25, or check its recorded bytes."""

import argparse
import pathlib
import subprocess
import tempfile


def main():
    """Compile in a temporary directory and compare or replace the embedded class file."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix="janex-bootstrap-") as directory:
        subprocess.run(["javac", "--release", "8", "-g:none", "-Xlint:-options",
                        "-d", directory, str(root / "Bootstrap.java")], check=True)
        compiled = (pathlib.Path(directory) / "org/janex/bootstrap/Bootstrap.class").read_bytes()
        target = root / "Bootstrap.class"
        if args.check:
            if compiled != target.read_bytes():
                raise SystemExit("Bootstrap.class differs from its Java source; run bootstrap/build.py")
            print("Verified embedded bootstrap class")
        else:
            target.write_bytes(compiled)


if __name__ == "__main__":
    main()

# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

"""Packages target binaries and writes an archive SHA-256 checksum."""

import hashlib
import os
from pathlib import Path
import sys
import tarfile
import zipfile


def main():
    """Creates a ZIP on Windows or a tar.gz preserving Unix executable permissions."""
    target = sys.argv[1]
    windows = target.endswith("-windows-msvc")
    directory = Path(os.environ.get("CARGO_TARGET_DIR", "target")) / target / "release"
    names = ["janex"] if target.endswith("-apple-darwin") else ["janex", "janex-launcher"]
    if windows:
        names = [name + ".exe" for name in names]
    destination = Path("build/distributions")
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / f"janex-{target}.{'zip' if windows else 'tar.gz'}"
    if windows:
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
            for name in names:
                output.write(directory / name, name)
    else:
        with tarfile.open(archive, "w:gz") as output:
            for name in names:
                output.add(directory / name, arcname=name)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(
        f"{digest}  {archive.name}\n", encoding="ascii"
    )
    print(archive)


if __name__ == "__main__":
    main()

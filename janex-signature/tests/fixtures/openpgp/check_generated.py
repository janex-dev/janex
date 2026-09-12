#!/usr/bin/env python3
"""Verify Janex-generated OpenPGP package signatures with an isolated GnuPG keyring."""

import argparse
import os
import pathlib
import shutil
import struct
import subprocess
import tempfile

from generate import run


def check_password_prompt(command):
    """Exercise a real controlling terminal and verify that the password is never echoed."""
    import errno
    import fcntl
    import pty
    import select
    import termios
    import time

    master, slave = pty.openpty()

    def controlling_terminal():
        """Give the child its own controlling terminal for the hidden password prompt."""
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    try:
        process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                                   preexec_fn=controlling_terminal)
    finally:
        os.close(slave)
    output = bytearray()
    sent = False
    deadline = time.monotonic() + 30
    try:
        while time.monotonic() < deadline:
            if not sent and b"Private key password: " in output and not termios.tcgetattr(master)[3] & termios.ECHO:
                os.write(master, b"public-fixture-password\n")
                sent = True
            if select.select([master], [], [], 0.05)[0]:
                try:
                    chunk = os.read(master, 4096)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break
                if not chunk:
                    break
                output.extend(chunk)
        assert sent, "password prompt did not disable terminal echo"
        assert process.wait(timeout=5) == 0, "terminal password loading failed"
        assert b"public-fixture-password" not in output, "password was echoed"
    finally:
        os.close(master)
        if process.poll() is None:
            process.kill()
            process.wait()
    print("Verified hidden terminal password input", flush=True)


def vuint(data, offset):
    """Decode a generated package's ULEB128 length without using the Janex reader."""
    value = 0
    for shift in range(0, 70, 7):
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, offset
    raise ValueError("invalid ULEB128")


def main():
    """Pack supported signing combinations, extract their exact input, and verify independently."""
    root = pathlib.Path(__file__).resolve().parent
    project = root.parents[3]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--janex", default=str(project / "target/debug" / ("janex.exe" if os.name == "nt" else "janex")))
    fallback = pathlib.Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/usr/bin/gpg.exe"
    parser.add_argument("--gpg", default=shutil.which("gpg") or str(fallback))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="jpgp-") as scratch:
        scratch = pathlib.Path(scratch)
        home = scratch / "keys"
        home.mkdir(mode=0o700)
        source = scratch / "source"
        source.mkdir()
        password = scratch / "password.txt"
        password.write_bytes(b"public-fixture-password\n")
        command = [args.gpg, "--homedir", str(home), "--batch", "--no-tty", "--no-auto-key-retrieve"]
        try:
            if os.name == "posix":
                check_password_prompt([args.janex, "pack", str(source), "--output", str(scratch / "terminal.janex"),
                                       "--main-class", "Main", "--openpgp-key", str(root / "encrypted.secret.pgp")])
            for name, algorithm in [("rsa256", "rsa-sha256"), ("rsa512", "rsa-sha512"),
                                    ("p256", "ecdsa-p256-sha256"), ("p384", "ecdsa-p384-sha384"),
                                    ("ed25519", "ed25519-sha256"), ("ed25519", "ed25519-sha512"),
                                    ("encrypted", "ed25519-sha256")]:
                package = scratch / f"{name}-{algorithm}.janex"
                run([args.janex, "pack", str(source), "--output", str(package), "--main-class", "Main",
                     "--openpgp-key", str(root / f"{name}.secret.pgp"), "--openpgp-algorithm", algorithm,
                     "--key-password-file", str(password)])
                data = package.read_bytes()
                assert data[-24:-16] == b"JANEXEND"
                start = len(data) - struct.unpack_from("<Q", data, len(data) - 16)[0]
                size, offset = vuint(data, start + 16)
                offset += size
                assert data[offset] == 2
                document = data[start:offset + 1]
                size, offset = vuint(data, offset + 1)
                assert offset + size == len(data) - 24
                signature = scratch / "signature.pgp"
                content = scratch / "input.bin"
                signature.write_bytes(data[offset:offset + size])
                content.write_bytes(document)
                run(command + ["--import", str(root / f"{name}.public.pgp")])
                run(command + ["--verify", str(signature), str(content)])
                content.write_bytes(document[:-1] + bytes([document[-1] ^ 1]))
                try:
                    run(command + ["--verify", str(signature), str(content)])
                except RuntimeError:
                    pass
                else:
                    raise AssertionError("GnuPG accepted changed signed metadata")
                print(f"GnuPG verified Janex {name} {algorithm}", flush=True)
        finally:
            gpg_path = pathlib.Path(shutil.which(args.gpg) or args.gpg)
            gpgconf = gpg_path.with_name("gpgconf.exe" if gpg_path.suffix == ".exe" else "gpgconf")
            run([str(gpgconf), "--homedir", str(home), "--kill", "all"])


if __name__ == "__main__":
    main()

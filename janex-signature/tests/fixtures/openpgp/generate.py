#!/usr/bin/env python3
"""Generate public OpenPGP test keys and signatures independently of Janex and rPGP."""

import argparse
import hashlib
import os
import pathlib
import shutil
import subprocess
import tempfile


ROOT = pathlib.Path(__file__).resolve().parent
CREATED = 1767225600  # 2026-01-01T00:00:00Z


def run(command, **options):
    """Run a fixture command without exposing key material in diagnostics."""
    executable = pathlib.Path(command[0])
    if os.name == "nt" and executable.parent.as_posix().endswith("/usr/bin") and executable.stem.startswith("gpg"):
        # Git for Windows ships MSYS GnuPG, which expects POSIX paths even when called from Python.
        command = [command[0]] + ["/" + value[0].lower() + value[2:].replace("\\", "/")
                                  if len(value) > 2 and value[1] == ":" else value for value in command[1:]]
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **options)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace"))
    return result.stdout


def packet(tag, body):
    """Encode one new-format packet with a definite five-octet length."""
    return bytes([0xC0 | tag, 255]) + len(body).to_bytes(4, "big") + body


def version_six(openssl):
    """Construct v6 fields in Python and sign the digest using OpenSSL Ed25519."""
    with tempfile.TemporaryDirectory(prefix="janex-pgp-v6-") as directory:
        work = pathlib.Path(directory)
        key = work / "key.pem"
        run([openssl, "genpkey", "-algorithm", "ED25519", "-out", str(key)])
        public_der = run([openssl, "pkey", "-in", str(key), "-pubout", "-outform", "DER"])
        secret_der = run([openssl, "pkey", "-in", str(key), "-outform", "DER"])
        assert public_der[:12] == bytes.fromhex("302a300506032b6570032100")
        assert secret_der[:16] == bytes.fromhex("302e020100300506032b657004220420")
        public = b"\x06" + CREATED.to_bytes(4, "big") + b"\x1b\x00\x00\x00\x20" + public_der[12:]
        fingerprint = hashlib.sha256(b"\x9b" + len(public).to_bytes(4, "big") + public).digest()
        (ROOT / "v6.public.pgp").write_bytes(packet(6, public))
        (ROOT / "v6.secret.pgp").write_bytes(packet(5, public + b"\x00" + secret_der[16:]))
        (ROOT / "v6.public.pem").write_bytes(run([openssl, "pkey", "-in", str(key), "-pubout"]))
        # The two scalar lengths are four octets in a v6 signature.
        hashed = b"\x05\x82" + CREATED.to_bytes(4, "big") + b"\x22\x21\x06" + fingerprint
        protected = b"\x06\x00\x1b\x0a" + len(hashed).to_bytes(4, "big") + hashed
        salt = bytes(range(32))
        digest = hashlib.sha512(salt + (ROOT / "input.bin").read_bytes() + protected +
                                b"\x06\xff" + len(protected).to_bytes(4, "big")).digest()
        (work / "digest.bin").write_bytes(digest)
        signature = run([openssl, "pkeyutl", "-sign", "-rawin", "-inkey", str(key),
                         "-in", str(work / "digest.bin")])
        body = protected + bytes(4) + digest[:2] + b"\x20" + salt + signature
        (ROOT / "v6.signature.pgp").write_bytes(packet(2, body))


def certifications(command, home):
    """Generate independent signing-subkey, expiry, and primary-revocation evidence."""
    for name, usage, expiry in [("subkey", "cert", "0"), ("expired", "sign", "1d")]:
        uid = f"Janex public policy test {name} <{name}@example.invalid>"
        run(command + ["--quick-generate-key", uid, "ed25519", usage, expiry])
        fingerprint = next(line.split(":")[9] for line in
                           run(command + ["--with-colons", "--list-keys", uid]).decode().splitlines()
                           if line.startswith("fpr:"))
        if name == "subkey":
            run(command + ["--quick-add-key", fingerprint, "ed25519", "sign", "1d"])
        for suffix, option in [("public", "--export"), ("secret", "--export-secret-keys")]:
            (ROOT / f"{name}.{suffix}.pgp").write_bytes(run(command + [option, fingerprint]))
        signature = ROOT / f"{name}.signature.pgp"
        run(command + ["--local-user", fingerprint, "--digest-algo", "SHA256", "--output",
                       str(signature), "--detach-sign", str(ROOT / "input.bin")])
        run(command + ["--verify", str(signature), str(ROOT / "input.bin")])
        if name == "subkey":
            revocation = (home / "openpgp-revocs.d" / f"{fingerprint}.rev").read_bytes()
            revocation = revocation[revocation.index(b":-----BEGIN PGP PUBLIC KEY BLOCK-----") + 1:]
            run(command + ["--import"], input=revocation)
            (ROOT / "subkey.revoked.public.pgp").write_bytes(run(command + ["--export", fingerprint]))
        print(f"Generated policy fixture {name}", flush=True)


def main():
    """Generate fixtures in an isolated keyring and terminate its agent before cleanup."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gpg", default="gpg")
    parser.add_argument("--openssl", default="openssl")
    parser.add_argument("--encrypted-only", action="store_true",
                        help="Generate only the encrypted signing-subkey fixtures")
    args = parser.parse_args()
    if args.encrypted_only:
        encrypted_key(args.gpg)
        return
    (ROOT / "input.bin").write_bytes(b"Janex OpenPGP interoperability fixture\x00\n\r\n")
    # GnuPG creates sockets beneath this path on Unix; keep the temporary name short.
    with tempfile.TemporaryDirectory(prefix="jpgp-") as directory:
        home = pathlib.Path(directory)
        home.chmod(0o700)
        command = [args.gpg, "--homedir", str(home), "--batch", "--yes", "--no-tty",
                   "--pinentry-mode", "loopback", "--passphrase", "", "--faked-system-time", f"{CREATED}!"]
        try:
            for name, algorithm, digest in [("rsa256", "rsa2048", "SHA256"),
                                             ("rsa512", "rsa2048", "SHA512"),
                                             ("p256", "nistp256", "SHA256"),
                                             ("p384", "nistp384", "SHA384"),
                                             ("ed25519", "ed25519", "SHA256")]:
                uid = f"Janex public test key {name} <{name}@example.invalid>"
                run(command + ["--quick-generate-key", uid, algorithm, "sign", "0"])
                fingerprint = next(line.split(":")[9] for line in
                                   run(command + ["--with-colons", "--list-keys", uid]).decode().splitlines()
                                   if line.startswith("fpr:"))
                for suffix, option in [("public", "--export"), ("secret", "--export-secret-keys")]:
                    (ROOT / f"{name}.{suffix}.pgp").write_bytes(run(command + [option, fingerprint]))
                signature = ROOT / f"{name}.signature.pgp"
                run(command + ["--local-user", fingerprint, "--digest-algo", digest,
                               "--output", str(signature), "--detach-sign", str(ROOT / "input.bin")])
                run(command + ["--verify", str(signature), str(ROOT / "input.bin")])
                if name == "ed25519":
                    (ROOT / "ed25519.public.asc").write_bytes(run(command + ["--armor", "--export", fingerprint]))
                    revocation = (home / "openpgp-revocs.d" / f"{fingerprint}.rev").read_bytes()
                    revocation = revocation[revocation.index(b":-----BEGIN PGP PUBLIC KEY BLOCK-----") + 1:]
                    run(command + ["--import"], input=revocation)
                    (ROOT / "ed25519.revoked.public.pgp").write_bytes(run(command + ["--export", fingerprint]))
                print(f"Generated and verified {name}", flush=True)
            certifications(command, home)
        finally:
            gpg_path = pathlib.Path(shutil.which(args.gpg) or args.gpg)
            gpgconf = gpg_path.with_name("gpgconf.exe" if gpg_path.suffix == ".exe" else "gpgconf")
            run([str(gpgconf), "--homedir", str(home), "--kill", "all"])
    version_six(args.openssl)
    encrypted_key(args.gpg)
    print("Generated v6 Ed25519/SHA-512 with OpenSSL", flush=True)


def encrypted_key(gpg):
    """Export independently protected signing-subkey material using a public test password."""
    with tempfile.TemporaryDirectory(prefix="jpgp-") as directory:
        home = pathlib.Path(directory)
        home.chmod(0o700)
        password = home / "password.txt"
        password.write_bytes(b"public-fixture-password\n")
        command = [gpg, "--homedir", str(home), "--batch", "--yes", "--no-tty",
                   "--pinentry-mode", "loopback", "--passphrase-file", str(password),
                   "--faked-system-time", f"{CREATED}!"]
        try:
            uid = "Janex public encrypted test key <encrypted@example.invalid>"
            run(command + ["--quick-generate-key", uid, "ed25519", "cert", "0"])
            fingerprint = next(line.split(":")[9] for line in
                               run(command + ["--with-colons", "--list-keys", uid]).decode().splitlines()
                               if line.startswith("fpr:"))
            run(command + ["--quick-add-key", fingerprint, "ed25519", "sign", "0"])
            for suffix, option in [("public", "--export"), ("secret", "--export-secret-keys")]:
                (ROOT / f"encrypted.{suffix}.pgp").write_bytes(run(command + [option, fingerprint]))
            (ROOT / "encrypted.secret.asc").write_bytes(run(command + ["--armor", "--export-secret-keys", fingerprint]))
            signature = ROOT / "encrypted.signature.pgp"
            run(command + ["--local-user", fingerprint, "--digest-algo", "SHA256", "--output",
                           str(signature), "--detach-sign", str(ROOT / "input.bin")])
            run(command + ["--verify", str(signature), str(ROOT / "input.bin")])
            print("Generated and verified encrypted signing-subkey fixtures", flush=True)
        finally:
            gpg_path = pathlib.Path(shutil.which(gpg) or gpg)
            gpgconf = gpg_path.with_name("gpgconf.exe" if gpg_path.suffix == ".exe" else "gpgconf")
            run([str(gpgconf), "--homedir", str(home), "--kill", "all"])


if __name__ == "__main__":
    main()

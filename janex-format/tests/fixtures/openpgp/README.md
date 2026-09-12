# OpenPGP Interoperability Fixtures

All secret keys in this directory are **public test material**. Never use them to sign releases.

`generate.py` uses an isolated GnuPG keyring to generate version-4 RSA, P-256, P-384, and Ed25519
keys and detached binary signatures. SHA-256 and SHA-512 RSA signatures use separate keys. The
fixed creation instant is 2026-01-01T00:00:00Z, and the keys have no expiration. GnuPG verifies
each resulting signature before its temporary keyring is removed.

`subkey` has a certification-only primary key and a signing subkey that expires after one day.
`subkey.revoked.public.pgp` additionally contains GnuPG's primary-key revocation certificate.
`expired` has a signing primary key that expires after one day. These fixtures exercise trust
policy separately from document signature validity.
`ed25519.public.asc` is the armored primary-key certificate; `ed25519.revoked.public.pgp`
adds its GnuPG-generated revocation for command-line authentication tests.

`encrypted` has a certification-only primary key and a signing subkey, both protected by GnuPG
with the public test password `public-fixture-password`. Binary and armored secret-key exports
exercise password-file loading and subkey selection. These keys do not expire. Use
`generate.py --encrypted-only` to regenerate only these fixtures.

The version-6 Ed25519 fixture uses independent Python packet encoding and OpenSSL signing:
SHA-512 covers the salt, exact document bytes, original protected fields, and RFC 9580 trailer;
OpenSSL signs that digest with Ed25519. The v6 key files contain a single bare key packet for
mechanism tests; they do not contain the certifications needed for publisher authentication.

Regenerate with GnuPG and OpenSSL available on `PATH`:

```text
python janex-format/tests/fixtures/openpgp/generate.py
```

Use `--gpg` and `--openssl` to select executables explicitly. Regeneration replaces random keys
and signatures. `input.bin` contains a NUL and mixed newline bytes to detect text normalization.

After building the CLI, run `check_generated.py` to verify Janex-generated signatures with GnuPG
for every supported version-4 algorithm combination and the encrypted signing subkey. It extracts
the protected metadata independently and also checks that changed metadata is rejected.
On Unix it additionally supplies the public test password through a controlling terminal, checking
that the prompt disables echo and that the password does not appear in terminal output.

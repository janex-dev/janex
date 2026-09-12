#!/usr/bin/env python3
"""Generate public test keys and independently encoded CMS fixtures using OpenSSL."""

import argparse
import hashlib
import pathlib
import subprocess
import tempfile


def tlv(tag, value):
    """Encode a definite-length DER value."""
    size = len(value)
    length = bytes([size]) if size < 128 else size.to_bytes((size.bit_length() + 7) // 8, "big")
    if size >= 128:
        length = bytes([0x80 | len(length)]) + length
    return bytes([tag]) + length + value


def children(encoded):
    """Split the content of one constructed DER value into complete child encodings."""
    def bounds(data, start):
        offset = start + 1
        size = data[offset]
        offset += 1
        if size & 0x80:
            length_bytes = size & 0x7F
            size = int.from_bytes(data[offset:offset + length_bytes], "big")
            offset += length_bytes
        return offset, offset + size

    start, end = bounds(encoded, 0)
    result = []
    while start < end:
        _, next_start = bounds(encoded, start)
        result.append(encoded[start:next_start])
        start = next_start
    assert start == end
    return result


def oid(text):
    """Encode an object identifier with base-128 arcs."""
    arcs = [int(arc) for arc in text.split(".")]
    content = bytearray()
    for arc in [40 * arcs[0] + arcs[1], *arcs[2:]]:
        parts = [arc & 127]
        while arc > 127:
            arc >>= 7
            parts.insert(0, (arc & 127) | 128)
        content.extend(parts)
    return tlv(6, content)


def sequence(*values):
    """Encode a DER sequence."""
    return tlv(0x30, b"".join(values))


def ordered_set(*values):
    """Encode a DER SET OF in lexicographic byte order."""
    return tlv(0x31, b"".join(sorted(values)))


def main():
    """Generate fixtures with no Rust implementation involved in their encoding or signing."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--openssl", default="openssl")
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent

    def openssl(*arguments):
        subprocess.run([args.openssl, *map(str, arguments)], cwd=root, check=True, capture_output=True)

    data = b"Janex CMS interoperability fixture\n"
    (root / "input.bin").write_bytes(data)
    (root / "password.txt").write_bytes(b"public-fixture-password\n")
    data_oid = oid("1.2.840.113549.1.7.1")
    for serial, (name, key_args, digest, digest_arc, sig_oid) in enumerate([
        ("rsa256", ["RSA", "-pkeyopt", "rsa_keygen_bits:2048"], "sha256", 1, "1.2.840.113549.1.1.1"),
        ("rsa512", ["RSA", "-pkeyopt", "rsa_keygen_bits:2048"], "sha512", 3, "1.2.840.113549.1.1.1"),
        ("p256", ["EC", "-pkeyopt", "ec_paramgen_curve:P-256"], "sha256", 1, "1.2.840.10045.4.3.2"),
        ("p384", ["EC", "-pkeyopt", "ec_paramgen_curve:P-384"], "sha384", 2, "1.2.840.10045.4.3.3"),
    ], start=1):
        key = root / f"{name}.key.pem"
        cert = root / f"{name}.cert.pem"
        openssl("genpkey", "-algorithm", *key_args, "-out", key)
        openssl("req", "-new", "-x509", "-key", key, "-out", cert, "-days", "36500", "-set_serial", serial,
                "-subj", f"/CN=Janex {name} Test Fixture", "-sha256", "-addext", "basicConstraints=critical,CA:FALSE",
                "-addext", "keyUsage=critical,digitalSignature", "-addext", "extendedKeyUsage=codeSigning")
        with tempfile.TemporaryDirectory() as scratch:
            scratch = pathlib.Path(scratch)
            cert_der_path = scratch / "certificate.der"
            openssl("x509", "-in", cert, "-outform", "DER", "-out", cert_der_path)
            cert_der = cert_der_path.read_bytes()
            tbs = children(children(cert_der)[0])
            serial_der, issuer_der = tbs[1], tbs[3]
            digest_algorithm = sequence(oid(f"2.16.840.1.101.3.4.2.{digest_arc}"))
            signature_algorithm = sequence(oid(sig_oid), b"\x05\x00" if name.startswith("rsa") else b"")
            protection = sequence(digest_algorithm, tlv(0xA1, b"".join(children(signature_algorithm))))
            attributes = ordered_set(
                sequence(oid("1.2.840.113549.1.9.3"), ordered_set(data_oid)),
                sequence(oid("1.2.840.113549.1.9.4"), ordered_set(tlv(4, hashlib.new(digest, data).digest()))),
                sequence(oid("1.2.840.113549.1.9.52"), ordered_set(protection)),
            )
            attrs_path = scratch / "attributes.der"
            attrs_path.write_bytes(attributes)
            sig_path = scratch / "signature.bin"
            openssl("dgst", f"-{digest}", "-sign", key, "-out", sig_path, attrs_path)
            signer_info = sequence(b"\x02\x01\x01", sequence(issuer_der, serial_der), digest_algorithm,
                                   tlv(0xA0, b"".join(children(attributes))), signature_algorithm, tlv(4, sig_path.read_bytes()))
            signed_data = sequence(b"\x02\x01\x01", ordered_set(digest_algorithm), sequence(data_oid),
                                   tlv(0xA0, cert_der), ordered_set(signer_info))
            envelope = sequence(oid("1.2.840.113549.1.7.2"), tlv(0xA0, signed_data))
            fixture = root / f"{name}.cms.der"
            fixture.write_bytes(envelope)
            openssl("cms", "-verify", "-binary", "-inform", "DER", "-in", fixture, "-content", root / "input.bin",
                    "-noverify", "-out", scratch / "verified.bin")
            assert (scratch / "verified.bin").read_bytes() == data
        openssl("pkcs8", "-topk8", "-in", key, "-v2", "aes-256-cbc", "-v2prf", "hmacWithSHA256",
                "-passout", "file:password.txt", "-out", root / f"{name}.encrypted.pem")
    openssl("cms", "-sign", "-binary", "-in", "input.bin", "-signer", "rsa256.cert.pem", "-inkey", "rsa256.key.pem",
            "-outform", "DER", "-md", "sha256", "-out", "missing-protection.cms.der")
    openssl("genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", "ca.key.pem")
    openssl("req", "-new", "-x509", "-key", "ca.key.pem", "-out", "ca.cert.pem", "-days", "36500", "-set_serial", "100",
            "-subj", "/CN=Janex CRL Test Issuer", "-sha256", "-addext", "basicConstraints=critical,CA:TRUE",
            "-addext", "keyUsage=critical,keyCertSign,cRLSign")
    with tempfile.TemporaryDirectory() as scratch:
        scratch = pathlib.Path(scratch)
        (scratch / "index").write_bytes(b"")
        (scratch / "serial").write_bytes(b"0100\n")
        (scratch / "crlnumber").write_bytes(b"01\n")
        config = scratch / "ca.cnf"
        config.write_text(f"""[ca]
default_ca = issuer
[issuer]
dir = "{scratch.as_posix()}"
database = $dir/index
new_certs_dir = $dir
serial = $dir/serial
crlnumber = $dir/crlnumber
certificate = "{root.as_posix()}/ca.cert.pem"
private_key = "{root.as_posix()}/ca.key.pem"
default_md = sha256
default_days = 36500
default_crl_days = 36500
policy = policy
x509_extensions = leaf
[policy]
commonName = supplied
[leaf]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature
extendedKeyUsage = codeSigning
""", encoding="utf-8")
        request = scratch / "issued.csr"
        openssl("req", "-new", "-key", "rsa256.key.pem", "-subj", "/CN=Janex Issued Test Signer", "-out", request)
        openssl("ca", "-batch", "-config", config, "-in", request, "-out", "issued.cert.pem", "-notext")
        openssl("ca", "-gencrl", "-config", config, "-out", "good.crl.pem")
        openssl("ca", "-gencrl", "-config", config, "-crldays", "1", "-out", "stale.crl.pem")
        openssl("ca", "-config", config, "-revoke", "issued.cert.pem", "-crl_reason", "keyCompromise")
        openssl("ca", "-gencrl", "-config", config, "-out", "revoked.crl.pem")
    openssl("req", "-new", "-x509", "-key", "rsa256.key.pem", "-out", "wrong-usage.cert.pem", "-days", "36500",
            "-subj", "/CN=Janex Wrong Usage", "-sha256", "-addext", "keyUsage=critical,keyEncipherment")


if __name__ == "__main__":
    main()

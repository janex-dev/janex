// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.io.StringReader;
import java.nio.charset.StandardCharsets;
import java.security.PublicKey;
import java.security.interfaces.RSAKey;

import org.bouncycastle.asn1.ASN1ObjectIdentifier;
import org.bouncycastle.asn1.x509.SubjectPublicKeyInfo;
import org.bouncycastle.jce.provider.BouncyCastleProvider;
import org.bouncycastle.openssl.PEMParser;

import static org.glavo.janex.reader.internal.Input.require;

/// Shares bounded key parsing and algorithm selection without registering a global provider.
final class SigningSupport {
    /// Private provider instance, never added to the JVM-wide provider list.
    static final BouncyCastleProvider PROVIDER = new BouncyCastleProvider();
    /// Maximum encoded certificate or secret-key input.
    static final int KEY_LIMIT = 8 * 1024 * 1024;

    /// Prevents instantiation.
    private SigningSupport() { }

    /// Parses exactly one PEM object, or returns null for DER input.
    static Object pem(byte[] bytes) throws IOException {
        String text = new String(bytes, StandardCharsets.US_ASCII);
        if (!text.stripLeading().startsWith("-----BEGIN ")) return null;
        try (PEMParser parser = new PEMParser(new StringReader(text))) {
            Object object = parser.readObject();
            require(object != null && parser.readObject() == null, "Expected one PEM object");
            return object;
        }
    }

    /// Selects or checks a supported digest and signature combination against the public key.
    static SigningAlgorithm algorithm(PublicKey key, SigningAlgorithm selected, boolean openPgp) throws IOException {
        try {
            return selectAlgorithm(key, selected, openPgp);
        } catch (IllegalArgumentException failure) {
            throw new IOException("Unsupported signing key parameters", failure);
        }
    }

    /// Matches supported key parameters before choosing a digest.
    private static SigningAlgorithm selectAlgorithm(PublicKey key, SigningAlgorithm selected, boolean openPgp) throws IOException {
        SigningAlgorithm preferred;
        if (key instanceof RSAKey rsa) {
            int bits = rsa.getModulus().bitLength();
            require(bits >= 2048 && bits <= 8192, "RSA signing key must contain 2048 through 8192 bits");
            preferred = SigningAlgorithm.RSA_SHA256;
        } else {
            var identifier = SubjectPublicKeyInfo.getInstance(key.getEncoded()).getAlgorithm();
            String oid = identifier.getAlgorithm().getId();
            if (oid.equals("1.2.840.10045.2.1")) {
                String curve = ASN1ObjectIdentifier.getInstance(identifier.getParameters()).getId();
                if (curve.equals("1.2.840.10045.3.1.7")) preferred = SigningAlgorithm.ECDSA_P256_SHA256;
                else if (curve.equals("1.3.132.0.34")) preferred = SigningAlgorithm.ECDSA_P384_SHA384;
                else throw new IOException("Unsupported signing curve");
            } else if (openPgp && oid.equals("1.3.101.112")) preferred = SigningAlgorithm.ED25519_SHA256;
            else throw new IOException("Unsupported signing key");
        }
        if (selected == null) return preferred;
        require(selected == preferred || preferred == SigningAlgorithm.RSA_SHA256 && selected == SigningAlgorithm.RSA_SHA512
                || preferred == SigningAlgorithm.ED25519_SHA256 && selected == SigningAlgorithm.ED25519_SHA512,
                "Signature algorithm does not match the key");
        return selected;
    }
}

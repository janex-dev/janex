// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

/// Publisher signature combinations supported by the Java writer and Rust Host.
public enum SigningAlgorithm {
    /// RSA PKCS#1 v1.5 with SHA-256, using 2048 through 8192 bits.
    RSA_SHA256("SHA256withRSA", 8),
    /// RSA PKCS#1 v1.5 with SHA-512, using 2048 through 8192 bits.
    RSA_SHA512("SHA512withRSA", 10),
    /// NIST P-256 ECDSA with SHA-256.
    ECDSA_P256_SHA256("SHA256withECDSA", 8),
    /// NIST P-384 ECDSA with SHA-384.
    ECDSA_P384_SHA384("SHA384withECDSA", 9),
    /// OpenPGP Ed25519 with SHA-256; unavailable for CMS and version-6 signatures.
    ED25519_SHA256("Ed25519", 8),
    /// OpenPGP Ed25519 with SHA-512; unavailable for CMS.
    ED25519_SHA512("Ed25519", 10);

    /// JCA signature algorithm name.
    final String jca;
    /// OpenPGP hash algorithm identifier.
    final int hash;

    /// Associates the JCA and OpenPGP mechanism names.
    SigningAlgorithm(String jca, int hash) {
        this.jca = jca;
        this.hash = hash;
    }
}

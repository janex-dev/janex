// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.IOException;
import java.io.InputStream;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Arrays;
import java.util.Objects;

/// An immutable Janex algorithm identifier and its canonical digest bytes.
///
/// Successful verification establishes content integrity, not publisher identity.
public final class Checksum {
    /// Supported algorithms with the identifiers and byte representations defined by Janex 0.1.
    public enum Algorithm {
        /// Seed-zero XXH3-64 with the default secret and a big-endian digest; not cryptographically secure.
        XXH3_64(0x11, 8, false),
        /// Seed-zero XXH3-128 with the default secret and the high half first; not cryptographically secure.
        XXH3_128(0x12, 16, false),
        /// SHA-256 in its standard 32-byte representation.
        SHA256(0x21, 32, true),
        /// SHA-512 in its standard 64-byte representation.
        SHA512(0x22, 64, true),
        /// SM3 in its standard 32-byte representation.
        SM3(0x31, 32, true);

        /// On-disk algorithm identifier.
        private final int id;
        /// Required digest length in bytes.
        private final int length;
        /// Whether the algorithm is permitted for cryptographic content authentication.
        private final boolean secure;

        /// Retains one fixed algorithm definition.
        Algorithm(int id, int length, boolean secure) {
            this.id = id;
            this.length = length;
            this.secure = secure;
        }

        /// Returns the unsigned byte identifier stored in a checksum field.
        public int id() {
            return id;
        }

        /// Returns the required digest length, excluding the algorithm identifier.
        public int digestLength() {
            return length;
        }

        /// Returns whether this algorithm may authenticate section or external-region bytes.
        public boolean isSecure() {
            return secure;
        }

        /// Creates independent, resettable streaming state with bounded internal buffering.
        ///
        /// The state follows [MessageDigest]'s update and digest contracts and is not thread-safe.
        /// XXH3 always uses seed zero and the default secret. SM3 does not require a JCA provider.
        ///
        /// @return a new digest whose output uses this algorithm's canonical byte representation
        /// @throws IOException if a required platform SHA implementation is unavailable
        public MessageDigest newDigest() throws IOException {
            switch (this) {
                case XXH3_64:
                    return new Xxh3(false);
                case XXH3_128:
                    return new Xxh3(true);
                case SM3:
                    return new Sm3();
                default:
                    try {
                        return MessageDigest.getInstance(this == SHA256 ? "SHA-256" : "SHA-512");
                    } catch (NoSuchAlgorithmException failure) {
                        throw new IOException("Required digest is unavailable: " + this, failure);
                    }
            }
        }

        /// Decodes one supported algorithm identifier.
        ///
        /// @param id unsigned byte value from a checksum field
        /// @return the corresponding supported algorithm
        /// @throws IOException if the identifier is reserved, unknown, or outside the unsigned byte range
        public static Algorithm fromId(int id) throws IOException {
            for (Algorithm algorithm : values()) {
                if (algorithm.id == id) {
                    return algorithm;
                }
            }
            throw new IOException(id == 0 ? "Checksum algorithm zero is reserved" : "Unsupported checksum algorithm: " + id);
        }
    }

    /// Supported checksum algorithm.
    private final Algorithm algorithm;
    /// Owned digest in canonical byte order.
    private final byte[] digest;

    /// Retains an already validated, owned digest.
    private Checksum(Algorithm algorithm, byte[] digest) {
        this.algorithm = algorithm;
        this.digest = digest;
    }

    /// Decodes a complete binary or CBOR checksum field without retaining its array.
    ///
    /// @param bytes algorithm byte followed by its exact canonical digest
    /// @return an immutable checksum
    /// @throws IOException if the field is empty, its algorithm is unsupported, or its length is incorrect
    /// @throws NullPointerException if bytes is null
    public static Checksum decode(byte[] bytes) throws IOException {
        Objects.requireNonNull(bytes);
        Input.require(bytes.length != 0, "Empty checksum");
        Algorithm algorithm = Algorithm.fromId(bytes[0] & 255);
        Input.require(bytes.length == algorithm.length + 1, "Incorrect checksum length");
        return new Checksum(algorithm, Arrays.copyOfRange(bytes, 1, bytes.length));
    }

    /// Returns the checksum algorithm.
    public Algorithm algorithm() {
        return algorithm;
    }

    /// Returns a new array containing the canonical digest without its algorithm byte.
    public byte[] digest() {
        return digest.clone();
    }

    /// Returns a new array containing the algorithm byte followed by the canonical digest.
    public byte[] encode() {
        byte[] encoded = new byte[digest.length + 1];
        encoded[0] = (byte) algorithm.id;
        System.arraycopy(digest, 0, encoded, 1, digest.length);
        return encoded;
    }

    /// Computes a checksum over an unchanged byte array without retaining it.
    ///
    /// @param algorithm supported checksum algorithm
    /// @param bytes complete input, not modified
    /// @return the computed checksum
    /// @throws IOException if the required platform digest is unavailable
    /// @throws NullPointerException if either argument is null
    public static Checksum compute(Algorithm algorithm, byte[] bytes) throws IOException {
        return new Checksum(algorithm, algorithm.newDigest().digest(Objects.requireNonNull(bytes)));
    }

    /// Computes a checksum over all remaining bytes, consuming the stream through EOF without closing it.
    ///
    /// An I/O failure may consume a prefix; no partial checksum is returned. The input is read with
    /// a fixed-size buffer, independently of the reader's buffered-value limits.
    ///
    /// @param algorithm supported checksum algorithm
    /// @param input stream owned by the caller
    /// @return the computed checksum
    /// @throws IOException if reading fails or the required platform digest is unavailable
    /// @throws NullPointerException if either argument is null
    public static Checksum compute(Algorithm algorithm, InputStream input) throws IOException {
        Objects.requireNonNull(input);
        MessageDigest digest = algorithm.newDigest();
        byte[] buffer = new byte[32768];
        for (int count; (count = input.read(buffer)) != -1;) {
            if (count == 0) {
                int value = input.read();
                if (value == -1) {
                    break;
                }
                digest.update((byte) value);
            } else {
                digest.update(buffer, 0, count);
            }
        }
        return new Checksum(algorithm, digest.digest());
    }

    /// Verifies an unchanged byte array against this checksum.
    ///
    /// @param bytes complete input, not modified
    /// @throws IOException if the checksum differs or its required platform digest is unavailable
    /// @throws NullPointerException if bytes is null
    public void verify(byte[] bytes) throws IOException {
        verifyDigest(compute(algorithm, bytes).digest);
    }

    /// Verifies all remaining stream bytes without closing the stream.
    ///
    /// Consumption and failure behavior are as specified by [#compute(Algorithm, InputStream)].
    ///
    /// @param input stream owned by the caller
    /// @throws IOException if reading fails, the checksum differs, or its platform digest is unavailable
    /// @throws NullPointerException if input is null
    public void verify(InputStream input) throws IOException {
        verifyDigest(compute(algorithm, input).digest);
    }

    /// Checks canonical digest bytes already calculated by a streaming caller.
    void verifyDigest(byte[] actual) throws IOException {
        Input.require(MessageDigest.isEqual(digest, actual), "Janex checksum mismatch");
    }
}

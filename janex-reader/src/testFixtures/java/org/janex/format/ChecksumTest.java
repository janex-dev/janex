// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.*;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.security.MessageDigest;
import java.util.Arrays;

/// Checks canonical checksum bytes, stream partitioning, reset behavior, and Rust-generated vectors.
public final class ChecksumTest {
    /// Prevents instantiation.
    private ChecksumTest() {
    }

    /// Runs fixed answers and optionally a complete stream of independently generated vectors.
    ///
    /// @param arguments optional path to a Rust-generated vector stream
    /// @throws Exception if an algorithm, encoded field, or streaming contract differs
    public static void main(String[] arguments) throws Exception {
        known(Checksum.Algorithm.SM3, "abc".getBytes(StandardCharsets.US_ASCII),
                "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0");
        byte[] block = new byte[64];
        for (int index = 0; index < block.length; index++) {
            block[index] = (byte) ('a' + index % 4);
        }
        known(Checksum.Algorithm.SM3, block, "debe9ff92275b8a138604889c18e5a4d6fdb70e5387e5765293dcba39c0c5732");
        known(Checksum.Algorithm.XXH3_64, new byte[0], "2d06800538d394c2");
        known(Checksum.Algorithm.XXH3_128, new byte[0], "99aa06d3014798d86001c324468d497f");
        byte[] input = new byte[65537];
        for (int index = 0; index < input.length; index++) {
            input[index] = (byte) (index % 251);
        }
        known(Checksum.Algorithm.XXH3_64, input, "70331d53d92bbc56");
        known(Checksum.Algorithm.XXH3_128, input, "0924e7a3a30e818770331d53d92bbc56");
        for (Checksum.Algorithm algorithm : Checksum.Algorithm.values()) {
            byte[] checksum = Checksum.compute(algorithm, block).encode();
            reject(() -> Checksum.decode(Arrays.copyOf(checksum, checksum.length - 1)));
            reject(() -> Checksum.decode(Arrays.copyOf(checksum, checksum.length + 1)));
            Checksum decoded = Checksum.decode(checksum);
            Arrays.fill(checksum, (byte) 0);
            decoded.verify(block);
            byte[] digest = decoded.digest();
            digest[0] ^= 1;
            decoded.verify(block);
            reject(() -> decoded.verify(new byte[0]));
            check(decoded.algorithm() == algorithm, "Algorithm identity");
            check(algorithm.newDigest().getDigestLength() == algorithm.digestLength(), "Digest length");
        }
        reject(() -> Checksum.decode(new byte[0]));
        reject(() -> Checksum.decode(new byte[]{0}));
        reject(() -> Checksum.decode(new byte[]{0x7f}));
        reject(() -> Checksum.Algorithm.fromId(256));
        if (arguments.length != 0) {
            try (DataInputStream vectors = new DataInputStream(Files.newInputStream(Paths.get(arguments[0])))) {
                int count = vectors.readInt();
                for (int vector = 0; vector < count; vector++) {
                    byte[] bytes = new byte[vectors.readInt()];
                    vectors.readFully(bytes);
                    for (int algorithm = 0; algorithm < Checksum.Algorithm.values().length; algorithm++) {
                        byte[] encoded = new byte[vectors.readUnsignedByte()];
                        vectors.readFully(encoded);
                        Checksum expected = Checksum.decode(encoded);
                        try {
                            exercise(expected, bytes);
                        } catch (Throwable failure) {
                            throw new AssertionError("Vector " + vector + ", length " + bytes.length + ", " + expected.algorithm(), failure);
                        }
                    }
                }
                check(vectors.read() == -1, "Trailing checksum vectors");
            }
        }
    }

    /// Checks one published canonical answer through the same streaming and lifecycle paths.
    private static void known(Checksum.Algorithm algorithm, byte[] bytes, String hexadecimal) throws Exception {
        byte[] encoded = new byte[algorithm.digestLength() + 1];
        encoded[0] = (byte) algorithm.id();
        for (int index = 1; index < encoded.length; index++) {
            encoded[index] = (byte) Integer.parseInt(hexadecimal.substring(index * 2 - 2, index * 2), 16);
        }
        exercise(Checksum.decode(encoded), bytes);
    }

    /// Compares whole-array, single-byte, fragmented, direct-buffer, and reset paths.
    private static void exercise(Checksum expected, byte[] bytes) throws Exception {
        expected.verify(bytes);
        expected.verify(new ByteArrayInputStream(bytes));
        MessageDigest digest = expected.algorithm().newDigest();
        for (int chunk : new int[]{1, 7, 63, 64, 65, 240, 241, 1023, 1024, 1025, 32768}) {
            for (int offset = 0; offset < bytes.length;) {
                int size = Math.min(chunk, bytes.length - offset);
                if (chunk == 1) {
                    digest.update(bytes[offset]);
                } else {
                    digest.update(bytes, offset, size);
                }
                digest.update(bytes, offset, 0);
                offset += size;
            }
            check(Arrays.equals(digest.digest(), expected.digest()), "Stream partition " + chunk);
        }
        ByteBuffer direct = ByteBuffer.allocateDirect(bytes.length + 9);
        direct.position(4);
        direct.put(bytes);
        direct.limit(direct.position());
        direct.position(4);
        digest.update(direct);
        check(!direct.hasRemaining(), "Digest did not consume ByteBuffer");
        check(Arrays.equals(digest.digest(), expected.digest()), "Direct ByteBuffer digest");
        digest.update((byte) 1);
        digest.reset();
        check(Arrays.equals(digest.digest(bytes), expected.digest()), "Reset digest");
    }

    /// Fails when a required checksum invariant does not hold.
    private static void check(boolean valid, String message) {
        if (!valid) {
            throw new AssertionError(message);
        }
    }

    /// Requires an invalid encoded checksum or incorrect payload to fail.
    private static void reject(Action action) throws Exception {
        try {
            action.run();
        } catch (IOException expected) {
            return;
        }
        throw new AssertionError("Invalid checksum accepted");
    }

    /// One potentially failing checksum operation.
    private interface Action {
        /// Executes the operation under test.
        void run() throws Exception;
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.*;
import java.util.Arrays;
import org.janex.bootstrap.internal.zstd.ZstdFrameDecompressor;

/// Compares the portable decoders with independent Rust-generated fixtures.
public final class CodecTest {
    /// Prevents instantiation.
    private CodecTest() {}
    /// Reads a counted fixture byte array.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] bytes = new byte[input.readInt()]; input.readFully(bytes); return bytes;
    }
    /// Checks all supplied valid and invalid codec vectors.
    /// @param args path to the independently generated fixture stream
    /// @throws Exception if a vector differs or a malformed input is accepted
    public static void main(String[] args) throws Exception {
        try (DataInputStream input = new DataInputStream(new FileInputStream(args[0]))) {
            int count = input.readInt();
            for (int i = 0; i < count; i++) {
                int kind = input.readUnsignedByte();
                boolean valid = input.readBoolean();
                byte[] encoded = bytes(input);
                byte[] expected = bytes(input);
                String[] pool = new String[input.readInt()];
                for (int j = 0; j < pool.length; j++) {
                    char[] text = new char[input.readInt()];
                    for (int k = 0; k < text.length; k++) text[k] = input.readChar();
                    pool[j] = new String(text);
                }
                byte[] actual;
                try {
                    if (kind == 0) {
                        actual = new byte[expected.length];
                        int length = new ZstdFrameDecompressor().decompress(encoded, 0, encoded.length, actual, 0, actual.length);
                        if (length != actual.length) throw new IOException("Decoded length mismatch");
                    } else actual = ClassFiles.restore(encoded, pool, expected.length);
                } catch (IOException | RuntimeException rejected) {
                    if (valid) throw new AssertionError("Rejected valid vector " + i, rejected);
                    continue;
                }
                if (!valid) throw new AssertionError("Accepted invalid vector " + i);
                if (!Arrays.equals(expected, actual)) throw new AssertionError("Incorrect vector " + i);
            }
            if (input.read() != -1) throw new AssertionError("Trailing fixture data");
            System.out.println("Verified " + count + " codec vectors");
        }
    }
}

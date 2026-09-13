// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.DataInputStream;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.Arrays;

import org.janex.bootstrap.internal.zstd.Zstandard;

/// Compares dictionary decoding with independently generated native Zstandard results.
public final class DictionaryTest {
    /// Prevents instantiation.
    private DictionaryTest() {
    }

    /// Reads a length-prefixed byte array from the trusted fixture stream.
    private static byte[] bytes(DataInputStream input) throws Exception {
        byte[] bytes = new byte[input.readInt()];
        input.readFully(bytes);
        return bytes;
    }

    /// Checks native vectors, input preservation, output slices, and exact failure types.
    ///
    /// @param arguments path to the native fixture stream
    /// @throws Exception if fixture reading or a successful decode fails
    public static void main(String[] arguments) throws Exception {
        try (DataInputStream input = new DataInputStream(Files.newInputStream(Paths.get(arguments[0])))) {
            int count = input.readInt();
            for (int i = 0; i < count; i++) {
                boolean valid = input.readBoolean();
                byte[] dictionary = bytes(input);
                byte[] encoded = bytes(input);
                byte[] expected = bytes(input);
                byte[] savedDictionary = dictionary.clone();
                byte[] padded = new byte[encoded.length + 14];
                System.arraycopy(encoded, 0, padded, 7, encoded.length);
                byte[] savedInput = padded.clone();
                byte[] output = new byte[expected.length + 18];
                Arrays.fill(output, (byte) 0x5a);
                try {
                    int written = Zstandard.decompress(padded, 7, encoded.length, output, 9, expected.length, dictionary);
                    if (!valid) {
                        throw new AssertionError("Malformed dictionary vector accepted: " + i);
                    }
                    if (written != expected.length || !Arrays.equals(expected, Arrays.copyOfRange(output, 9, 9 + written))) {
                        throw new AssertionError("Dictionary output mismatch: " + i);
                    }
                } catch (IllegalArgumentException failure) {
                    if (valid) {
                        throw new AssertionError("Valid dictionary vector rejected: " + i, failure);
                    }
                }
                for (int j = 0; j < 9; j++) {
                    if (output[j] != 0x5a || output[output.length - 1 - j] != 0x5a) {
                        throw new AssertionError("Output slice overwritten: " + i);
                    }
                }
                if (!Arrays.equals(dictionary, savedDictionary) || !Arrays.equals(padded, savedInput)) {
                    throw new AssertionError("Input modified: " + i);
                }
            }
            if (input.read() != -1) {
                throw new AssertionError("Trailing fixture bytes");
            }
            System.out.println("Verified " + count + " dictionary vectors");
        }
    }
}

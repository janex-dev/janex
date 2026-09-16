// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal.codec.zstd;

import java.util.Random;

/// Compares cached reverse reads against a bit-by-bit oracle across slice and window boundaries.
public final class ReverseBitsTest {
    /// Prevents instantiation.
    private ReverseBitsTest() {
    }

    /// Checks all short stream lengths, mixed read widths, padding, and exhausted reads.
    /// @param args unused
    public static void main(String[] args) {
        Random random = new Random(718);
        for (int length = 0; length <= 2048; length++) {
            int start = length % 7;
            byte[] data = new byte[start + (length >>> 3) + 5];
            random.nextBytes(data);
            int last = start + (length >>> 3);
            data[last] = (byte) ((data[last] & ((1 << (length & 7)) - 1)) | (1 << (length & 7)));
            Input input = new Input(data, start, last + 1);
            ReverseBits bits = new ReverseBits(input);
            int remaining = length;
            if (input.position != last + 1 || bits.remaining != remaining) {
                throw new AssertionError("Incorrect stream boundary");
            }
            while (remaining > 0) {
                for (int width = 0; width <= 31; width++) {
                    int expected = 0;
                    for (int i = 0; i < width; i++) {
                        int bit = remaining - i - 1;
                        expected = (expected << 1) | (bit < 0 ? 0 : (data[start + (bit >>> 3)] >>> (bit & 7)) & 1);
                    }
                    if (bits.peek(width) != expected || bits.remaining != remaining) {
                        throw new AssertionError("Incorrect peek at " + length + "/" + remaining + "/" + width);
                    }
                }
                int count = Math.min(remaining, 1 + random.nextInt(31));
                int expected = bits.peek(count);
                if (bits.read(count) != expected) {
                    throw new AssertionError("Read differs from peek");
                }
                remaining -= count;
            }
            bits.finish();
            if (bits.peek(31) != 0 || bits.read(0) != 0) {
                throw new AssertionError("Incorrect exhausted padding");
            }
            try {
                bits.read(1);
                throw new AssertionError("Truncated read accepted");
            } catch (IllegalArgumentException expected) {
                if (bits.remaining != 0) {
                    throw new AssertionError("Failed read consumed input");
                }
            }
        }
    }
}

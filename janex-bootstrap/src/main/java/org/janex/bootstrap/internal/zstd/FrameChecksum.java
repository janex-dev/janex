// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap.internal.zstd;

/// Computes the low 32 bits of seed-zero XXH64 required by Zstandard frame checksums.
final class FrameChecksum {
    /// Multiplicative constant for lanes and byte tails.
    private static final long P1 = 0x9e3779b185ebca87L;
    /// Multiplicative constant for lane input and final mixing.
    private static final long P2 = 0xc2b2ae3d27d4eb4fL;
    /// Multiplicative constant for the four-byte tail and final mixing.
    private static final long P3 = 0x165667b19e3779f9L;
    /// Additive constant for merged lanes and eight-byte tails.
    private static final long P4 = 0x85ebca77c2b2ae63L;
    /// Initial value for short input and multiplier for individual bytes.
    private static final long P5 = 0x27d4eb2f165667c5L;

    /// Prevents instantiation.
    private FrameChecksum() {
    }

    /// Mixes an eight-byte input lane into an accumulator.
    private static long lane(long accumulator, long value) {
        return Long.rotateLeft(accumulator + value * P2, 31) * P1;
    }

    /// Hashes an already validated array slice without modifying it.
    static int calculate(byte[] bytes, int start, int end) {
        Input input = new Input(bytes, start, end);
        long hash = P5;
        if (input.remaining() >= 32) {
            long[] lanes = {P1 + P2, P2, 0, -P1};
            do {
                for (int i = 0; i < lanes.length; i++) {
                    lanes[i] = lane(lanes[i], input.little(8));
                }
            } while (input.remaining() >= 32);
            hash = Long.rotateLeft(lanes[0], 1) + Long.rotateLeft(lanes[1], 7)
                    + Long.rotateLeft(lanes[2], 12) + Long.rotateLeft(lanes[3], 18);
            for (long value : lanes) {
                hash = (hash ^ lane(0, value)) * P1 + P4;
            }
        }
        hash += end - start;
        while (input.remaining() >= 8) {
            hash = Long.rotateLeft(hash ^ lane(0, input.little(8)), 27) * P1 + P4;
        }
        if (input.remaining() >= 4) {
            hash = Long.rotateLeft(hash ^ input.little(4) * P1, 23) * P2 + P3;
        }
        while (input.remaining() > 0) {
            hash = Long.rotateLeft(hash ^ input.octet() * P5, 11) * P1;
        }
        hash = (hash ^ (hash >>> 33)) * P2;
        hash = (hash ^ (hash >>> 29)) * P3;
        return (int) (hash ^ (hash >>> 32));
    }
}

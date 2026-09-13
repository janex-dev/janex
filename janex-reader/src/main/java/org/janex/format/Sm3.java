// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.security.MessageDigest;
import java.util.Arrays;

/// Implements the SM3 compression function and standard big-endian digest using Java 8 operations.
///
/// Algorithm description: https://datatracker.ietf.org/doc/html/draft-sca-cfrg-sm3-02#section-5.
final class Sm3 extends MessageDigest {
    /// Eight chaining words initialized by the SM3 specification.
    private final int[] state = new int[8];
    /// Expanded words reused by each compression block.
    private final int[] words = new int[68];
    /// Incomplete input block and padding storage.
    private final byte[] block = new byte[64];
    /// Number of currently buffered bytes.
    private int buffered;
    /// Total input bytes modulo 2^64.
    private long length;

    /// Creates an empty SM3 state.
    Sm3() {
        super("SM3");
        engineReset();
    }

    /// Reports the standard 256-bit digest size.
    @Override
    protected int engineGetDigestLength() {
        return 32;
    }

    /// Restores the initial state and discards buffered bytes.
    @Override
    protected void engineReset() {
        int[] initial = {0x7380166f, 0x4914b2b9, 0x172442d7, 0xda8a0600,
                0xa96f30bc, 0x163138aa, 0xe38dee4d, 0xb0fb0e4e};
        System.arraycopy(initial, 0, state, 0, state.length);
        Arrays.fill(block, (byte) 0);
        buffered = 0;
        length = 0;
    }

    /// Consumes one byte, compressing a completed block immediately.
    @Override
    protected void engineUpdate(byte value) {
        block[buffered++] = value;
        length++;
        if (buffered == block.length) {
            compress(block, 0);
            buffered = 0;
        }
    }

    /// Consumes a validated slice with direct compression of complete blocks.
    @Override
    protected void engineUpdate(byte[] input, int offset, int count) {
        length += count;
        while (count > 0) {
            if (buffered == 0 && count >= 64) {
                compress(input, offset);
                offset += 64;
                count -= 64;
            } else {
                int copied = Math.min(count, block.length - buffered);
                System.arraycopy(input, offset, block, buffered, copied);
                offset += copied;
                count -= copied;
                buffered += copied;
                if (buffered == block.length) {
                    compress(block, 0);
                    buffered = 0;
                }
            }
        }
    }

    /// Pads the current message, returns its canonical digest, and resets this state.
    @Override
    protected byte[] engineDigest() {
        long bits = length << 3;
        engineUpdate((byte) 0x80);
        while (buffered != 56) {
            engineUpdate((byte) 0);
        }
        for (int shift = 56; shift >= 0; shift -= 8) {
            engineUpdate((byte) (bits >>> shift));
        }
        byte[] result = new byte[32];
        for (int index = 0; index < state.length; index++) {
            for (int octet = 0; octet < 4; octet++) {
                result[index * 4 + octet] = (byte) (state[index] >>> (24 - octet * 8));
            }
        }
        engineReset();
        return result;
    }

    /// Expands one 512-bit block and applies the 64 SM3 rounds to the chaining words.
    private void compress(byte[] input, int offset) {
        for (int index = 0; index < 16; index++) {
            int word = 0;
            for (int octet = 0; octet < 4; octet++) {
                word = (word << 8) | (input[offset + index * 4 + octet] & 255);
            }
            words[index] = word;
        }
        for (int index = 16; index < words.length; index++) {
            int value = words[index - 16] ^ words[index - 9] ^ Integer.rotateLeft(words[index - 3], 15);
            words[index] = value ^ Integer.rotateLeft(value, 15) ^ Integer.rotateLeft(value, 23)
                    ^ Integer.rotateLeft(words[index - 13], 7) ^ words[index - 6];
        }
        int a = state[0], b = state[1], c = state[2], d = state[3];
        int e = state[4], f = state[5], g = state[6], h = state[7];
        for (int round = 0; round < 64; round++) {
            int rotatedA = Integer.rotateLeft(a, 12);
            int first = Integer.rotateLeft(rotatedA + e + Integer.rotateLeft(round < 16 ? 0x79cc4519 : 0x7a879d8a, round), 7);
            int ff = round < 16 ? a ^ b ^ c : (a & b) | (a & c) | (b & c);
            int gg = round < 16 ? e ^ f ^ g : (e & f) | (~e & g);
            int nextA = ff + d + (first ^ rotatedA) + (words[round] ^ words[round + 4]);
            int nextE = gg + h + first + words[round];
            d = c;
            c = Integer.rotateLeft(b, 9);
            b = a;
            a = nextA;
            h = g;
            g = Integer.rotateLeft(f, 19);
            f = e;
            e = nextE ^ Integer.rotateLeft(nextE, 9) ^ Integer.rotateLeft(nextE, 17);
        }
        state[0] ^= a;
        state[1] ^= b;
        state[2] ^= c;
        state[3] ^= d;
        state[4] ^= e;
        state[5] ^= f;
        state[6] ^= g;
        state[7] ^= h;
    }
}

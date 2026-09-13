// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader.internal.codec;

import java.security.MessageDigest;

/// Computes canonical XXH3-64 or XXH3-128 with seed zero and the standard secret.
///
/// Algorithm definition: https://github.com/Cyan4973/xxHash/blob/dev/doc/xxhash_spec.md.
/// Only a final 1024-byte block and the last 64 input bytes are buffered.
public final class Xxh3 extends MessageDigest {
    /// First 64-bit prime used for length mixing and products.
    private static final long P1 = 0x9e3779b185ebca87L;
    /// Second 64-bit prime used for length mixing and avalanche.
    private static final long P2 = 0xc2b2ae3d27d4eb4fL;
    /// Third 64-bit prime used for avalanche and initial state.
    private static final long P3 = 0x165667b19e3779f9L;
    /// Fourth 64-bit prime used for 128-bit finalization.
    private static final long P4 = 0x85ebca77c2b2ae63L;
    /// Fifth 64-bit prime used in the initial state.
    private static final long P5 = 0x27d4eb2f165667c5L;
    /// First 32-bit prime, widened without sign extension.
    private static final long Q1 = 0x9e3779b1L;
    /// Second 32-bit prime, widened without sign extension.
    private static final long Q2 = 0x85ebca77L;
    /// Third 32-bit prime, widened without sign extension.
    private static final long Q3 = 0xc2b2ae3dL;
    /// Multiplier for the XXH3 avalanche operation.
    private static final long M1 = 0x165667919e3779f9L;
    /// Multiplier for the four-to-eight-byte finalizer.
    private static final long M2 = 0x9fb21c651e98df25L;
    /// Standard 192-byte secret, stored as little-endian words.
    private static final long[] SECRET = {
            0xbe4ba423396cfeb8L, 0x1cad21f72c81017cL, 0xdb979083e96dd4deL, 0x1f67b3b7a4a44072L,
            0x78e5c0cc4ee679cbL, 0x2172ffcc7dd05a82L, 0x8e2443f7744608b8L, 0x4c263a81e69035e0L,
            0xcb00c391bb52283cL, 0xa32e531b8b65d088L, 0x4ef90da297486471L, 0xd8acdea946ef1938L,
            0x3f349ce33f76faa8L, 0x1d4f0bc7c7bbdcf9L, 0x3159b4cd4be0518aL, 0x647378d9c97e9fc8L,
            0xc3ebd33483acc5eaL, 0xeb6313faffa081c5L, 0x49daf0b751dd0d17L, 0x9e68d429265516d3L,
            0xfca1477d58be162bL, 0xce31d07ad1b8f88fL, 0x280416958f3acb45L, 0x7e404bbbcafbd7afL
    };
    /// Whether the output contains both 64-bit halves.
    private final boolean wide;
    /// Eight independent accumulation lanes for long messages.
    private final long[] lanes = new long[8];
    /// Final unprocessed block, retained even when exactly full.
    private final byte[] block = new byte[1024];
    /// Last input stripe, including bytes overlapping a previously processed block.
    private final byte[] tail = new byte[64];
    /// Number of buffered final-block bytes.
    private int buffered;
    /// Number of initialized bytes in the trailing stripe.
    private int tailLength;
    /// Total input byte length modulo 2^64.
    private long length;
    /// Whether the message has passed the short/medium input boundary.
    private boolean large;

    /// Creates an empty state for one output width.
    public Xxh3(boolean wide) {
        super(wide ? "XXH3-128" : "XXH3-64");
        this.wide = wide;
        engineReset();
    }

    /// Returns the selected canonical digest length.
    @Override
    protected int engineGetDigestLength() {
        return wide ? 16 : 8;
    }

    /// Discards the message and restores the standard initial lanes.
    @Override
    protected void engineReset() {
        long[] initial = {Q3, P1, P2, P3, P4, Q2, P5, Q1};
        System.arraycopy(initial, 0, lanes, 0, lanes.length);
        buffered = 0;
        tailLength = 0;
        length = 0;
        large = false;
    }

    /// Adds one input byte without allocating temporary storage.
    @Override
    protected void engineUpdate(byte value) {
        if (buffered == block.length) {
            processBlock(block, 0);
            buffered = 0;
        }
        block[buffered++] = value;
        if (tailLength == tail.length) {
            System.arraycopy(tail, 1, tail, 0, tail.length - 1);
            tailLength--;
        }
        tail[tailLength++] = value;
        length++;
        large |= length > 240;
    }

    /// Adds a validated array slice, processing complete nonfinal blocks without copying them.
    @Override
    protected void engineUpdate(byte[] input, int offset, int count) {
        if (count == 0) {
            return;
        }
        int copiedTail = Math.min(count, tail.length);
        int retained = Math.min(tailLength, tail.length - copiedTail);
        System.arraycopy(tail, tailLength - retained, tail, 0, retained);
        System.arraycopy(input, offset + count - copiedTail, tail, retained, copiedTail);
        tailLength = retained + copiedTail;
        length += count;
        large |= length > 240;
        while (count > 0) {
            if (buffered == block.length) {
                processBlock(block, 0);
                buffered = 0;
            }
            if (buffered == 0 && count > block.length) {
                processBlock(input, offset);
                offset += block.length;
                count -= block.length;
            } else {
                int copied = Math.min(count, block.length - buffered);
                System.arraycopy(input, offset, block, buffered, copied);
                buffered += copied;
                offset += copied;
                count -= copied;
            }
        }
    }

    /// Finalizes the message in canonical big-endian order and resets the state.
    @Override
    protected byte[] engineDigest() {
        long[] result;
        if (large) {
            for (int stripe = 0; stripe < (buffered - 1) / 64; stripe++) {
                accumulate(block, stripe * 64, stripe * 8);
            }
            accumulate(tail, 0, 121);
            result = new long[]{merge(length * P1, 11), wide ? merge(~(length * P2), 117) : 0};
        } else {
            result = buffered <= 16 ? small(buffered) : medium(buffered);
        }
        byte[] bytes = new byte[engineGetDigestLength()];
        for (int index = 0; index < bytes.length; index++) {
            int half = wide && index < 8 ? 1 : 0;
            bytes[index] = (byte) (result[half] >>> (56 - (index & 7) * 8));
        }
        engineReset();
        return bytes;
    }

    /// Accumulates one complete nonfinal block and scrambles its lanes.
    private void processBlock(byte[] input, int offset) {
        for (int stripe = 0; stripe < 16; stripe++) {
            accumulate(input, offset + stripe * 64, stripe * 8);
        }
        for (int lane = 0; lane < lanes.length; lane++) {
            lanes[lane] = (lanes[lane] ^ (lanes[lane] >>> 47) ^ secret(128 + lane * 8)) * Q1;
        }
    }

    /// Adds one input stripe using eight secret-selected lane products.
    private void accumulate(byte[] input, int offset, int secretOffset) {
        for (int lane = 0; lane < lanes.length; lane++) {
            long value = little(input, offset + lane * 8, 8);
            long mixed = value ^ secret(secretOffset + lane * 8);
            lanes[lane ^ 1] += value;
            lanes[lane] += (mixed & 0xffffffffL) * (mixed >>> 32);
        }
    }

    /// Merges accumulated lanes into one finalized 64-bit half.
    private long merge(long result, int secretOffset) {
        for (int lane = 0; lane < lanes.length; lane += 2) {
            result += folded(lanes[lane] ^ secret(secretOffset + lane * 8),
                    lanes[lane + 1] ^ secret(secretOffset + lane * 8 + 8));
        }
        return avalanche(result);
    }

    /// Computes the distinct short-input cases, returning low and high halves.
    private long[] small(int size) {
        if (size == 0) {
            return wide ? new long[]{avalanche64(secret(64) ^ secret(72)), avalanche64(secret(80) ^ secret(88))}
                    : new long[]{avalanche64(secret(56) ^ secret(64)), 0};
        }
        if (size <= 3) {
            int combined = (block[size - 1] & 255) | (size << 8) | ((block[0] & 255) << 16) | ((block[size / 2] & 255) << 24);
            long low = (combined & 0xffffffffL) ^ ((secret(0) ^ (secret(0) >>> 32)) & 0xffffffffL);
            long high = (Integer.rotateLeft(Integer.reverseBytes(combined), 13) & 0xffffffffL)
                    ^ ((secret(8) ^ (secret(8) >>> 32)) & 0xffffffffL);
            return new long[]{avalanche64(low), wide ? avalanche64(high) : 0};
        }
        if (size <= 8) {
            long first = little(block, 0, 4);
            long last = little(block, size - 4, 4);
            if (!wide) {
                long value = (last | (first << 32)) ^ secret(8) ^ secret(16);
                value ^= Long.rotateLeft(value, 49) ^ Long.rotateLeft(value, 24);
                value *= M2;
                value ^= (value >>> 35) + size;
                value *= M2;
                return new long[]{value ^ (value >>> 28), 0};
            }
            long value = (first | (last << 32)) ^ secret(16) ^ secret(24);
            long factor = P1 + (size << 2);
            long low = value * factor;
            long high = highProduct(value, factor) + (low << 1);
            low ^= high >>> 3;
            low = (low ^ (low >>> 35)) * M2;
            return new long[]{low ^ (low >>> 28), avalanche(high)};
        }
        long first = little(block, 0, 8);
        long last = little(block, size - 8, 8);
        if (!wide) {
            long low = first ^ secret(24) ^ secret(32);
            long high = last ^ secret(40) ^ secret(48);
            return new long[]{avalanche(size + Long.reverseBytes(low) + high + folded(low, high)), 0};
        }
        long value = first ^ last ^ secret(32) ^ secret(40);
        long other = last ^ secret(48) ^ secret(56);
        long low = value * P1 + ((long) (size - 1) << 54);
        long high = highProduct(value, P1) + other + (other & 0xffffffffL) * (Q2 - 1);
        low ^= Long.reverseBytes(high);
        high = highProduct(low, P2) + high * P2;
        return new long[]{avalanche(low * P2), avalanche(high)};
    }

    /// Computes medium inputs in paired chunks with width-specific accumulation.
    private long[] medium(int size) {
        long[] accumulators = {size * P1, 0};
        if (size <= 128) {
            for (int round = (size - 1) / 32; round >= 0; round--) {
                int first = round * 16;
                int last = size - first - 16;
                mixPair(accumulators, first, last, round * 32);
            }
        } else {
            for (int offset = 0; offset < 128; offset += 32) {
                mixPair(accumulators, offset, offset + 16, offset);
            }
            accumulators[0] = avalanche(accumulators[0]);
            if (wide) {
                accumulators[1] = avalanche(accumulators[1]);
                for (int offset = 128; offset <= size - 32; offset += 32) {
                    mixPair(accumulators, offset, offset + 16, offset - 125);
                }
                mixPair(accumulators, size - 16, size - 32, 103);
            } else {
                for (int offset = 128; offset <= size - 16; offset += 16) {
                    accumulators[0] += mix(offset, offset - 125);
                }
                accumulators[0] += mix(size - 16, 119);
            }
        }
        long low = avalanche(accumulators[0] + accumulators[1]);
        long high = wide ? -avalanche(accumulators[0] * P1 + accumulators[1] * P4 + size * P2) : 0;
        return new long[]{low, high};
    }

    /// Mixes two 16-byte chunks into the selected output-width accumulator.
    private void mixPair(long[] values, int first, int second, int secretOffset) {
        values[0] += mix(first, secretOffset);
        if (wide) {
            values[1] += mix(second, secretOffset + 16);
            values[0] ^= little(block, second, 8) + little(block, second + 8, 8);
            values[1] ^= little(block, first, 8) + little(block, first + 8, 8);
        } else {
            values[0] += mix(second, secretOffset + 16);
        }
    }

    /// Folds a keyed 16-byte input chunk into one 64-bit value.
    private long mix(int offset, int secretOffset) {
        return folded(little(block, offset, 8) ^ secret(secretOffset), little(block, offset + 8, 8) ^ secret(secretOffset + 8));
    }

    /// Reads an unaligned eight-byte window from the fixed little-endian secret.
    private static long secret(int offset) {
        int index = offset >>> 3;
        int shift = (offset & 7) * 8;
        return shift == 0 ? SECRET[index] : (SECRET[index] >>> shift) | (SECRET[index + 1] << (64 - shift));
    }

    /// Reads a validated four- or eight-byte little-endian input word.
    private static long little(byte[] input, int offset, int count) {
        long result = 0;
        for (int index = 0; index < count; index++) {
            result |= (long) (input[offset + index] & 255) << (index * 8);
        }
        return result;
    }

    /// Returns the unsigned high half of a 64-by-64-bit product using four 32-bit limbs.
    private static long highProduct(long first, long second) {
        long a = first & 0xffffffffL;
        long b = first >>> 32;
        long c = second & 0xffffffffL;
        long d = second >>> 32;
        long low = a * c;
        long middle = b * c + (low >>> 32);
        long carry = middle >>> 32;
        long remaining = (middle & 0xffffffffL) + a * d;
        return b * d + carry + (remaining >>> 32);
    }

    /// XORs both halves of an unsigned 128-bit product.
    private static long folded(long first, long second) {
        return first * second ^ highProduct(first, second);
    }

    /// Applies the XXH3 avalanche function.
    private static long avalanche(long value) {
        value = (value ^ (value >>> 37)) * M1;
        return value ^ (value >>> 32);
    }

    /// Applies the XXH64 avalanche used by the shortest XXH3 input cases.
    private static long avalanche64(long value) {
        value = (value ^ (value >>> 33)) * P2;
        value = (value ^ (value >>> 29)) * P3;
        return value ^ (value >>> 32);
    }
}

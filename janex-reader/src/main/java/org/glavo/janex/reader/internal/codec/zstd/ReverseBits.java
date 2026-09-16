// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal.codec.zstd;

import static org.glavo.janex.reader.internal.codec.zstd.Input.require;

/// Reads little-endian fields backwards from a stream terminated by a set bit.
final class ReverseBits {
    /// Encoded bytes, borrowed for the duration of decoding.
    private final byte[] bytes;
    /// First byte of the encoded stream.
    private final int start;
    /// Number of unconsumed data bits, excluding the terminator.
    int remaining;
    /// Low bit offset of the cached window, or negative before the first read.
    private int windowLow = -1;
    /// Up to eight input bytes in little-endian order.
    private long window;

    /// Consumes a section as one bitstream, rejecting an absent end marker.
    ReverseBits(Input input) {
        require(input.remaining() > 0, "empty entropy stream");
        bytes = input.bytes;
        start = input.position;
        int last = bytes[input.end - 1] & 255;
        require(last != 0, "missing entropy end marker");
        remaining = (input.remaining() - 1) * 8 + 31 - Integer.numberOfLeadingZeros(last);
        input.position = input.end;
    }

    /// Returns the next at most 31 bits, padding unavailable low bits with zero.
    int peek(int count) {
        int available = Math.min(count, remaining);
        if (available == 0) {
            return 0;
        }
        int low = remaining - available;
        if (windowLow < 0 || low < windowLow) {
            int end = (remaining + 7) >>> 3;
            int first = Math.max(0, end - 8);
            windowLow = first << 3;
            long value = 0;
            for (int i = first; i < end; i++) {
                value |= (long) (bytes[start + i] & 255) << ((i - first) * 8);
            }
            window = value;
        }
        return (int) (((window >>> (low - windowLow)) & ((1L << available) - 1)) << (count - available));
    }

    /// Consumes at most 31 bits, rejecting a truncated field.
    int read(int count) {
        require(count <= remaining, "truncated entropy stream");
        int value = peek(count);
        remaining -= count;
        return value;
    }

    /// Rejects an incompletely consumed entropy stream.
    void finish() {
        require(remaining == 0, "trailing entropy bits");
    }
}

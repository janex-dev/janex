// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader.internal.codec.zstd;

import static org.janex.reader.internal.codec.zstd.Input.require;

/// Reads little-endian fields backwards from a stream terminated by a set bit.
final class ReverseBits {
    /// Encoded bytes, borrowed for the duration of decoding.
    private final byte[] bytes;
    /// First byte of the encoded stream.
    private final int start;
    /// Number of unconsumed data bits, excluding the terminator.
    int remaining;

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
        int low = remaining - available;
        int index = start + (low >>> 3);
        int shift = low & 7;
        long value = 0;
        int byteCount = (shift + available + 7) >>> 3;
        for (int i = 0; i < byteCount; i++) {
            value |= (long) (bytes[index + i] & 255) << (i * 8);
        }
        return (int) (((value >>> shift) & ((1L << available) - 1)) << (count - available));
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

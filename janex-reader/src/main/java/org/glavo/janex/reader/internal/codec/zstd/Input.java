// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal.codec.zstd;

/// Reads a bounded, forward byte slice without taking ownership of its array.
final class Input {
    /// Backing input, which callers must not modify during decoding.
    final byte[] bytes;
    /// Exclusive slice boundary.
    final int end;
    /// Position of the next unread byte.
    int position;

    /// Creates a cursor over an already validated array range.
    Input(byte[] bytes, int start, int end) {
        this.bytes = bytes;
        this.position = start;
        this.end = end;
    }

    /// Rejects an invalid format condition with a diagnostic.
    static void require(boolean condition, String message) {
        if (!condition) {
            throw new IllegalArgumentException("Invalid Zstandard data: " + message);
        }
    }

    /// Returns the unread byte count.
    int remaining() {
        return end - position;
    }

    /// Reads one unsigned byte, rejecting a truncated slice.
    int octet() {
        require(position < end, "truncated input");
        return bytes[position++] & 255;
    }

    /// Reads up to eight little-endian bytes; eight bytes retain unsigned bits in a long.
    long little(int count) {
        require(count <= remaining(), "truncated integer");
        long value = 0;
        for (int i = 0; i < count; i++) {
            value |= (long) octet() << (i * 8);
        }
        return value;
    }

    /// Advances over a bounded child slice and returns its independent cursor.
    Input take(int length) {
        require(length >= 0 && length <= remaining(), "truncated section");
        Input part = new Input(bytes, position, position + length);
        position += length;
        return part;
    }

    /// Rejects bytes left over after decoding a complete section.
    void finish() {
        require(position == end, "trailing section bytes");
    }
}

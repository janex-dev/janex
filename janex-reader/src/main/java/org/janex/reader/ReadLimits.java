// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader;

import java.io.IOException;

import org.janex.reader.internal.Input;

/// Bounds individual buffered values, collection sizes, and structured nesting.
///
/// These immutable limits are reader policy, not file-format constraints. Byte limits apply to
/// encoded and decoded values, not the total size of a container scanned for integrity. Java
/// arrays and collections additionally require lengths representable by a nonnegative int.
public final class ReadLimits {
    /// Default policy: 256 MiB per value, 1,000,000 elements per collection, and nesting depth 64.
    public static final ReadLimits DEFAULT = new ReadLimits(256 * 1024 * 1024, 1_000_000, 64);

    /// Maximum bytes buffered for one encoded or decoded value.
    private final int maxBytes;
    /// Maximum elements in one collection.
    private final int maxElements;
    /// Maximum structured nesting depth, with the outermost value at depth zero.
    private final int maxDepth;

    /// Creates a reader policy; zero permits only zero-length values or collections at that bound.
    ///
    /// @param maxBytes maximum bytes in one buffered value, nonnegative
    /// @param maxElements maximum elements in one collection, nonnegative
    /// @param maxDepth maximum nesting depth, nonnegative; zero permits only the outermost value
    /// @throws IllegalArgumentException if any limit is negative
    public ReadLimits(int maxBytes, int maxElements, int maxDepth) {
        if (maxBytes < 0 || maxElements < 0 || maxDepth < 0) {
            throw new IllegalArgumentException("Negative read limit");
        }
        this.maxBytes = maxBytes;
        this.maxElements = maxElements;
        this.maxDepth = maxDepth;
    }

    /// Returns the maximum bytes in one buffered value.
    public int maxBytes() {
        return maxBytes;
    }

    /// Returns the maximum elements in one collection.
    public int maxElements() {
        return maxElements;
    }

    /// Returns the maximum structured nesting depth.
    public int maxDepth() {
        return maxDepth;
    }

    /// Checks an unsigned byte length, rejecting negative long representations as oversized.
    public int bytes(long value) throws IOException {
        if (value < 0 || value > maxBytes) {
            throw new IOException("Janex byte limit exceeded");
        }
        return (int) value;
    }

    /// Checks an unsigned collection length before allocation or insertion.
    public int elements(long value) throws IOException {
        if (value < 0 || value > maxElements) {
            throw new IOException("Janex element limit exceeded");
        }
        return (int) value;
    }

    /// Checks structured nesting independently of malformed-input diagnostics.
    public void depth(int value) throws IOException {
        if (value < 0 || value > maxDepth) {
            throw new IOException("Janex nesting limit exceeded");
        }
    }

    /// Checks the UTF-8 length of well-formed Unicode text without allocating encoded bytes.
    int text(String value) throws IOException {
        long length = 0;
        for (int i = 0; i < value.length(); i++) {
            char unit = value.charAt(i);
            if (unit < 0x80) {
                length++;
            } else if (unit < 0x800) {
                length += 2;
            } else if (Character.isHighSurrogate(unit) && i + 1 < value.length()
                    && Character.isLowSurrogate(value.charAt(i + 1))) {
                length += 4;
                i++;
            } else {
                Input.require(!Character.isSurrogate(unit), "Unpaired surrogate in resource text");
                length += 3;
            }
        }
        return bytes(length);
    }
}

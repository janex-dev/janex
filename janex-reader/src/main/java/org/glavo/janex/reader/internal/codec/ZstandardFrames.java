// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal.codec;

import java.io.IOException;

import org.glavo.janex.reader.ReadLimits;
import org.glavo.janex.reader.internal.Input;

import static org.glavo.janex.reader.internal.Input.require;

/// Checks Zstandard frame boundaries and the native reader's advertised-window limit.
///
/// This scanner does not decompress blocks, validate dictionaries, or verify frame checksums.
/// A decoder must still consume the complete input and verify the required decoded length.
public final class ZstandardFrames {
    /// Prevents instantiation.
    private ZstandardFrames() {
    }

    /// Checks complete ordinary and skippable frames without allocating a decoding window.
    ///
    /// The window cap is the power of two strictly greater than the byte limit, clamped to
    /// 1 KiB through 2 GiB, matching native Zstandard preparation. At least one ordinary frame
    /// must be present. Input bytes are not modified or retained.
    ///
    /// @param bytes complete encoded frames, nonnull
    /// @param limits nonnull input-length and decoding-window policy
    /// @throws IOException if framing is malformed or an input or window bound is exceeded
    public static void validate(byte[] bytes, ReadLimits limits) throws IOException {
        Input input = new Input(bytes, limits);
        int log = Math.max(10, Math.min(31, 32 - Integer.numberOfLeadingZeros(Math.max(1, limits.maxBytes()))));
        long windowLimit = 1L << log;
        boolean ordinary = false;
        while (input.remaining() != 0) {
            long magic = input.little(4);
            if ((magic & 0xfffffff0L) == 0x184d2a50L) {
                skip(input, input.little(4));
                continue;
            }
            require(magic == 0xfd2fb528L, "Invalid Zstandard frame magic");
            ordinary = true;
            int descriptor = input.u8();
            require((descriptor & 8) == 0, "Reserved Zstandard frame flag");
            boolean single = (descriptor & 32) != 0;
            long window = 0;
            if (!single) {
                int value = input.u8();
                long base = 1L << (10 + (value >>> 3));
                window = base + (base >>> 3) * (value & 7);
            }
            int dictionary = descriptor & 3;
            skip(input, dictionary == 3 ? 4 : dictionary);
            int sizeFlag = descriptor >>> 6;
            int sizeWidth = sizeFlag == 0 ? single ? 1 : 0 : 1 << sizeFlag;
            long contentSize = input.little(sizeWidth);
            if (sizeWidth == 2) {
                contentSize += 256;
            }
            if (single) {
                window = contentSize;
            }
            if (window < 0 || window > windowLimit) {
                throw new IOException("Zstandard window limit exceeded");
            }
            boolean last;
            do {
                long header = input.little(3);
                last = (header & 1) != 0;
                int kind = (int) ((header >>> 1) & 3);
                long length = header >>> 3;
                require(kind != 3 && length <= Math.min(window, 128 * 1024), "Invalid Zstandard block header");
                skip(input, kind == 1 ? 1 : length);
            } while (!last);
            if ((descriptor & 4) != 0) {
                skip(input, 4);
            }
        }
        require(ordinary, "Missing ordinary Zstandard frame");
    }

    /// Advances across a bounded encoded field without copying it.
    private static void skip(Input input, long length) throws IOException {
        require(length >= 0 && length <= input.remaining(), "Truncated Zstandard frame");
        input.skip(length);
    }
}

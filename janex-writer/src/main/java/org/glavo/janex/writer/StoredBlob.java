// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.util.Arrays;

import io.airlift.compress.zstd.ZstdCompressor;

/// Holds stored bytes and their BlobEncoding description.
/// @param bytes encoded content
/// @param encoding binary BlobEncoding
record StoredBlob(byte[] bytes, byte[] encoding) {
    /// Creates a stored representation without copying its arrays.
    StoredBlob { }

    /// Chooses compression only when bytes and their enclosing description together shrink.
    /// The offset is relative to the pool payload; page descriptions use CBOR byte strings.
    static StoredBlob encode(byte[] input, boolean compression, int offset, boolean page) throws IOException {
        StoredBlob raw = new StoredBlob(input, encoding(input.length, -1));
        if (!compression || input.length == 0) return raw;
        ZstdCompressor compressor = new ZstdCompressor();
        int capacity = compressor.maxCompressedLength(input.length);
        if (capacity < input.length) return raw;
        byte[] buffer = new byte[capacity];
        int length = compressor.compress(input, 0, input.length, buffer, 0, buffer.length);
        byte[] encoding = encoding(length, input.length);
        if ((long) length + overhead(encoding, offset, page)
                >= (long) input.length + overhead(raw.encoding, offset, page)) return raw;
        return new StoredBlob(Arrays.copyOf(buffer, length), encoding);
    }

    /// Encodes raw storage for a negative input length, or one dictionary-free Zstandard filter.
    private static byte[] encoding(int storedLength, int inputLength) {
        Encoding output = new Encoding();
        output.uint(storedLength);
        output.uint(inputLength < 0 ? 0 : 1);
        if (inputLength >= 0) {
            output.uint(inputLength);
            output.write(1);
            output.write(0);
        }
        return output.toByteArray();
    }

    /// Counts the encoded description, including its CBOR or Stored-entry length prefix.
    private static int overhead(byte[] encoding, int offset, boolean page) throws IOException {
        if (page) return Encoding.cbor(encoding).length;
        Encoding payload = new Encoding();
        payload.uint(offset);
        payload.writeBytes(encoding);
        Encoding entry = new Encoding();
        entry.write(0);
        entry.sized(payload.toByteArray());
        return entry.size();
    }
}

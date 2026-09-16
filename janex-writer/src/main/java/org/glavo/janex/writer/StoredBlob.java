// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.util.Arrays;

import com.github.luben.zstd.Zstd;
import com.github.luben.zstd.ZstdCompressCtx;
import com.github.luben.zstd.ZstdException;

import static org.glavo.janex.reader.internal.Input.require;

/// Holds stored bytes and their BlobEncoding description.
/// @param bytes encoded content
/// @param encoding binary BlobEncoding
record StoredBlob(byte[] bytes, byte[] encoding) {
    /// Creates a stored representation without copying its arrays.
    StoredBlob { }

    /// Creates a caller-owned, single-threaded context, or null when compression is disabled.
    /// Disabled compression does not load the native library or validate the unused level.
    static ZstdCompressCtx compressor(PackOptions options) throws IOException {
        if (!options.compression) return null;
        try {
            int minimum = Zstd.minCompressionLevel();
            int maximum = Zstd.maxCompressionLevel();
            require(options.compressionLevel >= minimum && options.compressionLevel <= maximum,
                    "compressionLevel must be between " + minimum + " and " + maximum);
            ZstdCompressCtx context = new ZstdCompressCtx();
            try {
                return context.setLevel(options.compressionLevel).setWorkers(0).setChecksum(false).setContentSize(true);
            } catch (RuntimeException | Error failure) {
                context.close();
                throw failure;
            }
        } catch (ZstdException | LinkageError failure) {
            throw new IOException("Cannot initialize the native Zstandard compressor", failure);
        }
    }

    /// Chooses compression only when bytes and their enclosing description together shrink.
    /// The offset is relative to the pool payload; page descriptions use CBOR byte strings.
    /// The caller owns the reusable context; null disables compression.
    static StoredBlob encode(byte[] input, ZstdCompressCtx compressor, int offset, boolean page) throws IOException {
        StoredBlob raw = new StoredBlob(input, encoding(input.length, -1));
        if (compressor == null || input.length == 0) return raw;
        long capacity = Zstd.compressBound(input.length);
        if (capacity > Integer.MAX_VALUE) return raw;
        byte[] buffer = new byte[(int) capacity];
        int length;
        try {
            length = compressor.compressByteArray(buffer, 0, buffer.length, input, 0, input.length);
        } catch (ZstdException failure) {
            throw new IOException("Zstandard compression failed", failure);
        }
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

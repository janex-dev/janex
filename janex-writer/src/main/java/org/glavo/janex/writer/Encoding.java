// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.CharBuffer;
import java.nio.charset.StandardCharsets;
import java.util.*;

/// Encodes the binary and deterministic CBOR subset emitted by this writer.
final class Encoding extends ByteArrayOutputStream {
    /// Creates an empty encoder.
    Encoding() { }

    /// Writes a little-endian integer using the supplied number of bytes.
    void little(long value, int count) {
        for (int i = 0; i < count; i++) {
            write((int) (value >>> (i * 8)));
        }
    }

    /// Writes a nonnegative integer in shortest ULEB128 form.
    void uint(long value) {
        if (value < 0) throw new IllegalArgumentException("Negative unsigned integer");
        do {
            int next = (int) value & 127;
            value >>>= 7;
            write(next | (value == 0 ? 0 : 128));
        } while (value != 0);
    }

    /// Writes a byte-length prefix and its payload.
    void sized(byte[] bytes) {
        uint(bytes.length);
        writeBytes(bytes);
    }

    /// Writes one Sized CBOR map, using zero bytes for an empty map.
    void map(Map<?, ?> value) throws IOException {
        sized(value.isEmpty() ? new byte[0] : cbor(value));
    }

    /// Writes inline content with no transforms.
    void inline(byte[] bytes) {
        write(0);
        sized(bytes);
        write(0);
    }

    /// Writes a blob content reference with no transforms.
    void blob(long pool, long index) {
        write(1);
        uint(pool);
        uint(index);
        write(0);
    }

    /// Encodes well-formed Unicode without substituting malformed surrogates.
    static byte[] utf8(String text) throws IOException {
        var bytes = StandardCharsets.UTF_8.newEncoder().encode(CharBuffer.wrap(text));
        byte[] result = new byte[bytes.remaining()];
        bytes.get(result);
        return result;
    }

    /// Returns unsigned lexicographic UTF-8 order for validated strings.
    static int compare(String first, String second) {
        return Arrays.compareUnsigned(first.getBytes(StandardCharsets.UTF_8), second.getBytes(StandardCharsets.UTF_8));
    }

    /// Encodes one supported deterministic CBOR value.
    static byte[] cbor(Object value) throws IOException {
        Encoding output = new Encoding();
        output.value(value);
        return output.toByteArray();
    }

    /// Encodes definite maps, arrays, byte strings, text, booleans, null, and nonnegative integers.
    private void value(Object value) throws IOException {
        if (value == null) {
            write(0xf6);
        } else if (value instanceof Boolean flag) {
            write(flag ? 0xf5 : 0xf4);
        } else if (value instanceof Number number) {
            head(0, number.longValue());
        } else if (value instanceof byte[] bytes) {
            head(0x40, bytes.length);
            writeBytes(bytes);
        } else if (value instanceof String string) {
            byte[] bytes = utf8(string);
            head(0x60, bytes.length);
            writeBytes(bytes);
        } else if (value instanceof List<?> list) {
            head(0x80, list.size());
            for (Object item : list) value(item);
        } else if (value instanceof Map<?, ?> map) {
            SortedMap<byte[], Object> sorted = new TreeMap<>(Arrays::compareUnsigned);
            for (var entry : map.entrySet()) sorted.put(cbor(entry.getKey()), entry.getValue());
            head(0xa0, sorted.size());
            for (var entry : sorted.entrySet()) {
                writeBytes(entry.getKey());
                value(entry.getValue());
            }
        } else {
            throw new IOException("Unsupported CBOR value");
        }
    }

    /// Writes a shortest CBOR argument in network byte order.
    private void head(int major, long value) {
        if (value < 0) throw new IllegalArgumentException("Negative CBOR argument");
        if (value < 24) {
            write(major | (int) value);
        } else {
            int count = value <= 0xff ? 1 : value <= 0xffff ? 2 : value <= 0xffff_ffffL ? 4 : 8;
            write(major | (count == 1 ? 24 : count == 2 ? 25 : count == 4 ? 26 : 27));
            for (int i = count - 1; i >= 0; i--) write((int) (value >>> (i * 8)));
        }
    }
}

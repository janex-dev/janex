// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.IOException;
import java.math.BigInteger;
import java.nio.ByteBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.*;

/// Reads bounded binary fields and deterministic CBOR without interpreting application schemas.
final class Input {
    /// Maximum size of a buffered value.
    static final int MAX_BYTES = 512 * 1024 * 1024;
    /// Maximum number of collection elements.
    static final int MAX_ELEMENTS = 1_000_000;
    /// Owned or borrowed immutable input bytes.
    final byte[] bytes;
    /// Offset of the next unread byte.
    int position;

    /// Borrows bytes that must remain unchanged while this cursor is used.
    Input(byte[] bytes) throws IOException {
        size(bytes.length);
        this.bytes = bytes;
    }

    /// Rejects failed format constraints.
    static void require(boolean valid, String message) throws IOException {
        if (!valid) {
            throw new IOException(message);
        }
    }

    /// Checks a bounded nonnegative byte length.
    static int size(long value) throws IOException {
        require(value >= 0 && value <= MAX_BYTES, "Janex byte limit exceeded");
        return (int) value;
    }

    /// Checks a bounded collection length.
    static int count(long value) throws IOException {
        require(value >= 0 && value <= MAX_ELEMENTS, "Janex element limit exceeded");
        return (int) value;
    }

    /// Reads one unsigned byte.
    int u8() throws IOException {
        require(position < bytes.length, "Truncated Janex value");
        return bytes[position++] & 255;
    }

    /// Reads a little-endian integer, retaining all 64 bits.
    long little(int length) throws IOException {
        long result = 0;
        for (int i = 0; i < length; i++) {
            result |= (long) u8() << (i * 8);
        }
        return result;
    }

    /// Reads a ULEB128 value, retaining all unsigned bits.
    long uint() throws IOException {
        long result = 0;
        for (int i = 0; i < 10; i++) {
            int next = u8();
            require(i < 9 || next <= 1, "ULEB128 overflow");
            result |= (long) (next & 127) << (i * 7);
            if (next < 128) {
                return result;
            }
        }
        throw new IOException("ULEB128 overflow");
    }

    /// Copies an exact bounded byte range and advances the cursor.
    byte[] take(long length) throws IOException {
        int size = size(length);
        require(size <= bytes.length - position, "Truncated Janex value");
        byte[] result = Arrays.copyOfRange(bytes, position, position + size);
        position += size;
        return result;
    }

    /// Reads one byte-sized binary value.
    byte[] sized() throws IOException {
        return take(uint());
    }

    /// Reads a length-prefixed UTF-8 string without replacement characters.
    String string() throws IOException {
        return utf8(sized());
    }

    /// Decodes strict UTF-8.
    static String utf8(byte[] bytes) throws IOException {
        try {
            return StandardCharsets.UTF_8.newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(bytes)).toString();
        } catch (CharacterCodingException failure) {
            throw new IOException("Invalid UTF-8", failure);
        }
    }

    /// Reads exactly one sized CBOR map, including the zero-length empty-map encoding.
    Map<Object, Object> map() throws IOException {
        byte[] value = sized();
        if (value.length == 0) {
            return Collections.emptyMap();
        }
        Input input = new Input(value);
        Map<Object, Object> result = map(input.cbor(0));
        input.end();
        require(!result.isEmpty(), "Empty sized CBOR map must use zero length");
        return result;
    }

    /// Requires a CBOR map.
    @SuppressWarnings("unchecked")
    static Map<Object, Object> map(Object value) throws IOException {
        require(value instanceof Map, "Expected CBOR map");
        return (Map<Object, Object>) value;
    }

    /// Requires a CBOR array.
    @SuppressWarnings("unchecked")
    static List<Object> list(Object value) throws IOException {
        require(value instanceof List, "Expected CBOR array");
        return (List<Object>) value;
    }

    /// Requires a CBOR text string.
    static String text(Object value) throws IOException {
        require(value instanceof String, "Expected CBOR text");
        return (String) value;
    }

    /// Requires a CBOR byte string.
    static byte[] binary(Object value) throws IOException {
        require(value instanceof byte[], "Expected CBOR byte string");
        return (byte[]) value;
    }

    /// Requires a CBOR unsigned integer, retaining all unsigned bits.
    static long number(Object value) throws IOException {
        require(value instanceof BigInteger, "Expected CBOR unsigned integer");
        BigInteger number = (BigInteger) value;
        require(number.signum() >= 0 && number.bitLength() <= 64, "Invalid unsigned integer");
        return number.longValue();
    }

    /// Looks up an unsigned-integer map key.
    static Object get(Map<Object, Object> map, int key) {
        return map.get(BigInteger.valueOf(key));
    }

    /// Tests whether an unsigned-integer map key is present, including null values.
    static boolean has(Map<Object, Object> map, int key) {
        return map.containsKey(BigInteger.valueOf(key));
    }

    /// Requires unsigned-integer map keys.
    static Map<Object, Object> integers(Map<Object, Object> map) throws IOException {
        for (Object key : map.keySet()) {
            number(key);
        }
        return map;
    }

    /// Compares unsigned byte strings in lexicographic order.
    static int compare(byte[] left, byte[] right) {
        for (int i = 0; i < Math.min(left.length, right.length); i++) {
            int difference = (left[i] & 255) - (right[i] & 255);
            if (difference != 0) {
                return difference;
            }
        }
        return Integer.compare(left.length, right.length);
    }

    /// Reads a shortest-form CBOR argument.
    private BigInteger argument(int info) throws IOException {
        if (info < 24) {
            return BigInteger.valueOf(info);
        }
        require(info <= 27, "Indefinite or reserved CBOR encoding");
        int length = 1 << (info - 24);
        BigInteger result = new BigInteger(1, take(length));
        BigInteger minimum = info == 24 ? BigInteger.valueOf(24) : BigInteger.ONE.shiftLeft(length * 4);
        require(result.compareTo(minimum) >= 0, "Nonminimal CBOR argument");
        return result;
    }

    /// Validates and reads one deterministic CBOR item with bounded nesting.
    private Object cbor(int depth) throws IOException {
        require(depth <= 64, "CBOR nesting limit exceeded");
        int initial = u8();
        int major = initial >>> 5;
        int info = initial & 31;
        if (major == 7) {
            if (info == 20 || info == 21) {
                return info == 21;
            }
            if (info == 22) {
                return null;
            }
            // Unknown extension values are validated even when callers ignore them.
            if (info == 25) {
                int bits = (u8() << 8) | u8();
                require((bits & 0x7c00) != 0x7c00 || (bits & 1023) == 0 || bits == 0x7e00,
                        "Noncanonical CBOR NaN");
                return new Opaque(initial, bits);
            }
            if (info == 26 || info == 27) {
                byte[] raw = take(info == 26 ? 4 : 8);
                double value = info == 26 ? ByteBuffer.wrap(raw).getFloat() : ByteBuffer.wrap(raw).getDouble();
                require(!Double.isNaN(value), "Noncanonical CBOR NaN");
                if (info == 27) {
                    require((double) (float) value != value, "Nonminimal CBOR float");
                } else {
                    float absolute = Math.abs((float) value);
                    boolean half = Float.isInfinite(absolute) || absolute == 0;
                    if (!half && absolute <= 65504) {
                        double step = absolute < Math.scalb(1.0, -14) ? Math.scalb(1.0, -24)
                                : Math.scalb(1.0, Math.getExponent(absolute) - 10);
                        half = absolute / step == Math.rint(absolute / step);
                    }
                    require(!half, "Nonminimal CBOR float");
                }
                return new Opaque(initial, raw);
            }
            if (info == 24) {
                int simple = u8();
                require(simple >= 32, "Nonminimal CBOR simple value");
                return new Opaque(initial, simple);
            }
            require(info < 24, "Invalid CBOR simple value");
            return new Opaque(initial, info);
        }
        BigInteger argument = argument(info);
        if (major == 0) {
            return argument;
        }
        if (major == 1) {
            return argument.negate().subtract(BigInteger.ONE);
        }
        if (major == 6) {
            Object value = cbor(depth + 1);
            return new Opaque(argument, value);
        }
        require(argument.bitLength() <= 31, "CBOR length exceeds limit");
        int length = argument.intValue();
        if (major == 2 || major == 3) {
            byte[] raw = take(length);
            return major == 2 ? raw : utf8(raw);
        }
        count(length);
        if (major == 4) {
            List<Object> result = new ArrayList<Object>();
            for (int i = 0; i < length; i++) {
                result.add(cbor(depth + 1));
            }
            return result;
        }
        require(major == 5, "Invalid CBOR major type");
        Map<Object, Object> result = new LinkedHashMap<Object, Object>();
        byte[] previous = null;
        for (int i = 0; i < length; i++) {
            int start = position;
            Object key = cbor(depth + 1);
            byte[] encoded = Arrays.copyOfRange(bytes, start, position);
            require(previous == null || compare(previous, encoded) < 0, "CBOR keys are not unique and sorted");
            previous = encoded;
            result.put(key, cbor(depth + 1));
        }
        return result;
    }

    /// Retains an uninterpreted CBOR simple value or tag.
    static final class Opaque {
        /// Encoded type or tag.
        final Object type;
        /// Associated value.
        final Object value;

        /// Creates an uninterpreted extension value.
        Opaque(Object type, Object value) {
            this.type = type;
            this.value = value;
        }
    }

    /// Requires complete consumption of this byte boundary.
    void end() throws IOException {
        require(position == bytes.length, "Trailing Janex bytes");
    }
}

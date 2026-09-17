// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal;

import java.io.IOException;
import java.math.BigInteger;
import java.nio.ByteBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.time.DateTimeException;
import java.time.Instant;
import java.util.*;

import org.glavo.janex.reader.ReadLimits;

/// Reads bounded binary fields and deterministic CBOR without interpreting application schemas.
public final class Input {
    /// Nanoseconds per POSIX second for decoding integer resource timestamps.
    private static final BigInteger NANOS_PER_SECOND = BigInteger.valueOf(1_000_000_000);
    /// Maximum size of a buffered value.
    public static final int MAX_BYTES = ReadLimits.DEFAULT.maxBytes();
    /// Maximum number of collection elements.
    public static final int MAX_ELEMENTS = 1_000_000;
    /// Owned or borrowed immutable input bytes.
    private final byte[] bytes;
    /// Policy inherited by nested fields.
    private final ReadLimits limits;
    /// Offset of the next unread byte.
    private int position;
    /// Optional original byte spans for decoded maps, keyed by object identity.
    private final Map<Object, int[]> mapSpans;

    /// Identifies malformed values separately from source I/O failures and resource limits.
    public static final class Invalid extends IOException {
        /// Serialization identity for diagnostic transport.
        private static final long serialVersionUID = 1L;

        /// Creates a malformed-value diagnostic.
        public Invalid(String message) {
            super(message);
        }
    }

    /// Borrows bytes that must remain unchanged while this cursor is used.
    public Input(byte[] bytes) throws IOException {
        this(bytes, false);
    }

    /// Borrows input bytes and optionally records exact map encodings for metadata access.
    public Input(byte[] bytes, boolean recordMaps) throws IOException {
        this(bytes, recordMaps, ReadLimits.DEFAULT);
    }

    /// Borrows input bytes under the supplied policy.
    public Input(byte[] bytes, ReadLimits limits) throws IOException {
        this(bytes, false, limits);
    }

    /// Borrows bytes with optional map-span recording and inherited nested limits.
    public Input(byte[] bytes, boolean recordMaps, ReadLimits limits) throws IOException {
        this.limits = Objects.requireNonNull(limits);
        limits.bytes(bytes.length);
        this.bytes = bytes;
        mapSpans = recordMaps ? new IdentityHashMap<Object, int[]>() : null;
    }

    /// Returns the offset of the next unread byte.
    public int position() {
        return position;
    }

    /// Returns the number of unread bytes.
    public int remaining() {
        return bytes.length - position;
    }

    /// Returns the inherited immutable read policy.
    public ReadLimits limits() {
        return limits;
    }

    /// Returns an unsigned byte at an absolute array index without advancing this cursor.
    public int byteAt(int index) {
        return bytes[index] & 255;
    }

    /// Returns a copy of the consumed prefix.
    public byte[] prefix() {
        return Arrays.copyOf(bytes, position);
    }

    /// Creates a fresh cursor borrowing the same unchanged bytes and policy.
    public Input duplicate() throws IOException {
        return new Input(bytes, limits);
    }

    /// Skips an exact nonnegative byte count, leaving the position unchanged on failure.
    public void skip(long count) throws IOException {
        require(count >= 0 && count <= remaining(), "Truncated binary value");
        position += (int) count;
    }

    /// Rejects failed format constraints.
    public static void require(boolean valid, String message) throws IOException {
        if (!valid) {
            throw new Invalid(message);
        }
    }

    /// Checks a bounded nonnegative byte length.
    public static int size(long value) throws IOException {
        return ReadLimits.DEFAULT.bytes(value);
    }

    /// Checks a bounded collection length.
    public static int count(long value) throws IOException {
        return ReadLimits.DEFAULT.elements(value);
    }

    /// Reads one unsigned byte.
    public int u8() throws IOException {
        require(position < bytes.length, "Truncated Janex value");
        return bytes[position++] & 255;
    }

    /// Reads a little-endian integer, retaining all 64 bits.
    public long little(int length) throws IOException {
        long result = 0;
        for (int i = 0; i < length; i++) {
            result |= (long) u8() << (i * 8);
        }
        return result;
    }

    /// Reads a ULEB128 value, retaining all unsigned bits.
    public long uint() throws IOException {
        long result = 0;
        for (int i = 0; i < 10; i++) {
            int next = u8();
            require(i < 9 || next <= 1, "ULEB128 overflow");
            result |= (long) (next & 127) << (i * 7);
            if (next < 128) {
                return result;
            }
        }
        throw new Invalid("ULEB128 overflow");
    }

    /// Copies an exact bounded byte range and advances the cursor.
    public byte[] take(long length) throws IOException {
        int size = limits.bytes(length);
        require(size <= bytes.length - position, "Truncated Janex value");
        byte[] result = Arrays.copyOfRange(bytes, position, position + size);
        position += size;
        return result;
    }

    /// Reads one byte-sized binary value.
    public byte[] sized() throws IOException {
        return take(uint());
    }

    /// Reads a length-prefixed UTF-8 string without replacement characters.
    public String string() throws IOException {
        return utf8(sized());
    }

    /// Decodes strict UTF-8.
    public static String utf8(byte[] bytes) throws IOException {
        return utf8(ByteBuffer.wrap(bytes));
    }

    /// Decodes the remaining bytes as strict UTF-8, advancing past consumed bytes even on failure.
    public static String utf8(ByteBuffer bytes) throws IOException {
        try {
            return StandardCharsets.UTF_8.newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(bytes).toString();
        } catch (CharacterCodingException failure) {
            Invalid invalid = new Invalid("Invalid UTF-8");
            invalid.initCause(failure);
            throw invalid;
        }
    }

    /// Reads exactly one sized CBOR map, including the zero-length empty-map encoding.
    public Map<Object, Object> map() throws IOException {
        byte[] value = sized();
        if (value.length == 0) {
            return Collections.emptyMap();
        }
        Input input = new Input(value, limits);
        Map<Object, Object> result = map(input.cbor(0));
        input.end();
        require(!result.isEmpty(), "Empty sized CBOR map must use zero length");
        return result;
    }

    /// Copies the original CBOR bytes of a map decoded with span recording enabled.
    public byte[] encodedMap(Map<Object, Object> map) {
        int[] span = mapSpans.get(map);
        return Arrays.copyOfRange(bytes, span[0], span[1]);
    }

    /// Requires a CBOR map.
    @SuppressWarnings("unchecked")
    public static Map<Object, Object> map(Object value) throws IOException {
        require(value instanceof Map, "Expected CBOR map");
        return (Map<Object, Object>) value;
    }

    /// Requires a CBOR array.
    @SuppressWarnings("unchecked")
    public static List<Object> list(Object value) throws IOException {
        require(value instanceof List, "Expected CBOR array");
        return (List<Object>) value;
    }

    /// Requires a CBOR text string.
    public static String text(Object value) throws IOException {
        require(value instanceof String, "Expected CBOR text");
        return (String) value;
    }

    /// Requires a CBOR byte string.
    public static byte[] binary(Object value) throws IOException {
        require(value instanceof byte[], "Expected CBOR byte string");
        return (byte[]) value;
    }

    /// Requires a CBOR unsigned integer, retaining all unsigned bits.
    public static long number(Object value) throws IOException {
        if (value instanceof Long) {
            long number = (Long) value;
            require(number >= 0, "Invalid unsigned integer");
            return number;
        }
        require(value instanceof BigInteger, "Expected CBOR unsigned integer");
        BigInteger number = (BigInteger) value;
        require(number.signum() >= 0 && number.bitLength() <= 64, "Invalid unsigned integer");
        return number.longValue();
    }

    /// Looks up an unsigned-integer map key.
    public static Object get(Map<Object, Object> map, int key) {
        return map.get((long) key);
    }

    /// Tests whether an unsigned-integer map key is present, including null values.
    public static boolean has(Map<Object, Object> map, int key) {
        return map.containsKey((long) key);
    }

    /// Requires unsigned-integer map keys.
    public static Map<Object, Object> integers(Map<Object, Object> map) throws IOException {
        for (Object key : map.keySet()) {
            number(key);
        }
        return map;
    }

    /// Compares unsigned byte strings in lexicographic order.
    public static int compare(byte[] left, byte[] right) {
        for (int i = 0; i < Math.min(left.length, right.length); i++) {
            int difference = (left[i] & 255) - (right[i] & 255);
            if (difference != 0) {
                return difference;
            }
        }
        return Integer.compare(left.length, right.length);
    }

    /// Reads a shortest-form CBOR argument, retaining all unsigned 64 bits.
    private long argument(int info) throws IOException {
        if (info < 24) {
            return info;
        }
        require(info <= 27, "Indefinite or reserved CBOR encoding");
        int length = 1 << (info - 24);
        long result = 0;
        for (int i = 0; i < length; i++) {
            result = (result << 8) | u8();
        }
        long minimum = info == 24 ? 24 : 1L << (length * 4);
        require(Long.compareUnsigned(result, minimum) >= 0, "Nonminimal CBOR argument");
        return result;
    }

    /// Boxes an unsigned argument as Long, using BigInteger only above Long.MAX_VALUE.
    private static Object unsigned(long value) {
        if (value >= 0) {
            return value;
        }
        return BigInteger.valueOf(value & Long.MAX_VALUE).setBit(63);
    }

    /// Validates and reads one deterministic CBOR item with bounded nesting.
    /// Integers use Long when representable and BigInteger otherwise; tags retain their encoded payload.
    public Object cbor(int depth) throws IOException {
        limits.depth(depth);
        int begin = position;
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
        long argument = argument(info);
        if (major == 0) {
            return unsigned(argument);
        }
        if (major == 1) {
            if (argument >= 0) {
                return ~argument;
            }
            return ((BigInteger) unsigned(argument)).negate().subtract(BigInteger.ONE);
        }
        if (major == 6) {
            Object value = cbor(depth + 1);
            return new Opaque(unsigned(argument), value);
        }
        if (argument < 0 || argument > Integer.MAX_VALUE) {
            throw new IOException("CBOR length exceeds limit");
        }
        int length = (int) argument;
        if (major == 2 || major == 3) {
            byte[] raw = take(length);
            return major == 2 ? raw : utf8(raw);
        }
        limits.elements(length);
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
        if (mapSpans != null) {
            mapSpans.put(result, new int[]{begin, position});
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

    /// Decodes integer POSIX nanoseconds within the format's timestamp range.
    /// @param value a CBOR integer or canonical bignum
    /// @return the exact timestamp, retaining nanosecond precision
    /// @throws IOException if the encoding is invalid or the timestamp is out of range
    public static Instant timestamp(Object value) throws IOException {
        if (value instanceof Long) {
            long nanos = (Long) value;
            return Instant.ofEpochSecond(nanos / 1_000_000_000, nanos % 1_000_000_000);
        }
        if (value instanceof Opaque) {
            Opaque tag = (Opaque) value;
            require(Long.valueOf(2).equals(tag.type) || Long.valueOf(3).equals(tag.type), "Invalid timestamp tag");
            byte[] magnitude = binary(tag.value);
            require(magnitude.length > 8 && magnitude.length <= 16 && magnitude[0] != 0, "Invalid timestamp bignum");
            BigInteger integer = new BigInteger(1, magnitude);
            value = Long.valueOf(2).equals(tag.type) ? integer : integer.negate().subtract(BigInteger.ONE);
        }
        require(value instanceof BigInteger && ((BigInteger) value).bitLength() <= 127, "Invalid resource timestamp");
        BigInteger[] parts = ((BigInteger) value).divideAndRemainder(NANOS_PER_SECOND);
        try {
            return Instant.ofEpochSecond(parts[0].longValueExact(), parts[1].longValue());
        } catch (ArithmeticException | DateTimeException invalid) {
            throw new IOException("Resource timestamp out of range", invalid);
        }
    }

    /// Requires complete consumption of this byte boundary.
    public void end() throws IOException {
        require(position == bytes.length, "Trailing Janex bytes");
    }
}

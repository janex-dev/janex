// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap.internal.zstd;

import java.util.Arrays;
import java.util.Objects;
import static org.janex.bootstrap.internal.zstd.Input.require;

/// Decodes dictionary-free Zstandard frames into a caller-provided byte array.
///
/// Concatenated frames and interspersed skippable frames are supported. Each invocation
/// requires at least one ordinary frame, consumes the entire input range, and verifies
/// frame content sizes and checksums when present. Offset codes through 31 are supported.
/// Dictionaries are supplied by the Host through its separate decoding path.
///
/// Instances are unnecessary; calls have independent state. Callers must prevent concurrent
/// modification of input and output ranges. The implementation requires only Java 8 APIs.
public final class Zstandard {
    /// Maximum encoded and decoded block size permitted by the format.
    private static final int BLOCK_LIMIT = 128 * 1024;
    /// Additional-bit widths for literal-length codes 16 through 35.
    private static final int[] LITERAL_BITS = { 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16 };
    /// Additional-bit widths for match-length codes 32 through 52.
    private static final int[] MATCH_BITS = { 1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16 };
    /// Baselines generated from the literal-length code widths.
    private static final int[] LITERAL_BASE = bases(16, 16, LITERAL_BITS);
    /// Baselines generated from the match-length code widths.
    private static final int[] MATCH_BASE = bases(32, 35, MATCH_BITS);
    /// Predefined literal-length, offset, and match-length FSE tables.
    private static final Fse[] DEFAULTS = defaults();

    /// Prevents instantiation.
    private Zstandard() {}

    /// Decodes all frames in an input slice and returns the number of bytes written.
    ///
    /// Output capacity may exceed the decoded length. Bytes outside the output slice are
    /// not modified. On failure, already written output remains, but its length is unspecified;
    /// callers must discard it. Input and output arrays must be distinct, even for disjoint ranges.
    /// A failed call does not affect subsequent calls.
    ///
    /// @param input encoded bytes
    /// @param inputOffset first encoded byte
    /// @param inputLength number of encoded bytes
    /// @param output destination array
    /// @param outputOffset first writable byte
    /// @param outputLength maximum number of writable bytes
    /// @return number of decoded bytes written starting at {@code outputOffset}
    /// @throws NullPointerException if either array is null
    /// @throws IndexOutOfBoundsException if either range is outside its array
    /// @throws IllegalArgumentException if the arrays are identical, data is malformed,
    ///         a nonzero dictionary ID is present, or output capacity is insufficient
    public static int decompress(byte[] input, int inputOffset, int inputLength,
                                 byte[] output, int outputOffset, int outputLength) {
        range(input, inputOffset, inputLength);
        range(output, outputOffset, outputLength);
        if (input == output) throw new IllegalArgumentException("Input and output arrays must be distinct");
        Input cursor = new Input(input, inputOffset, inputOffset + inputLength);
        int position = outputOffset;
        int limit = outputOffset + outputLength;
        boolean frameSeen = false;
        while (cursor.remaining() > 0) {
            int magic = (int) cursor.little(4);
            if ((magic & 0xfffffff0) == 0x184d2a50) {
                long length = cursor.little(4);
                require(length <= cursor.remaining(), "truncated skippable frame");
                cursor.take((int) length);
            } else {
                require(magic == 0xfd2fb528, "unknown frame magic");
                frameSeen = true;
                position = new Frame(output, position, limit).decode(cursor);
            }
        }
        require(frameSeen, "no Zstandard frame");
        return position - outputOffset;
    }

    /// Checks an array range without overflowing offset arithmetic.
    private static void range(byte[] bytes, int offset, int length) {
        Objects.requireNonNull(bytes);
        if (offset < 0 || length < 0 || offset > bytes.length - length) throw new IndexOutOfBoundsException("Invalid array range");
    }

    /// Generates consecutive length intervals from their additional-bit widths.
    private static int[] bases(int first, int initial, int[] widths) {
        int[] values = new int[first + widths.length];
        for (int i = 0; i < first; i++) values[i] = i + (first == 32 ? 3 : 0);
        for (int i = 0; i < widths.length; i++) {
            values[first + i] = initial;
            initial += 1 << widths[i];
        }
        return values;
    }

    /// Builds the format's predefined distributions once for all decoder calls.
    private static Fse[] defaults() {
        int[] literals = new int[36];
        Arrays.fill(literals, 0, 13, 2);
        Arrays.fill(literals, 13, 16, 1);
        Arrays.fill(literals, 16, 27, 2);
        Arrays.fill(literals, 27, 32, 1);
        Arrays.fill(literals, 32, 36, -1);
        literals[0] = 4;
        literals[1] = literals[25] = 3;
        int[] offsets = new int[29];
        Arrays.fill(offsets, 0, 24, 1);
        Arrays.fill(offsets, 6, 9, 2);
        Arrays.fill(offsets, 24, 29, -1);
        int[] matches = new int[53];
        Arrays.fill(matches, 0, 46, 1);
        Arrays.fill(matches, 3, 9, 2);
        Arrays.fill(matches, 46, 53, -1);
        matches[1] = 4;
        matches[2] = 3;
        return new Fse[] { Fse.distribution(6, literals), Fse.distribution(5, offsets), Fse.distribution(6, matches) };
    }

    /// Holds output and entropy history for exactly one frame.
    private static final class Frame {
        /// Caller-owned output array.
        private final byte[] output;
        /// First decoded byte of this frame; matches cannot precede it.
        private final int start;
        /// Exclusive caller-provided output boundary.
        private final int limit;
        /// Next writable output position.
        private int position;
        /// Maximum backward match distance declared by the frame.
        private long window;
        /// Maximum encoded and regenerated size of each block.
        private int blockLimit;
        /// Most recently transmitted Huffman table, or null before its first definition.
        private PrefixTable prefixes;
        /// Previous literal-length, offset, and match-length tables, initially absent.
        private final Fse[] tables = new Fse[3];
        /// Most recent three match distances.
        private final int[] offsets = { 1, 4, 8 };

        /// Creates fresh entropy and match history at an output position.
        Frame(byte[] output, int position, int limit) {
            this.output = output;
            this.start = this.position = position;
            this.limit = limit;
        }

        /// Decodes the frame after its magic and returns the new output position.
        int decode(Input input) {
            int descriptor = input.octet();
            require((descriptor & 8) == 0, "reserved frame bit");
            boolean single = (descriptor & 32) != 0;
            if (!single) {
                int description = input.octet();
                long base = 1L << (10 + (description >>> 3));
                window = base + base / 8 * (description & 7);
            }
            int dictionaryWidth = descriptor & 3;
            if (dictionaryWidth == 3) dictionaryWidth = 4;
            require(input.little(dictionaryWidth) == 0, "external dictionary is required");
            int sizeFlag = descriptor >>> 6;
            int sizeWidth = sizeFlag == 0 ? (single ? 1 : 0) : 1 << sizeFlag;
            long contentSize = -1;
            if (sizeWidth != 0) {
                contentSize = input.little(sizeWidth);
                if (sizeWidth == 2) contentSize += 256;
                require(contentSize >= 0 && contentSize <= limit - position, "frame exceeds output capacity");
            }
            if (single) window = contentSize;
            blockLimit = (int) Math.min(window, BLOCK_LIMIT);
            boolean last;
            do {
                int header = (int) input.little(3);
                last = (header & 1) != 0;
                int type = (header >>> 1) & 3;
                int size = header >>> 3;
                require(size <= blockLimit, "encoded block exceeds limit");
                int before = position;
                if (type == 0) {
                    space(size);
                    Input raw = input.take(size);
                    System.arraycopy(raw.bytes, raw.position, output, position, size);
                    position += size;
                } else if (type == 1) {
                    space(size);
                    Arrays.fill(output, position, position + size, (byte) input.octet());
                    position += size;
                } else if (type == 2) compressed(input.take(size));
                else require(false, "reserved block type");
                require(position - before <= blockLimit, "decoded block exceeds limit");
                require(contentSize < 0 || position - start <= contentSize, "frame content size mismatch");
            } while (!last);
            require(contentSize < 0 || position - start == contentSize, "frame content size mismatch");
            if ((descriptor & 4) != 0) {
                require((int) input.little(4) == FrameChecksum.calculate(output, start, position), "frame checksum mismatch");
            }
            return position;
        }

        /// Checks remaining output capacity before writing a decoded range.
        private void space(int length) {
            require(length >= 0 && length <= limit - position, "decoded data exceeds output capacity");
        }

        /// Decodes a literals section into a block-bounded temporary array.
        private byte[] literals(Input input) {
            int first = input.octet();
            int type = first & 3;
            int format = (first >>> 2) & 3;
            int size;
            if (type < 2) {
                if ((format & 1) == 0) size = first >>> 3;
                else size = (first >>> 4) | ((int) input.little(format == 1 ? 1 : 2) << 4);
                require(size <= blockLimit, "literal size exceeds block limit");
                byte[] result = new byte[size];
                if (type == 0) {
                    Input raw = input.take(size);
                    System.arraycopy(raw.bytes, raw.position, result, 0, size);
                } else Arrays.fill(result, (byte) input.octet());
                return result;
            }
            int width = format < 2 ? 10 : (format == 2 ? 14 : 18);
            int extraBytes = format < 2 ? 2 : (format == 2 ? 3 : 4);
            long header = first | (input.little(extraBytes) << 8);
            size = (int) ((header >>> 4) & ((1 << width) - 1));
            int encodedSize = (int) (header >>> (4 + width));
            require(size <= blockLimit, "literal size exceeds block limit");
            Input encoded = input.take(encodedSize);
            if (type == 2) prefixes = PrefixTable.read(encoded);
            require(prefixes != null, "missing Huffman table");
            byte[] result = new byte[size];
            if (format == 0) prefixes.decode(encoded, result, 0, size);
            else {
                require(size >= 6, "four-stream literal size is too small");
                int[] lengths = { (int) encoded.little(2), (int) encoded.little(2), (int) encoded.little(2) };
                int stride = (size + 3) / 4;
                for (int i = 0; i < 4; i++) {
                    Input stream = encoded.take(i < 3 ? lengths[i] : encoded.remaining());
                    prefixes.decode(stream, result, i * stride, Math.min(size, (i + 1) * stride));
                }
            }
            encoded.finish();
            return result;
        }

        /// Selects or reads the next sequence table for one symbol alphabet.
        private void table(Input input, int index, int mode) {
            int maximumSymbol = index == 0 ? 35 : (index == 1 ? 31 : 52);
            if (mode == 0) tables[index] = DEFAULTS[index];
            else if (mode == 1) {
                int symbol = input.octet();
                require(symbol <= maximumSymbol, "sequence symbol exceeds alphabet");
                tables[index] = Fse.repeated(symbol);
            } else if (mode == 2) tables[index] = Fse.read(input, maximumSymbol, index == 1 ? 8 : 9);
            else require(tables[index] != null, "missing repeated FSE table");
        }

        /// Decodes sequences, copies their literals and matches, and appends remaining literals.
        private void compressed(Input input) {
            int blockStart = position;
            byte[] literals = literals(input);
            int count = input.octet();
            if (count == 255) count = (int) input.little(2) + 0x7f00;
            else if (count >= 128) count = ((count - 128) << 8) + input.octet();
            int used = 0;
            if (count > 0) {
                require(count <= blockLimit / 3, "sequence count exceeds block limit");
                int modes = input.octet();
                require((modes & 3) == 0, "reserved sequence bits");
                for (int i = 0; i < 3; i++) table(input, i, (modes >>> (6 - i * 2)) & 3);
                ReverseBits bits = new ReverseBits(input);
                int[] states = new int[3];
                for (int i = 0; i < 3; i++) states[i] = bits.read(tables[i].accuracy);
                for (int sequence = 0; sequence < count; sequence++) {
                    int literalCode = tables[0].rows[states[0]] & 255;
                    int offsetCode = tables[1].rows[states[1]] & 255;
                    int matchCode = tables[2].rows[states[2]] & 255;
                    long offsetValue = (1L << offsetCode) + bits.read(offsetCode);
                    int matchLength = MATCH_BASE[matchCode] + bits.read(matchCode < 32 ? 0 : MATCH_BITS[matchCode - 32]);
                    int literalLength = LITERAL_BASE[literalCode] + bits.read(literalCode < 16 ? 0 : LITERAL_BITS[literalCode - 16]);
                    require(literalLength <= literals.length - used, "sequence exceeds literals");
                    require((long) position - blockStart + literalLength + matchLength <= blockLimit, "sequence exceeds block limit");
                    space(literalLength + matchLength);
                    System.arraycopy(literals, used, output, position, literalLength);
                    position += literalLength;
                    used += literalLength;
                    int distance = distance(offsetValue, literalLength);
                    require(distance > 0 && distance <= window && distance <= position - start, "match exceeds frame history");
                    int matchStart = position;
                    int copied = Math.min(distance, matchLength);
                    System.arraycopy(output, position - distance, output, position, copied);
                    position += copied;
                    while (copied < matchLength) {
                        int length = Math.min(copied, matchLength - copied);
                        System.arraycopy(output, matchStart, output, position, length);
                        position += length;
                        copied += length;
                    }
                    if (sequence + 1 < count) {
                        states[0] = tables[0].next(states[0], bits);
                        states[2] = tables[2].next(states[2], bits);
                        states[1] = tables[1].next(states[1], bits);
                    }
                }
                bits.finish();
            }
            input.finish();
            int trailing = literals.length - used;
            require(position - blockStart + trailing <= blockLimit, "trailing literals exceed block limit");
            space(trailing);
            System.arraycopy(literals, used, output, position, trailing);
            position += trailing;
        }

        /// Resolves a match distance and updates the three-entry offset history.
        private int distance(long value, int literalLength) {
            long distance;
            int selected;
            if (value > 3) {
                distance = value - 3;
                selected = 2;
            } else {
                selected = (int) value - 1 + (literalLength == 0 ? 1 : 0);
                if (selected == 3) {
                    distance = offsets[0] - 1;
                    selected = 2;
                } else distance = offsets[selected];
            }
            require(distance > 0 && distance <= Integer.MAX_VALUE, "unsupported match distance");
            for (int i = selected; i > 0; i--) offsets[i] = offsets[i - 1];
            offsets[0] = (int) distance;
            return (int) distance;
        }
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader.internal.codec;

import java.io.ByteArrayOutputStream;
import java.util.Arrays;

import org.janex.reader.internal.codec.zstd.Zstandard;

/// Exercises format boundaries and array contracts with small, explicitly encoded frames.
public final class ZstandardTest {
    /// Prevents instantiation.
    private ZstandardTest() {
    }

    /// Converts unsigned byte literals to their array representation.
    private static byte[] bytes(int... values) {
        byte[] result = new byte[values.length];
        for (int i = 0; i < values.length; i++) {
            result[i] = (byte) values[i];
        }
        return result;
    }

    /// Concatenates complete byte sequences.
    private static byte[] join(byte[]... parts) {
        ByteArrayOutputStream result = new ByteArrayOutputStream();
        for (byte[] part : parts) {
            result.write(part, 0, part.length);
        }
        return result.toByteArray();
    }

    /// Wraps a payload with a block header; size is the regenerated size for RLE blocks.
    private static byte[] block(int type, boolean last, int size, byte[] payload) {
        int header = (size << 3) | (type << 1) | (last ? 1 : 0);
        return join(bytes(header, header >>> 8, header >>> 16), payload);
    }

    /// Wraps blocks in a dictionary-free frame with a 1024-byte window and unknown content size.
    private static byte[] frame(byte[]... blocks) {
        return join(bytes(0x28, 0xb5, 0x2f, 0xfd, 0, 0), join(blocks));
    }

    /// Produces a compressed block containing only raw literals and no sequences.
    private static byte[] literals(int header, byte[] text) {
        byte[] content = join(bytes(header), text, bytes(0));
        return block(2, true, content.length, content);
    }

    /// Builds a single-stream literal header and its encoded payload.
    private static byte[] huffman(int type, int size, byte[] payload) {
        int header = type | (size << 4) | (payload.length << 14);
        return join(bytes(header, header >>> 8, header >>> 16), payload, bytes(0));
    }

    /// Checks exact decoding and rejects every strict prefix of a single-frame test case.
    private static void valid(byte[] input, byte[] expected) {
        byte[] output = new byte[expected.length];
        int length = Zstandard.decompress(input, 0, input.length, output, 0, output.length);
        if (length != expected.length || !Arrays.equals(output, expected)) {
            throw new AssertionError("Incorrect fixture output");
        }
        for (int end = 0; end < input.length; end++) {
            invalid(Arrays.copyOf(input, end), output.length);
        }
    }

    /// Requires a format rejection, allowing no accidental array exceptions.
    private static void invalid(byte[] input, int capacity) {
        expect(IllegalArgumentException.class,
                () -> Zstandard.decompress(input, 0, input.length, new byte[capacity], 0, capacity));
    }

    /// Requires exactly the documented exception type for a failing operation.
    private static void expect(Class<? extends RuntimeException> type, Runnable operation) {
        try {
            operation.run();
        } catch (RuntimeException failure) {
            if (failure.getClass() == type) {
                return;
            }
            throw new AssertionError("Unexpected exception type", failure);
        }
        throw new AssertionError("Expected " + type.getSimpleName());
    }

    /// Runs array, frame, entropy-table, and history-boundary checks.
    ///
    /// @param args unused
    public static void main(String[] args) {
        byte[] raw = frame(block(0, true, 3, bytes(10, 20, 30)));
        valid(raw, bytes(10, 20, 30));
        valid(frame(block(1, true, 5, bytes(42))), bytes(42, 42, 42, 42, 42));
        valid(frame(literals(3 << 3, bytes(1, 2, 3))), bytes(1, 2, 3));
        valid(frame(literals((3 << 4) | 4, bytes(0, 1, 2, 3))), bytes(1, 2, 3));
        valid(frame(literals((3 << 4) | 12, bytes(0, 0, 1, 2, 3))), bytes(1, 2, 3));
        valid(frame(literals((3 << 3) | 1, bytes(42))), bytes(42, 42, 42));

        byte[] tree = huffman(2, 3, bytes(0x81, 0x11, 0x23));
        byte[] reuse = huffman(3, 3, bytes(0x23));
        valid(frame(block(2, true, tree.length, tree)), bytes(0, 1, 2));
        byte[] fourStreams = huffman(6, 8, bytes(128, 0x10, 1, 0, 1, 0, 1, 0, 5, 5, 5, 5));
        valid(frame(block(2, true, fourStreams.length, fourStreams)), bytes(0, 1, 0, 1, 0, 1, 0, 1));
        byte[] emptyFourth = huffman(6, 6, bytes(128, 0x10, 1, 0, 1, 0, 1, 0, 5, 5, 5, 1));
        valid(frame(block(2, true, emptyFourth.length, emptyFourth)), bytes(0, 1, 0, 1, 0, 1));
        byte[] badJump = huffman(6, 8, bytes(128, 0x10, 255, 255, 1, 0, 1, 0, 5, 5, 5, 5));
        invalid(frame(block(2, true, badJump.length, badJump)), 8);
        valid(frame(block(2, false, tree.length, tree), block(0, false, 1, bytes(9)),
                block(2, true, reuse.length, reuse)), bytes(0, 1, 2, 9, 0, 1, 2));
        invalid(frame(block(2, true, reuse.length, reuse)), 3);
        invalid(join(frame(block(2, true, tree.length, tree)), frame(block(2, true, reuse.length, reuse))), 6);

        // RLE tables encode literal length 1, offset code 0, and match length 3.
        byte[] sequence = bytes(8, 42, 1, 0x54, 1, 0, 0, 1);
        byte[] repeat = bytes(8, 43, 1, 0xfc, 1);
        valid(frame(block(2, true, sequence.length, sequence)), bytes(42, 42, 42, 42));
        valid(frame(block(2, false, sequence.length, sequence), block(0, false, 1, bytes(44)),
                block(2, true, repeat.length, repeat)), bytes(42, 42, 42, 42, 44, 43, 43, 43, 43));
        invalid(frame(block(2, true, repeat.length, repeat)), 4);
        invalid(join(frame(block(2, true, sequence.length, sequence)), frame(block(2, true, repeat.length, repeat))), 8);
        byte[] missingHistory = bytes(0, 1, 0x54, 0, 0, 0, 1);
        invalid(join(raw, frame(block(2, true, missingHistory.length, missingHistory))), 6);

        byte[] invalidCode = sequence.clone();
        invalidCode[4] = 36;
        invalid(frame(block(2, true, invalidCode.length, invalidCode)), 4);
        invalidCode = sequence.clone();
        invalidCode[5] = 32;
        invalid(frame(block(2, true, invalidCode.length, invalidCode)), 4);
        invalidCode = sequence.clone();
        invalidCode[6] = 53;
        invalid(frame(block(2, true, invalidCode.length, invalidCode)), 4);
        invalidCode = sequence.clone();
        invalidCode[3] |= 1;
        invalid(frame(block(2, true, invalidCode.length, invalidCode)), 4);
        invalid(frame(block(3, true, 0, bytes())), 0);
        invalid(frame(block(0, true, 1025, new byte[1025])), 1025);
        invalid(frame(block(2, true, 3, bytes(0, 0, 1))), 0);
        invalid(frame(block(2, true, 3, bytes(0, 1, 0xa8))), 0);
        invalid(frame(block(2, true, 6, bytes(0, 1, 0xa8, 15, 0, 1))), 0);
        invalid(frame(block(2, true, 7, huffman(2, 1, bytes(128, 0xf0, 1)))), 1);
        invalid(raw, 2);
        invalid(join(raw, bytes(0)), 3);

        for (int tag = 0x50; tag <= 0x5f; tag++) {
            byte[] skipped = bytes(tag, 0x2a, 0x4d, 0x18, 2, 0, 0, 0, 42, 43);
            valid(join(skipped, raw), bytes(10, 20, 30));
            invalid(skipped, 0);
        }
        byte[] magic = bytes(0x28, 0xb5, 0x2f, 0xfd);
        byte[] rawBlock = block(0, true, 3, bytes(10, 20, 30));
        valid(join(magic, bytes(0x23, 0, 0, 0, 0, 3), rawBlock), bytes(10, 20, 30));
        invalid(join(magic, bytes(0x21, 1, 3), rawBlock), 3);
        invalid(join(magic, bytes(0x20, 2), rawBlock), 3);
        invalid(join(magic, bytes(0x20, 4), rawBlock), 4);
        valid(join(magic, bytes(0xa0, 3, 0, 0, 0), rawBlock), bytes(10, 20, 30));
        valid(join(magic, bytes(0x10, 0), rawBlock), bytes(10, 20, 30));
        invalid(join(magic, bytes(8, 0), rawBlock), 3);

        byte[] target = new byte[4];
        expect(NullPointerException.class, () -> Zstandard.decompress(null, 0, 0, target, 0, 0));
        expect(NullPointerException.class, () -> Zstandard.decompress(raw, 0, raw.length, null, 0, 0));
        expect(IndexOutOfBoundsException.class, () -> Zstandard.decompress(raw, -1, 1, target, 0, 4));
        expect(IndexOutOfBoundsException.class, () -> Zstandard.decompress(raw, 1, Integer.MAX_VALUE, target, 0, 4));
        expect(IndexOutOfBoundsException.class, () -> Zstandard.decompress(raw, 0, raw.length, target, 2, 3));
        expect(IllegalArgumentException.class, () -> Zstandard.decompress(raw, 0, 1, raw, 1, 1));
        expect(NullPointerException.class, () -> Zstandard.decompress(raw, 0, raw.length, target, 0, 4, null));
        expect(IllegalArgumentException.class, () -> Zstandard.decompress(raw, 0, raw.length, target, 0, 4, target));

        // With no literals, repeat-offset code zero selects the second initial offset, four.
        byte[] dictionary = bytes(10, 20, 30, 40, 50, 60, 70, 80);
        byte[] dictionaryFrame = frame(block(2, true, missingHistory.length, missingHistory));
        byte[] dictionaryOutput = new byte[3];
        if (Zstandard.decompress(dictionaryFrame, 0, dictionaryFrame.length,
                dictionaryOutput, 0, 3, dictionary) != 3 || !Arrays.equals(dictionaryOutput, bytes(50, 60, 70))) {
            throw new AssertionError("Raw dictionary history mismatch");
        }
        valid(raw, bytes(10, 20, 30));
        System.out.println("Verified Zstandard format boundaries and array contracts");
    }
}

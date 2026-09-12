// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap.internal.zstd;

import java.util.Arrays;

import static org.janex.bootstrap.internal.zstd.Input.require;

/// Decodes Huffman literals using a complete table of at most 2048 prefixes.
final class PrefixTable {
    /// Maximum code length in bits.
    private final int depth;
    /// Prefix entries packed as a symbol followed by its code length.
    private final int[] entries;

    /// Completes the implicit final weight and assigns prefixes in weight and symbol order.
    private PrefixTable(int[] weights, int count) {
        int sum = 0;
        for (int i = 0; i < count; i++) {
            require(weights[i] <= 11, "Huffman weight exceeds limit");
            if (weights[i] != 0) {
                sum += 1 << (weights[i] - 1);
            }
        }
        require(sum > 0, "empty Huffman alphabet");
        depth = 32 - Integer.numberOfLeadingZeros(sum);
        require(depth <= 11, "Huffman depth exceeds limit");
        int missing = (1 << depth) - sum;
        require((missing & (missing - 1)) == 0, "invalid final Huffman weight");
        weights[count++] = 32 - Integer.numberOfLeadingZeros(missing);
        entries = new int[1 << depth];
        int position = 0;
        int smallest = 0;
        for (int weight = 1; weight <= depth; weight++) {
            for (int symbol = 0; symbol < count; symbol++) {
                if (weights[symbol] != weight) {
                    continue;
                }
                if (weight == 1) {
                    smallest++;
                }
                int width = 1 << (weight - 1);
                Arrays.fill(entries, position, position + width, symbol | ((depth + 1 - weight) << 8));
                position += width;
            }
        }
        require(smallest >= 2 && (smallest & 1) == 0, "invalid shortest Huffman weights");
        require(position == entries.length, "incomplete Huffman tree");
    }

    /// Reads either directly encoded or FSE-compressed weights from a literals section.
    static PrefixTable read(Input input) {
        int header = input.octet();
        int[] weights = new int[256];
        int count;
        if (header >= 128) {
            count = header - 127;
            for (int i = 0; i < count; i += 2) {
                int pair = input.octet();
                weights[i] = pair >>> 4;
                if (i + 1 < count) {
                    weights[i + 1] = pair & 15;
                }
            }
        } else {
            Input compressed = input.take(header);
            Fse table = Fse.read(compressed, 11, 6);
            ReverseBits bits = new ReverseBits(compressed);
            int[] states = {bits.read(table.accuracy), bits.read(table.accuracy)};
            int turn = 0;
            count = 0;
            while (true) {
                require(count < 255, "too many Huffman weights");
                int row = table.rows[states[turn]];
                weights[count++] = row & 255;
                int consumed = (row >>> 8) & 255;
                if (consumed > bits.remaining) {
                    require(count < 255, "too many Huffman weights");
                    weights[count++] = table.rows[states[turn ^ 1]] & 255;
                    break;
                }
                states[turn] = table.next(states[turn], bits);
                turn ^= 1;
            }
        }
        return new PrefixTable(weights, count);
    }

    /// Decodes exactly the given output range and requires complete bitstream consumption.
    void decode(Input input, byte[] output, int start, int end) {
        ReverseBits bits = new ReverseBits(input);
        for (int i = start; i < end; i++) {
            int entry = entries[bits.peek(depth)];
            bits.read(entry >>> 8);
            output[i] = (byte) entry;
        }
        bits.finish();
    }
}

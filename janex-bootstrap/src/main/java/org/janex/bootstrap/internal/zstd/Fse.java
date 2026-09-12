// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap.internal.zstd;

import static org.janex.bootstrap.internal.zstd.Input.require;

/// Stores immutable FSE decoding transitions derived from normalized symbol frequencies.
final class Fse {
    /// Number of bits used to initialize a state.
    final int accuracy;
    /// State rows packed as symbol, bit count, and transition base, from low bits to high bits.
    final int[] rows;

    /// Creates a table with the supplied accuracy and state count.
    private Fse(int accuracy) {
        this.accuracy = accuracy;
        rows = new int[1 << accuracy];
    }

    /// Creates a zero-bit table that always decodes the same symbol.
    static Fse repeated(int symbol) {
        Fse table = new Fse(0);
        table.rows[0] = symbol;
        return table;
    }

    /// Constructs a table from a complete, validated normalized distribution.
    static Fse distribution(int accuracy, int[] frequencies) {
        Fse table = new Fse(accuracy);
        int size = table.rows.length;
        int tail = size - 1;
        int[] occurrences = new int[frequencies.length];
        for (int symbol = 0; symbol < frequencies.length; symbol++) {
            int frequency = frequencies[symbol];
            occurrences[symbol] = Math.abs(frequency);
            if (frequency == -1) table.rows[tail--] = symbol;
        }
        int position = 0;
        int stride = size / 2 + size / 8 + 3;
        for (int symbol = 0; symbol < frequencies.length; symbol++) {
            for (int n = 0; n < frequencies[symbol]; n++) {
                table.rows[position] = symbol;
                do { position = (position + stride) & (size - 1); } while (position > tail);
            }
        }
        require(position == 0, "incomplete FSE distribution");
        for (int state = 0; state < size; state++) {
            int symbol = table.rows[state];
            int ordinal = occurrences[symbol]++;
            int bits = accuracy - (31 - Integer.numberOfLeadingZeros(ordinal));
            int base = (ordinal << bits) - size;
            table.rows[state] = symbol | (bits << 8) | (base << 16);
        }
        return table;
    }

    /// Reads a normalized distribution, enforcing the alphabet and accuracy limits.
    static Fse read(Input input, int maximumSymbol, int maximumAccuracy) {
        ForwardBits bits = new ForwardBits(input);
        int accuracy = bits.read(4) + 5;
        require(accuracy <= maximumAccuracy, "FSE accuracy exceeds limit");
        int[] frequencies = new int[maximumSymbol + 1];
        int remaining = 1 << accuracy;
        int symbol = 0;
        int present = 0;
        while (remaining > 0) {
            require(symbol <= maximumSymbol, "FSE symbol exceeds alphabet");
            int maximum = remaining + 1;
            int width = 32 - Integer.numberOfLeadingZeros(maximum);
            int shortValues = (1 << width) - 1 - maximum;
            int value = bits.read(width - 1);
            if (value >= shortValues) {
                value += bits.read(1) << (width - 1);
                if (value >= 1 << (width - 1)) value -= shortValues;
            }
            int frequency = value - 1;
            require(Math.abs(frequency) <= remaining, "FSE probability exceeds total");
            frequencies[symbol++] = frequency;
            remaining -= Math.abs(frequency);
            if (frequency != 0) present++;
            else {
                int run;
                do {
                    run = bits.read(2);
                    symbol += run;
                    require(symbol <= maximumSymbol + 1, "FSE zero run exceeds alphabet");
                } while (run == 3);
            }
        }
        require(present >= 2, "FSE requires two symbols");
        return distribution(accuracy, frequencies);
    }

    /// Updates a state by consuming the number of bits recorded in its row.
    int next(int state, ReverseBits bits) {
        int row = rows[state];
        return (row >>> 16) + bits.read((row >>> 8) & 255);
    }

    /// Reads little-endian fields forward, consuming only bytes containing a field.
    private static final class ForwardBits {
        /// Cursor advanced as bytes enter the reservoir.
        private final Input input;
        /// Low bits contain the unread reservoir contents.
        private int reservoir;
        /// Number of unread bits in the reservoir.
        private int available;

        /// Creates an empty reservoir over a section cursor.
        ForwardBits(Input input) { this.input = input; }

        /// Reads a field of at most ten bits, rejecting truncation.
        int read(int count) {
            while (available < count) {
                reservoir |= input.octet() << available;
                available += 8;
            }
            int value = reservoir & ((1 << count) - 1);
            reservoir >>>= count;
            available -= count;
            return value;
        }
    }
}

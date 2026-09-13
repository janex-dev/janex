// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader;

import java.io.*;
import java.util.Arrays;
import java.util.Map;

import org.janex.reader.internal.Input;

/// Compares deterministic CBOR, sized-map framing, and ULEB128 with native results.
public final class CborTest {
    /// Prevents instantiation.
    private CborTest() {
    }

    /// Checks shared valid and malformed encodings, including exact unknown map bytes.
    public static void main(String[] arguments) throws Exception {
        try (DataInputStream input = new DataInputStream(new BufferedInputStream(new FileInputStream(arguments[0])))) {
            int count = input.readInt();
            for (int index = 0; index < count; index++) {
                int mode = input.readUnsignedByte();
                byte[] encoded = new byte[input.readInt()];
                input.readFully(encoded);
                boolean expected = input.readBoolean();
                long result = input.readLong();
                boolean accepted = false;
                IOException failure = null;
                try {
                    Input cursor = new Input(encoded, true);
                    Object value = null;
                    long actual;
                    if (mode == 2) {
                        actual = cursor.uint();
                    } else {
                        value = mode == 1 ? cursor.map() : cursor.cbor(0);
                        actual = value instanceof Map ? ((Map<?, ?>) value).size() : -1;
                    }
                    cursor.end();
                    accepted = true;
                    if (expected && actual != result) {
                        throw new AssertionError("Decoded CBOR result mismatch at " + index);
                    }
                    if (mode == 0 && value instanceof Map && !Arrays.equals(encoded, cursor.encodedMap(Input.map(value)))) {
                        throw new AssertionError("Original map bytes changed at " + index);
                    }
                } catch (IOException invalid) {
                    failure = invalid;
                }
                if (accepted != expected) {
                    throw new AssertionError("CBOR vector " + index + " mode " + mode + ": " + Arrays.toString(encoded), failure);
                }
            }
            if (input.read() != -1) {
                throw new AssertionError("Trailing CBOR vectors");
            }
        }
    }
}

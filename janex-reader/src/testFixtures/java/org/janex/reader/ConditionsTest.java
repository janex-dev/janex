// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader;

import java.io.*;
import java.nio.charset.StandardCharsets;

import org.janex.reader.internal.Conditions;
import org.janex.reader.internal.Input;

/// Compares version parsing, VERS selection, and condition validation with native results.
public final class ConditionsTest {
    /// Prevents instantiation.
    private ConditionsTest() {
    }

    /// Evaluates trusted vectors in an isolated JVM with explicitly supplied runtime properties.
    public static void main(String[] arguments) throws Exception {
        try (DataInputStream input = new DataInputStream(new BufferedInputStream(new FileInputStream(arguments[0])))) {
            int count = input.readInt();
            for (int index = 0; index < count; index++) {
                int mode = input.readUnsignedByte();
                byte[] encoded = bytes(input);
                String candidate = text(input);
                int expected = input.readInt();
                int actual = -1;
                IOException failure = null;
                String[] properties = new String[4];
                if (mode == 2) {
                    for (int property = 0; property < properties.length; property++) {
                        properties[property] = text(input);
                    }
                }
                try {
                    if (mode == 0) {
                        System.setProperty("java.version", candidate);
                        actual = Conditions.feature();
                    } else if (mode == 1) {
                        actual = Conditions.range(new String(encoded, StandardCharsets.UTF_8), candidate) ? 1 : 0;
                    } else {
                        Input cursor = new Input(encoded);
                        java.util.Map<Object, Object> condition = Input.map(cursor.cbor(0));
                        cursor.end();
                        Conditions.validate(condition);
                        System.setProperty("os.name", properties[0]);
                        System.setProperty("os.arch", properties[1]);
                        System.setProperty("java.version", properties[2]);
                        System.setProperty("java.vendor", properties[3]);
                        actual = Conditions.matches(condition) ? 1 : 0;
                    }
                } catch (IOException invalid) {
                    failure = invalid;
                }
                if (actual != expected) {
                    throw new AssertionError("Condition vector " + index + " mode " + mode + " candidate " + candidate
                            + " value " + new String(encoded, StandardCharsets.UTF_8) + ": expected " + expected + ", got " + actual, failure);
                }
            }
            if (input.read() != -1) {
                throw new AssertionError("Trailing condition vectors");
            }
        }
    }

    /// Reads a trusted length-prefixed byte sequence.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] value = new byte[input.readInt()];
        input.readFully(value);
        return value;
    }

    /// Reads trusted UTF-8 text.
    private static String text(DataInputStream input) throws IOException {
        return new String(bytes(input), StandardCharsets.UTF_8);
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal.codec;

import java.io.*;
import java.util.Arrays;

import org.glavo.janex.reader.ClassFile;
import org.glavo.janex.reader.ReadLimits;

/// Compares ordinary validation and CLASSFILE restoration with native format results.
public final class ClassFileTest {
    /// Prevents instantiation.
    private ClassFileTest() {
    }

    /// Checks valid classes, malformed structures, exact restoration, and inherited limits.
    public static void main(String[] arguments) throws Exception {
        try (DataInputStream input = new DataInputStream(new BufferedInputStream(new FileInputStream(arguments[0])))) {
            int count = input.readInt();
            for (int index = 0; index < count; index++) {
                boolean transform = input.readBoolean();
                ReadLimits limits = new ReadLimits(input.readInt(), input.readInt(), input.readInt());
                byte[] encoded = bytes(input);
                int length = input.readInt();
                byte[][] pool = new byte[input.readInt()][];
                for (int item = 0; item < pool.length; item++) {
                    pool[item] = bytes(input);
                }
                boolean valid = input.readBoolean();
                byte[] expected = bytes(input);
                boolean accepted = false;
                IOException failure = null;
                try {
                    if (transform) {
                        byte[] restored = ClassFile.restore(encoded, pool, length, limits);
                        if (valid && !Arrays.equals(expected, restored)) {
                            throw new AssertionError("Restored class bytes differ at " + index);
                        }
                    } else {
                        ClassFile.validate(encoded, limits);
                    }
                    accepted = true;
                } catch (IOException invalid) {
                    failure = invalid;
                }
                if (accepted != valid) {
                    throw new AssertionError("Class vector " + index + " transform " + transform
                            + " limits " + limits.maxBytes() + "/" + limits.maxElements() + ": expected " + valid, failure);
                }
            }
            if (input.read() != -1) {
                throw new AssertionError("Trailing class vectors");
            }
        }
    }

    /// Reads one trusted harness byte sequence.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] value = new byte[input.readInt()];
        input.readFully(value);
        return value;
    }
}

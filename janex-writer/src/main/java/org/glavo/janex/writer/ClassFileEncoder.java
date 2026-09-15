// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.charset.CharacterCodingException;
import java.util.Arrays;

import org.glavo.janex.reader.ClassFile;
import org.glavo.janex.reader.ReadLimits;
import org.glavo.janex.reader.internal.Input;

/// Encodes the two lossless external constant-pool string forms in Janex 0.1.
final class ClassFileEncoder {
    /// Prevents instantiation.
    private ClassFileEncoder() { }

    /// Returns transformed bytes or null for malformed or unprofitable class files.
    /// Retains original entries for strings that cannot round-trip through the UTF-8 pool.
    /// Rejected transformations leave the pool unchanged; allocation-limit failures propagate.
    static byte[] transform(byte[] bytes, StringPool strings, ReadLimits limits) throws IOException {
        try {
            ClassFile.validate(bytes, limits);
        } catch (Input.Invalid invalid) {
            return null;
        }
        DataInputStream input = new DataInputStream(new ByteArrayInputStream(bytes));
        input.skipNBytes(8);
        int count = input.readUnsignedShort();
        int[] starts = new int[count];
        int[] ends = new int[count];
        boolean[] classes = new boolean[count];
        for (int i = 1; i < count; i++) {
            starts[i] = bytes.length - input.available();
            int tag = input.readUnsignedByte();
            int size = switch (tag) {
                case 1 -> input.readUnsignedShort();
                case 3, 4, 9, 10, 11, 12, 17, 18 -> 4;
                case 5, 6 -> 8;
                case 7 -> { classes[input.readUnsignedShort()] = true; yield 0; }
                case 8, 16, 19, 20 -> 2;
                case 15 -> 3;
                default -> throw new IOException("Unknown constant-pool tag");
            };
            input.skipNBytes(size);
            ends[i] = bytes.length - input.available();
            if (tag == 5 || tag == 6) i++;
        }
        int body = bytes.length - input.available();
        int checkpoint = strings.size();
        boolean accepted = false;
        try {
            Encoding output = new Encoding();
            output.write(bytes, 0, 10);
            for (int i = 1; i < count; i++) {
                if (starts[i] == 0) continue;
                byte[] replacement = null;
                if (bytes[starts[i]] == 1) {
                    DataInputStream utf = new DataInputStream(new ByteArrayInputStream(bytes, starts[i] + 1,
                            ends[i] - starts[i] - 1));
                    String text = utf.readUTF();
                    int before = strings.size();
                    try {
                        Encoding.utf8(text);
                        ByteArrayOutputStream modified = new ByteArrayOutputStream();
                        new DataOutputStream(modified).writeUTF(text);
                        if (Arrays.equals(modified.toByteArray(), Arrays.copyOfRange(bytes, starts[i] + 1, ends[i]))) {
                            Encoding entry = new Encoding();
                            if (classes[i] && !text.startsWith("[") && !text.isEmpty() && !text.endsWith("/")) {
                                int slash = text.lastIndexOf('/');
                                entry.write(0xfe);
                                entry.uint(strings.intern(slash < 0 ? "" : text.substring(0, slash)));
                                entry.uint(strings.intern(text.substring(slash + 1)));
                            } else {
                                entry.write(0xff);
                                entry.uint(strings.intern(text));
                            }
                            if (entry.size() < ends[i] - starts[i]) replacement = entry.toByteArray();
                        }
                    } catch (CharacterCodingException ignored) {
                        // Unpaired surrogates retain their original Modified UTF-8 entry.
                    }
                    if (replacement == null) strings.truncate(before);
                }
                if (replacement == null) output.write(bytes, starts[i], ends[i] - starts[i]);
                else output.writeBytes(replacement);
            }
            output.write(bytes, body, bytes.length - body);
            Encoding descriptor = new Encoding();
            descriptor.uint(bytes.length);
            if (output.size() + descriptor.size() + 2 >= bytes.length) return null;
            byte[] result = output.toByteArray();
            result[2] = (byte) 0xca;
            result[3] = 0x70;
            accepted = true;
            return result;
        } finally {
            if (!accepted) strings.truncate(checkpoint);
        }
    }
}

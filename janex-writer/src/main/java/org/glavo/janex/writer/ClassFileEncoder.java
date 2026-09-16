// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.ByteArrayInputStream;
import java.io.DataInputStream;
import java.io.EOFException;
import java.io.IOException;
import java.util.Arrays;

import org.glavo.janex.reader.ReadLimits;
import org.glavo.janex.reader.internal.Input;

/// Encodes lossless external constant-pool strings and byte templates in Janex 0.1.
final class ClassFileEncoder {
    /// Prevents instantiation.
    private ClassFileEncoder() { }

    /// Returns transformed bytes or null for unsupported framing or unprofitable class files.
    /// Only constant-pool framing is parsed; class-file internals are not validated.
    /// Retains original entries whose UTF-8 and Modified UTF-8 bytes differ.
    /// Rejected transformations leave the pool unchanged; allocation-limit failures propagate.
    static byte[] transform(byte[] bytes, DataPool strings, ReadLimits limits) throws IOException {
        limits.bytes(bytes.length);
        if (bytes.length < 10) return null;
        DataInputStream input = new DataInputStream(new ByteArrayInputStream(bytes));
        if (input.readInt() != 0xcafebabe) return null;
        input.skipNBytes(4);
        int count = input.readUnsignedShort();
        if (count == 0) return null;
        limits.elements(count);
        int[] starts = new int[count];
        int[] ends = new int[count];
        boolean[] classes = new boolean[count];
        try {
            for (int i = 1; i < count; i++) {
                starts[i] = bytes.length - input.available();
                int tag = input.readUnsignedByte();
                int size = switch (tag) {
                    case 1 -> input.readUnsignedShort();
                    case 3, 4, 9, 10, 11, 12, 17, 18 -> 4;
                    case 5, 6 -> 8;
                    case 7 -> {
                        int reference = input.readUnsignedShort();
                        if (reference < count) classes[reference] = true;
                        yield 0;
                    }
                    case 8, 16, 19, 20 -> 2;
                    case 15 -> 3;
                    default -> -1;
                };
                if (size < 0) return null;
                input.skipNBytes(size);
                ends[i] = bytes.length - input.available();
                if ((tag == 5 || tag == 6) && ++i >= count) return null;
            }
        } catch (EOFException truncated) {
            return null;
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
                    byte[] raw = Arrays.copyOfRange(bytes, starts[i] + 3, ends[i]);
                    int before = strings.size();
                    try {
                        String text = Input.utf8(raw);
                        if (sharedEncoding(text)) {
                            Encoding entry = new Encoding();
                            if (classes[i] && !text.startsWith("[") && !text.startsWith("/")
                                    && !text.isEmpty() && !text.endsWith("/")) {
                                int slash = text.lastIndexOf('/');
                                entry.write(0xfe);
                                entry.uint(strings.intern(Encoding.utf8(slash < 0 ? "" : text.substring(0, slash))));
                                entry.uint(strings.intern(Encoding.utf8(text.substring(slash + 1))));
                            } else {
                                byte[] template = template(raw, strings);
                                entry.write(template == null ? 0xff : 0xfd);
                                entry.uint(strings.intern(template == null ? raw : template));
                            }
                            if (entry.size() < ends[i] - starts[i]) replacement = entry.toByteArray();
                        }
                    } catch (Input.Invalid ignored) {
                        // Entries not representable as UTF-8 retain their original bytes.
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

    /// Extracts byte-exact class-name fragments without validating the Java type grammar.
    private static byte[] template(byte[] bytes, DataPool pool) throws IOException {
        if (bytes.length == 0 || "([L<".indexOf(bytes[0]) < 0) return null;
        Encoding output = new Encoding();
        boolean changed = false;
        int position = 0;
        while (position < bytes.length) {
            int value = bytes[position++] & 255;
            output.write(value);
            if (value != 'L') continue;
            int start = position;
            int end = start;
            while (end < bytes.length && ";<.:>[()".indexOf(bytes[end] & 255) < 0) end++;
            if (end == start || end == bytes.length || ";<.".indexOf(bytes[end] & 255) < 0
                    || bytes[start] == '/' || bytes[end - 1] == '/') {
                output.write(bytes, start, end - start);
                position = end;
                continue;
            }
            int slash = end - 1;
            while (slash >= start && bytes[slash] != '/') slash--;
            int checkpoint = pool.size();
            Encoding reference = new Encoding();
            reference.write(0);
            reference.uint(pool.intern(Arrays.copyOfRange(bytes, start, slash < start ? start : slash)));
            reference.uint(pool.intern(Arrays.copyOfRange(bytes, slash < start ? start : slash + 1, end)));
            if (reference.size() < end - start) {
                output.writeBytes(reference.toByteArray());
                position = end;
                changed = true;
            } else {
                pool.truncate(checkpoint);
                output.write(bytes, start, end - start);
                position = end;
            }
        }
        return changed ? output.toByteArray() : null;
    }

    /// Tests whether UTF-8 and Modified UTF-8 use the same bytes for this decoded text.
    private static boolean sharedEncoding(String text) {
        for (int i = 0; i < text.length(); i++) {
            char ch = text.charAt(i);
            if (ch == 0 || Character.isSurrogate(ch)) return false;
        }
        return true;
    }
}

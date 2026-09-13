// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.*;
import org.janex.format.ClassFile;
import org.janex.format.ReadLimits;

/// Restores the two external UTF-8 constant-pool entry forms defined by Janex 0.1.
final class ClassFiles {
    /// Prevents instantiation.
    private ClassFiles() {
    }

    /// Restores exact class bytes within the declared size; JVM verification checks bytecode.
    ///
    /// @param bytes  transformed class bytes
    /// @param pool   selected root or file override pool
    /// @param length required original class length
    /// @return a new ordinary class file
    /// @throws IOException if framing, references, or output length are invalid
    static byte[] restore(byte[] bytes, String[] pool, int length) throws IOException {
        return restore(bytes, pool, length, ReadLimits.DEFAULT);
    }

    /// Restores and structurally validates class bytes under the inherited resource limits.
    static byte[] restore(byte[] bytes, String[] pool, int length, ReadLimits limits) throws IOException {
        if (length < 0 || length > limits.maxBytes() || bytes.length > limits.maxBytes()) {
            throw new IOException("CLASSFILE byte limit exceeded");
        }
        DataInputStream input = new DataInputStream(new ByteArrayInputStream(bytes));
        byte[] result = new byte[length];
        ByteArrayOutput output = new ByteArrayOutput(result);
        DataOutputStream data = new DataOutputStream(output);
        if (input.readInt() != 0xcafeca70) {
            throw new IOException("Invalid transformed class magic");
        }
        data.writeInt(0xcafebabe);
        data.writeInt(input.readInt());
        int count = input.readUnsignedShort();
        if (count == 0 || count > limits.maxElements()) {
            throw new IOException("Invalid constant pool count");
        }
        data.writeShort(count);
        for (int i = 1; i < count; i++) {
            int tag = input.readUnsignedByte();
            if (tag == 0xff || tag == 0xfe) {
                String text = string(input, pool);
                if (tag == 0xfe) {
                    String name = string(input, pool);
                    if (name.isEmpty()) {
                        throw new IOException("Empty external class name");
                    }
                    if ((long) text.length() + name.length() + (text.isEmpty() ? 0 : 1) > 65535) {
                        throw new IOException("External class string exceeds 65535 bytes");
                    }
                    text = text.isEmpty() ? name : text + '/' + name;
                }
                data.writeByte(1);
                data.writeUTF(text);
            } else {
                data.writeByte(tag);
                int size;
                switch (tag) {
                    case 1:
                        size = input.readUnsignedShort();
                        data.writeShort(size);
                        break;
                    case 3:
                    case 4:
                    case 9:
                    case 10:
                    case 11:
                    case 12:
                    case 17:
                    case 18:
                        size = 4;
                        break;
                    case 5:
                    case 6:
                        if (++i >= count) {
                            throw new IOException("Missing constant pool reserved slot");
                        }
                        size = 8;
                        break;
                    case 7:
                    case 8:
                    case 16:
                    case 19:
                    case 20:
                        size = 2;
                        break;
                    case 15:
                        size = 3;
                        break;
                    default:
                        throw new IOException("Unknown constant pool tag");
                }
                for (int j = 0; j < size; j++) {
                    data.writeByte(input.readUnsignedByte());
                }
            }
        }
        while (input.available() != 0) {
            data.writeByte(input.readUnsignedByte());
        }
        if (output.position != result.length) {
            throw new IOException("CLASSFILE decoded size mismatch");
        }
        ClassFile.validate(result, limits);
        return result;
    }

    /// Reads a bounded ULEB128 string index, accepting zero padding permitted by the format.
    private static String string(DataInputStream input, String[] pool) throws IOException {
        long value = 0;
        for (int shift = 0; shift < 70; shift += 7) {
            int next = input.readUnsignedByte();
            if (shift == 63 && next > 1) {
                throw new IOException("String index overflow");
            }
            value |= (long) (next & 127) << shift;
            if (next < 128) {
                if (value < 0 || value >= pool.length) {
                    throw new IOException("String index out of range");
                }
                String text = pool[(int) value];
                if (text.length() > 65535) {
                    throw new IOException("External class string exceeds 65535 bytes");
                }
                return text;
            }
        }
        throw new IOException("String index overflow");
    }

    /// Writes into an exact-size output without growing or exposing partial results.
    private static final class ByteArrayOutput extends OutputStream {
        /// Caller-owned result buffer.
        private final byte[] bytes;
        /// Number of bytes written.
        int position;

        /// Binds an empty output to its result buffer.
        ByteArrayOutput(byte[] bytes) {
            this.bytes = bytes;
        }

        /// Writes one byte or rejects output exceeding the declared size.
        @Override
        public void write(int value) throws IOException {
            if (position == bytes.length) {
                throw new IOException("CLASSFILE exceeds declared size");
            }
            bytes[position++] = (byte) value;
        }
    }
}

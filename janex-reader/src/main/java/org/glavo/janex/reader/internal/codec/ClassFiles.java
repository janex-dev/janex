// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal.codec;

import java.io.*;

import org.glavo.janex.reader.ClassFile;
import org.glavo.janex.reader.ReadLimits;

/// Restores external CONSTANT_Utf8 entries by copying pooled Modified UTF-8 bytes.
public final class ClassFiles {
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
    public static byte[] restore(byte[] bytes, byte[][] pool, int length) throws IOException {
        return restore(bytes, pool, length, ReadLimits.DEFAULT);
    }

    /// Restores and structurally validates class bytes under the inherited resource limits.
    public static byte[] restore(byte[] bytes, byte[][] pool, int length, ReadLimits limits) throws IOException {
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
                byte[] text = entry(input, pool);
                if (tag == 0xfe) {
                    byte[] name = entry(input, pool);
                    if (name.length == 0) {
                        throw new IOException("Empty external class name");
                    }
                    if ((long) text.length + name.length + (text.length == 0 ? 0 : 1) > 65535) {
                        throw new IOException("External class string exceeds 65535 bytes");
                    }
                    data.writeByte(1);
                    data.writeShort(text.length + name.length + (text.length == 0 ? 0 : 1));
                    if (text.length != 0) {
                        data.write(text);
                        data.writeByte('/');
                    }
                    data.write(name);
                } else {
                    data.writeByte(1);
                    data.writeShort(text.length);
                    data.write(text);
                }
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

    /// Reads a bounded ULEB128 data index, accepting zero padding permitted by the format.
    private static byte[] entry(DataInputStream input, byte[][] pool) throws IOException {
        long value = 0;
        for (int shift = 0; shift < 70; shift += 7) {
            int next = input.readUnsignedByte();
            if (shift == 63 && next > 1) {
                throw new IOException("Data-pool index overflow");
            }
            value |= (long) (next & 127) << shift;
            if (next < 128) {
                if (value < 0 || value >= pool.length) {
                    throw new IOException("Data-pool index out of range");
                }
                byte[] text = pool[(int) value];
                if (text.length > 65535) {
                    throw new IOException("External class string exceeds 65535 bytes");
                }
                return text;
            }
        }
        throw new IOException("Data-pool index overflow");
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

        /// Copies one byte range, rejecting overflow before modifying the result.
        @Override
        public void write(byte[] source, int offset, int length) throws IOException {
            if (offset < 0 || length < 0 || offset > source.length - length) {
                throw new IndexOutOfBoundsException();
            }
            if (length > bytes.length - position) {
                throw new IOException("CLASSFILE exceeds declared size");
            }
            System.arraycopy(source, offset, bytes, position, length);
            position += length;
        }
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal.codec;

import java.io.*;

import org.glavo.janex.reader.DataPool;

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
    public static byte[] restore(byte[] bytes, DataPool pool, int length) throws IOException {
        return restore(bytes, pool, length, ReadLimits.DEFAULT);
    }

    /// Restores bytes with bounded transform framing; class-file validation belongs to the JVM.
    public static byte[] restore(byte[] bytes, DataPool pool, int length, ReadLimits limits) throws IOException {
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
            if (tag == 0xfd) {
                int template = entry(input, pool, true);
                if (pool.byteLength(template) > limits.maxBytes()) {
                    throw new IOException("CLASSFILE byte limit exceeded");
                }
                data.writeByte(1);
                int lengthPosition = output.position;
                data.writeShort(0);
                expandTemplate(pool.cursor(template), pool, output);
                int textLength = output.position - lengthPosition - 2;
                result[lengthPosition] = (byte) (textLength >>> 8);
                result[lengthPosition + 1] = (byte) textLength;
            } else if (tag == 0xff || tag == 0xfe) {
                int text = entry(input, pool);
                int textLength = pool.byteLength(text);
                if (tag == 0xfe) {
                    int name = entry(input, pool);
                    int nameLength = pool.byteLength(name);
                    if (nameLength == 0) {
                        throw new IOException("Empty external class name");
                    }
                    if ((long) textLength + nameLength + (textLength == 0 ? 0 : 1) > 65535) {
                        throw new IOException("External class string exceeds 65535 bytes");
                    }
                    data.writeByte(1);
                    data.writeShort(textLength + nameLength + (textLength == 0 ? 0 : 1));
                    if (textLength != 0) {
                        output.write(pool, text);
                        data.writeByte('/');
                    }
                    output.write(pool, name);
                } else {
                    data.writeByte(1);
                    data.writeShort(textLength);
                    output.write(pool, text);
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
                copy(input, bytes, output, size);
            }
        }
        copy(input, bytes, output, input.available());
        if (output.position != result.length) {
            throw new IOException("CLASSFILE decoded size mismatch");
        }
        return result;
    }

    /// Expands references directly into the result without allocating a restored string.
    private static void expandTemplate(DataPool.Cursor template, DataPool pool, ByteArrayOutput output) throws IOException {
        int start = output.position;
        while (template.remaining() != 0) {
            int next = template.readUnsignedByte();
            if (next != 0) {
                checkTemplateLength((long) output.position - start + 1);
                output.write(next);
                continue;
            }
            int packageName = entry(template, pool);
            int className = entry(template, pool);
            int packageLength = pool.byteLength(packageName);
            int classLength = pool.byteLength(className);
            if (classLength == 0) throw new IOException("Empty external class name");
            checkTemplateLength((long) output.position - start + packageLength
                    + classLength + (packageLength == 0 ? 0 : 1));
            if (packageLength != 0) {
                output.write(pool, packageName);
                output.write('/');
            }
            output.write(pool, className);
        }
    }

    /// Rejects a template expansion that cannot fit a CONSTANT_Utf8 byte length.
    private static void checkTemplateLength(long length) throws IOException {
        if (length > 65535) throw new IOException("External class string exceeds 65535 bytes");
    }

    /// Copies a framed input range without interpreting unchanged class-file bytes.
    private static void copy(DataInputStream input, byte[] bytes, ByteArrayOutput output, int length) throws IOException {
        int remaining = input.available();
        if (length > remaining) {
            throw new EOFException("Truncated constant-pool entry");
        }
        output.write(bytes, bytes.length - remaining, length);
        input.skipBytes(length);
    }

    /// Reads an external string index with the CONSTANT_Utf8 byte-length bound.
    private static int entry(DataInputStream input, DataPool pool) throws IOException {
        return entry(input, pool, false);
    }

    /// Reads an index, allowing templates to exceed the restored string length bound.
    private static int entry(DataInputStream input, DataPool pool, boolean template) throws IOException {
        long value = 0;
        for (int shift = 0; shift < 70; shift += 7) {
            int next = input.readUnsignedByte();
            if (shift == 63 && next > 1) throw new IOException("Data-pool index overflow");
            value |= (long) (next & 127) << shift;
            if (next < 128) return checkedEntry(value, pool, template);
        }
        throw new IOException("Data-pool index overflow");
    }

    /// Reads a nonrecursive reference from a template, advancing past consumed bytes on failure.
    private static int entry(DataPool.Cursor input, DataPool pool) throws IOException {
        long value = 0;
        for (int shift = 0; shift < 70; shift += 7) {
            int next = input.readUnsignedByte();
            if (shift == 63 && next > 1) throw new IOException("Data-pool index overflow");
            value |= (long) (next & 127) << shift;
            if (next < 128) return checkedEntry(value, pool, false);
        }
        throw new IOException("Data-pool index overflow");
    }

    /// Checks an unsigned index and the length bound for directly copied string entries.
    private static int checkedEntry(long value, DataPool pool, boolean template) throws IOException {
        if (value < 0 || value >= pool.size()) throw new IOException("Data-pool index out of range");
        int index = (int) value;
        if (!template && pool.byteLength(index) > 65535) {
            throw new IOException("External class string exceeds 65535 bytes");
        }
        return index;
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

        /// Copies a complete pool entry directly into the final class buffer.
        void write(DataPool pool, int index) throws IOException {
            int length = pool.byteLength(index);
            if (length > bytes.length - position) throw new IOException("CLASSFILE exceeds declared size");
            pool.copyTo(index, bytes, position);
            position += length;
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

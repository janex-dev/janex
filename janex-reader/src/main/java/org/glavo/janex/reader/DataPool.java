// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.DataInput;
import java.io.DataOutput;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Set;

import org.glavo.janex.reader.internal.Input;

import static org.glavo.janex.reader.internal.Input.require;

/// An immutable, indexed collection of opaque bytes with an empty entry at index zero.
/// Entries share a contiguous buffer. Views remain valid independently of the reader that
/// created the pool, and concurrent reads do not require synchronization.
public final class DataPool {
    /// Owned entry payloads; unused capacity may follow the last entry.
    private final byte[] bytes;
    /// Entry boundaries, including the final payload end.
    private final int[] offsets;

    /// Takes exclusive ownership of validated storage and offsets.
    private DataPool(byte[] bytes, int[] offsets) {
        this.bytes = bytes;
        this.offsets = offsets;
    }

    /// Decodes exactly one format data pool without retaining the input array.
    /// @param encoded nonnull encoded bytes, unchanged during this call
    /// @param limits nonnull byte and element limits
    /// @return an independently owned immutable pool
    /// @throws IOException if framing, uniqueness, index zero, or limits are invalid
    public static DataPool decode(byte[] encoded, ReadLimits limits) throws IOException {
        Input input = new Input(encoded, limits);
        int count = limits.elements(input.uint());
        require(count > 0 && count < Integer.MAX_VALUE && count <= input.remaining(), "Invalid data pool count");
        byte[] bytes = new byte[encoded.length];
        int[] offsets = new int[count + 1];
        Set<ByteBuffer> unique = new HashSet<ByteBuffer>();
        for (int i = 0; i < count; i++) {
            int length = limits.bytes(input.uint());
            int start = input.position();
            input.skip(length);
            require(i != 0 || length == 0, "Data pool must start with empty bytes");
            require(unique.add(ByteBuffer.wrap(encoded, start, length)), "Duplicate data-pool entry");
            System.arraycopy(encoded, start, bytes, offsets[i], length);
            offsets[i + 1] = offsets[i] + length;
        }
        input.end();
        return new DataPool(bytes, offsets);
    }

    /// Copies an array of distinct entries under the default read limits.
    /// @param entries nonnull array of nonnull bytes, unchanged during this call
    /// @return a pool independent of the array and its entries
    /// @throws IOException if entries are duplicate, index zero is invalid, or limits are exceeded
    public static DataPool copyOf(byte[][] entries) throws IOException {
        int count = ReadLimits.DEFAULT.elements(entries.length);
        require(count > 0 && count < Integer.MAX_VALUE && entries[0].length == 0, "Invalid data pool");
        int[] offsets = new int[count + 1];
        Set<ByteBuffer> unique = new HashSet<ByteBuffer>();
        for (int i = 0; i < count; i++) {
            offsets[i + 1] = ReadLimits.DEFAULT.bytes((long) offsets[i] + entries[i].length);
            require(unique.add(ByteBuffer.wrap(entries[i])), "Duplicate data-pool entry");
        }
        byte[] bytes = new byte[offsets[count]];
        for (int i = 0; i < count; i++) System.arraycopy(entries[i], 0, bytes, offsets[i], entries[i].length);
        return new DataPool(bytes, offsets);
    }

    /// Reads a pool from an already validated private bootstrap index.
    /// The encoding is a big-endian signed int count followed by count pairs of nonnegative
    /// int byte lengths and payloads. Uniqueness is the index producer's responsibility.
    /// @param input nonnull input, consumed through this pool and not closed
    /// @param limits nonnull limits on entry count and total payload bytes
    /// @return an independently owned pool
    /// @throws IOException if I/O fails, framing or index zero is invalid, or limits are exceeded;
    /// consumed input is not rolled back
    public static DataPool readIndex(DataInput input, ReadLimits limits) throws IOException {
        int count = limits.elements(input.readInt());
        require(count > 0 && count < Integer.MAX_VALUE, "Invalid data pool count");
        int[] offsets = new int[count + 1];
        byte[] bytes = new byte[0];
        byte[] scratch = null;
        for (int i = 0; i < count; i++) {
            int length = limits.bytes(input.readInt());
            require(i != 0 || length == 0, "Data pool must start with empty bytes");
            int end = limits.bytes((long) offsets[i] + length);
            if (end > bytes.length) {
                int capacity = (int) Math.min(limits.maxBytes(), Math.max(end, Math.max(1024L, bytes.length * 2L)));
                bytes = Arrays.copyOf(bytes, capacity);
            }
            if (length != 0 && scratch == null) scratch = new byte[Math.min(8192, limits.maxBytes())];
            for (int copied = 0; copied < length;) {
                int chunk = Math.min(length - copied, scratch.length);
                input.readFully(scratch, 0, chunk);
                System.arraycopy(scratch, 0, bytes, offsets[i] + copied, chunk);
                copied += chunk;
            }
            offsets[i + 1] = end;
        }
        return new DataPool(bytes, offsets);
    }

    /// Writes the private-index encoding accepted by [#readIndex(DataInput, ReadLimits)].
    /// @param output nonnull output, neither flushed nor closed; it may retain supplied buffers
    /// @throws IOException if writing fails; previously written bytes are retained
    public void writeIndex(DataOutput output) throws IOException {
        output.writeInt(size());
        byte[] scratch = new byte[Math.min(8192, offsets[size()])];
        for (int i = 0; i < size(); i++) {
            int length = offsets[i + 1] - offsets[i];
            output.writeInt(length);
            for (int copied = 0; copied < length;) {
                int count = Math.min(length - copied, scratch.length);
                System.arraycopy(bytes, offsets[i] + copied, scratch, 0, count);
                output.write(scratch, 0, count);
                copied += count;
            }
        }
    }

    /// Returns the number of entries, including index zero.
    /// @return a positive entry count
    public int size() {
        return offsets.length - 1;
    }

    /// Returns an entry's byte length.
    /// @param index zero-based entry index
    /// @return the entry length, possibly zero
    /// @throws IOException if the index is negative or outside this pool
    public int byteLength(int index) throws IOException {
        require(index >= 0 && index < size(), "Invalid data-pool index");
        return offsets[index + 1] - offsets[index];
    }

    /// Returns an independent read-only view with position zero and limit equal to the entry length.
    /// @param index zero-based entry index
    /// @return a view sharing immutable storage, valid for its lifetime
    /// @throws IOException if the index is negative or outside this pool
    public ByteBuffer view(int index) throws IOException {
        int length = byteLength(index);
        return ByteBuffer.wrap(bytes, offsets[index], length).slice().asReadOnlyBuffer();
    }

    /// Copies a complete entry into caller-owned storage without changing this pool.
    /// @param index zero-based entry index
    /// @param target nonnull destination array
    /// @param offset destination start offset in bytes
    /// @throws IOException if the entry index is invalid
    /// @throws IndexOutOfBoundsException if the complete entry does not fit; no bytes are copied
    public void copyTo(int index, byte[] target, int offset) throws IOException {
        int length = byteLength(index);
        System.arraycopy(bytes, offsets[index], target, offset, length);
    }
}

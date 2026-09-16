// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.DataInput;
import java.io.DataOutput;
import java.io.EOFException;
import java.io.IOException;
import java.nio.ByteBuffer;

import org.glavo.janex.reader.internal.Input;

import static org.glavo.janex.reader.internal.Input.require;

/// An immutable, indexed collection of opaque bytes with an empty entry at index zero.
/// Entries share a contiguous buffer. Views remain valid independently of the reader that
/// created the pool, and concurrent reads do not require synchronization.
/// Entries retain their supplied indices without checking uniqueness.
public final class DataPool {
    /// Owned entry payloads in index order.
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
    /// @throws IOException if framing, index zero, or limits are invalid
    public static DataPool decode(byte[] encoded, ReadLimits limits) throws IOException {
        Input input = new Input(encoded, limits);
        int count = limits.elements(input.uint());
        require(count > 0 && count < Integer.MAX_VALUE && count <= input.remaining(), "Invalid data pool count");
        byte[] bytes = new byte[encoded.length];
        int[] offsets = new int[count + 1];
        for (int i = 0; i < count; i++) {
            int length = limits.bytes(input.uint());
            int start = input.position();
            input.skip(length);
            require(i != 0 || length == 0, "Data pool must start with empty bytes");
            System.arraycopy(encoded, start, bytes, offsets[i], length);
            offsets[i + 1] = offsets[i] + length;
        }
        input.end();
        return new DataPool(bytes, offsets);
    }

    /// Copies an array of entries under the default read limits.
    /// @param entries nonnull array of nonnull bytes, unchanged during this call
    /// @return a pool independent of the array and its entries
    /// @throws IOException if index zero is invalid or limits are exceeded
    public static DataPool copyOf(byte[][] entries) throws IOException {
        int count = ReadLimits.DEFAULT.elements(entries.length);
        require(count > 0 && count < Integer.MAX_VALUE && entries[0].length == 0, "Invalid data pool");
        int[] offsets = new int[count + 1];
        for (int i = 0; i < count; i++) {
            offsets[i + 1] = ReadLimits.DEFAULT.bytes((long) offsets[i] + entries[i].length);
        }
        byte[] bytes = new byte[offsets[count]];
        for (int i = 0; i < count; i++) System.arraycopy(entries[i], 0, bytes, offsets[i], entries[i].length);
        return new DataPool(bytes, offsets);
    }

    /// Reads a pool from an already validated private bootstrap index.
    /// The encoding contains a big-endian nonnegative int entry count, total payload length,
    /// one int length per entry, and the concatenated payloads. Uniqueness is the producer's responsibility.
    /// @param input nonnull input, consumed through this pool and not closed
    /// @param limits nonnull limits on entry count and total payload bytes
    /// @return an independently owned pool
    /// @throws IOException if I/O fails, framing or index zero is invalid, or limits are exceeded;
    /// consumed input is not rolled back
    public static DataPool readIndex(DataInput input, ReadLimits limits) throws IOException {
        int count = limits.elements(input.readInt());
        require(count > 0 && count < Integer.MAX_VALUE, "Invalid data pool count");
        int total = limits.bytes(input.readInt());
        int[] offsets = new int[count + 1];
        for (int i = 0; i < count; i++) {
            int length = limits.bytes(input.readInt());
            require(i != 0 || length == 0, "Data pool must start with empty bytes");
            require(length <= total - offsets[i], "Data pool length exceeds payload");
            offsets[i + 1] = offsets[i] + length;
        }
        require(offsets[count] == total, "Data pool payload length mismatch");
        byte[] bytes = new byte[total];
        byte[] scratch = new byte[Math.min(8192, total)];
        for (int copied = 0; copied < total;) {
            int chunk = Math.min(total - copied, scratch.length);
            input.readFully(scratch, 0, chunk);
            System.arraycopy(scratch, 0, bytes, copied, chunk);
            copied += chunk;
        }
        return new DataPool(bytes, offsets);
    }

    /// Writes the private-index encoding accepted by [#readIndex(DataInput, ReadLimits)].
    /// @param output nonnull output, neither flushed nor closed; it may retain supplied buffers
    /// @throws IOException if writing fails; previously written bytes are retained
    public void writeIndex(DataOutput output) throws IOException {
        output.writeInt(size());
        int total = offsets[size()];
        output.writeInt(total);
        for (int i = 0; i < size(); i++) {
            output.writeInt(offsets[i + 1] - offsets[i]);
        }
        byte[] scratch = new byte[Math.min(8192, total)];
        for (int copied = 0; copied < total;) {
            int count = Math.min(total - copied, scratch.length);
            System.arraycopy(bytes, copied, scratch, 0, count);
            output.write(scratch, 0, count);
            copied += count;
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

    /// Creates an independent forward cursor over one immutable entry without copying its bytes.
    /// @param index zero-based entry index
    /// @return a cursor positioned at the start, valid independently of the originating reader
    /// @throws IOException if the index is negative or outside this pool
    public Cursor cursor(int index) throws IOException {
        int length = byteLength(index);
        return new Cursor(bytes, offsets[index], length);
    }

    /// A forward reader over immutable pool bytes; each cursor has independent mutable state.
    /// A cursor must not be accessed concurrently without external synchronization.
    public static final class Cursor {
        /// Shared immutable payload storage.
        private final byte[] bytes;
        /// Exclusive end of this entry in the shared storage.
        private final int end;
        /// Offset of the next byte in the shared storage.
        private int position;

        /// Binds a validated range without exposing the shared array.
        private Cursor(byte[] bytes, int offset, int length) {
            this.bytes = bytes;
            position = offset;
            end = offset + length;
        }

        /// Returns the number of unread bytes, or zero when exhausted.
        public int remaining() {
            return end - position;
        }

        /// Reads an unsigned byte and advances by one.
        /// @return a value from zero through 255
        /// @throws EOFException if exhausted; the position remains unchanged
        public int readUnsignedByte() throws EOFException {
            if (position == end) {
                throw new EOFException("Data-pool entry exhausted");
            }
            return bytes[position++] & 255;
        }
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

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap.internal.zstd;

/// Provides checked, little-endian byte-array access to the vendored decoder.
final class MemoryAccess {
    /// Array offsets are ordinary zero-based indices.
    static final long ARRAY_BYTE_BASE_OFFSET = 0;
    /// Prevents instantiation.
    private MemoryAccess() {}
    /// Returns a checked array index.
    private static int index(long offset) { return Math.toIntExact(offset); }
    /// Reads one byte.
    static byte getByte(Object array, long offset) { return ((byte[]) array)[index(offset)]; }
    /// Reads an unsigned byte.
    static int getUnsignedByte(Object array, long offset) { return getByte(array, offset) & 255; }
    /// Writes one byte.
    static void putByte(Object array, long offset, byte value) { ((byte[]) array)[index(offset)] = value; }
    /// Reads a little-endian short.
    static short getShort(Object array, long offset) { return (short) (getUnsignedByte(array, offset) | getUnsignedByte(array, offset + 1) << 8); }
    /// Reads a little-endian integer.
    static int getInt(Object array, long offset) { return (getShort(array, offset) & 65535) | getShort(array, offset + 2) << 16; }
    /// Reads an unsigned little-endian integer.
    static long getUnsignedInt(Object array, long offset) { return getInt(array, offset) & 0xffffffffL; }
    /// Reads a little-endian long.
    static long getLong(Object array, long offset) { return getUnsignedInt(array, offset) | (long) getInt(array, offset + 4) << 32; }
    /// Writes a little-endian short.
    static void putShort(Object array, long offset, short value) { putByte(array, offset, (byte) value); putByte(array, offset + 1, (byte) (value >>> 8)); }
    /// Writes a little-endian integer.
    static void putInt(Object array, long offset, int value) { putShort(array, offset, (short) value); putShort(array, offset + 2, (short) (value >>> 16)); }
    /// Writes a little-endian long.
    static void putLong(Object array, long offset, long value) { putInt(array, offset, (int) value); putInt(array, offset + 4, (int) (value >>> 32)); }
    /// Copies a checked array range, supporting overlapping source and destination ranges.
    static void copyMemory(Object source, long from, Object target, long to, long size) { System.arraycopy(source, index(from), target, index(to), index(size)); }
}

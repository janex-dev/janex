// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.zip.ZipException;

import org.glavo.janex.reader.ReadLimits;

/// Locates single-disk ZIP directories and validates their referenced records without decoding data.
public final class ZipDirectory {
    /// Physical archive start to which ZIP offsets are relative.
    final long base;
    /// Physical central-directory start.
    final long start;
    /// Physical central-directory end, before the ZIP end records.
    final long end;
    /// Bounded number of central-directory entries.
    final int count;

    /// Returns the physical archive start used as the base of ZIP offsets.
    public long base() {
        return base;
    }

    /// Retains checked directory coordinates.
    private ZipDirectory(long base, long start, long end, int count) {
        this.base = base;
        this.start = start;
        this.end = end;
        this.count = count;
    }

    /// Reads an exact range from an immutable source without retaining a shared cursor.
    @FunctionalInterface
    public interface Reader {
        /// Returns the requested bytes or throws when the source cannot supply them.
        byte[] read(long offset, long length) throws IOException;
    }

    /// Receives an entry after its central, local, and optional descriptor records validate.
    @FunctionalInterface
    interface Visitor {
        /// Processes an entry without taking ownership of the underlying source.
        void visit(Entry entry) throws IOException;
    }

    /// One validated entry's header, name, and physical data coordinates.
    static final class Entry {
        /// Fixed central-directory header bytes.
        final byte[] header;
        /// Raw filename shared by the central and local headers.
        final byte[] name;
        /// Physical local-header start.
        final long local;
        /// Physical compressed-data start.
        final long data;
        /// End of compressed data and any data descriptor.
        final long end;
        /// Compressed length.
        final long stored;
        /// Uncompressed length, retaining all unsigned bits.
        final long decoded;

        /// Retains validated header data and coordinates.
        Entry(byte[] header, byte[] name, long local, long data, long end, long stored, long decoded) {
            this.header = header;
            this.name = name;
            this.local = local;
            this.data = data;
            this.end = end;
            this.stored = stored;
            this.decoded = decoded;
        }
    }

    /// Returns all structurally valid terminal directories, retaining ambiguous candidates.
    public static List<ZipDirectory> find(long length, Reader reader) throws IOException {
        return find(length, reader, ReadLimits.DEFAULT);
    }

    /// Discovers terminal directories using the caller's candidate and entry-count limits.
    public static List<ZipDirectory> find(long length, Reader reader, ReadLimits limits) throws IOException {
        long tailStart = Math.max(0, length - 65557);
        byte[] tail = reader.read(tailStart, length - tailStart);
        List<ZipDirectory> result = new ArrayList<ZipDirectory>();
        for (int i = 0; i <= tail.length - 22; i++) {
            if (integer(tail, i, 4) != 0x06054b50L || i + 22 + integer(tail, i + 20, 2) != tail.length
                    || integer(tail, i + 4, 4) != 0) {
                continue;
            }
            byte[] record = Arrays.copyOfRange(tail, i, i + 22);
            for (ZipDirectory directory : directories(tailStart + i, record, reader, limits)) {
                try {
                    directory.entries(reader, entry -> {});
                    limits.elements(result.size() + 1L);
                    result.add(directory);
                } catch (ZipException invalid) {
                    // An invalid ZIP candidate does not hide other terminal records.
                }
            }
        }
        return result;
    }

    /// Derives directory candidates from ordinary or ZIP64 end records.
    private static List<ZipDirectory> directories(long eocd, byte[] record, Reader reader, ReadLimits limits) throws IOException {
        long count = integer(record, 10, 2);
        long size = integer(record, 12, 4);
        long offset = integer(record, 16, 4);
        byte[] locator = eocd < 20 ? new byte[0] : reader.read(eocd - 20, 20);
        List<ZipDirectory> result = new ArrayList<ZipDirectory>();
        if (locator.length == 0 || integer(locator, 0, 4) != 0x07064b50L) {
            if (count == integer(record, 8, 2) && count != 65535 && size != 0xffffffffL && offset != 0xffffffffL) {
                coordinates(result, eocd, size, offset, count, limits);
            }
            return result;
        }
        if (integer(locator, 4, 4) != 0 || integer(locator, 16, 4) != 1) {
            return result;
        }
        long relative = integer(locator, 8, 8);
        long limit = eocd - 20;
        if (relative < 0) {
            return result;
        }
        for (long scan = relative; scan <= limit - 56;) {
            byte[] bytes = reader.read(scan, Math.min(65536, limit - scan));
            for (int i = 0; i <= bytes.length - 4; i++) {
                long position = scan + i;
                if (integer(bytes, i, 4) != 0x06064b50L || position > limit - 56) {
                    continue;
                }
                byte[] fixed = reader.read(position, 56);
                long recordSize = integer(fixed, 4, 8);
                if (recordSize != limit - position - 12) {
                    continue;
                }
                long count64 = integer(fixed, 32, 8);
                long size64 = integer(fixed, 40, 8);
                long offset64 = integer(fixed, 48, 8);
                if (integer(fixed, 16, 8) != 0 || integer(fixed, 24, 8) != count64
                        || count != 65535 && count != count64
                        || integer(record, 8, 2) != 65535 && integer(record, 8, 2) != count64
                        || size != 0xffffffffL && size != size64 || offset != 0xffffffffL && offset != offset64
                        || size64 < 0 || offset64 < 0 || offset64 > relative || size64 != relative - offset64) {
                    continue;
                }
                coordinates(result, position, size64, offset64, count64, limits);
            }
            scan += bytes.length - 3;
        }
        return result;
    }

    /// Adds one bounded set of physical coordinates when all subtractions are valid.
    private static void coordinates(List<ZipDirectory> result, long end, long size, long offset, long count, ReadLimits limits) throws IOException {
        if (size >= 0 && size <= end && offset >= 0 && offset <= end - size) {
            limits.elements(result.size() + 1L);
            result.add(new ZipDirectory(end - size - offset, end - size, end, limits.elements(count)));
        }
    }

    /// Visits central-directory entries after validating referenced local records and data bounds.
    void entries(Reader reader, Visitor visitor) throws IOException {
        long position = start;
        for (int i = 0; i < count; i++) {
            check(position <= end - 46, "Truncated ZIP central directory");
            byte[] header = reader.read(position, 46);
            check(integer(header, 0, 4) == 0x02014b50L, "Incorrect ZIP directory signature");
            int nameLength = (int) integer(header, 28, 2);
            int extraLength = (int) integer(header, 30, 2);
            int variableLength = nameLength + extraLength + (int) integer(header, 32, 2);
            check(variableLength <= end - position - 46, "ZIP entry exceeds central directory");
            byte[] variable = reader.read(position + 46, variableLength);
            long[] fields = {integer(header, 24, 4), integer(header, 20, 4), integer(header, 42, 4), integer(header, 34, 2)};
            expand(variable, nameLength, extraLength, fields);
            check(fields[3] == 0, "Multidisk JAR is unsupported");
            check(fields[2] >= 0 && fields[2] <= start - base - 30, "ZIP local header outside data region");
            byte[] name = Arrays.copyOf(variable, nameLength);
            visitor.visit(local(reader, base + fields[2], header, name, fields[1], fields[0]));
            position += 46 + variableLength;
        }
        if (position != end) {
            check(position <= end - 6, "Trailing ZIP directory bytes");
            byte[] signature = reader.read(position, 6);
            check(integer(signature, 0, 4) == 0x05054b50L && integer(signature, 4, 2) == end - position - 6,
                    "Invalid ZIP directory digital signature record");
        }
    }

    /// Validates a local header and optional 32-bit or 64-bit data descriptor.
    private Entry local(Reader reader, long position, byte[] central, byte[] name, long stored, long decoded) throws IOException {
        byte[] header = reader.read(position, 30);
        check(integer(header, 0, 4) == 0x04034b50L && integer(header, 6, 4) == integer(central, 8, 4),
                "ZIP local and central headers disagree");
        int nameLength = (int) integer(header, 26, 2);
        int extraLength = (int) integer(header, 28, 2);
        check(nameLength + extraLength <= start - position - 30, "ZIP local fields exceed data region");
        byte[] variable = reader.read(position + 30, nameLength + extraLength);
        check(name.length == nameLength, "ZIP local and central names disagree");
        for (int i = 0; i < nameLength; i++) {
            check(name[i] == variable[i], "ZIP local and central names disagree");
        }
        long data = position + 30 + nameLength + extraLength;
        check(stored >= 0 && stored <= start - data, "ZIP payload outside data region");
        long dataEnd = data + stored;
        long crc = integer(central, 16, 4);
        if ((integer(header, 6, 2) & 8) == 0) {
            long[] sizes = {integer(header, 22, 4), integer(header, 18, 4)};
            expand(variable, nameLength, extraLength, sizes);
            check(sizes[0] == decoded && sizes[1] == stored && integer(header, 14, 4) == crc,
                    "ZIP local sizes or CRC disagree with directory");
        } else {
            byte[] descriptor = reader.read(dataEnd, Math.min(24, start - dataEnd));
            int descriptorLength = 0;
            for (int skip : new int[]{0, 4}) {
                if (descriptor.length < skip + 12 || skip == 4 && integer(descriptor, 0, 4) != 0x08074b50L
                        || integer(descriptor, skip, 4) != crc) {
                    continue;
                }
                if (integer(descriptor, skip + 4, 4) == stored && integer(descriptor, skip + 8, 4) == decoded) {
                    descriptorLength = skip + 12;
                } else if (descriptor.length >= skip + 20 && integer(descriptor, skip + 4, 8) == stored
                        && integer(descriptor, skip + 12, 8) == decoded) {
                    descriptorLength = skip + 20;
                }
                if (descriptorLength != 0) {
                    break;
                }
            }
            check(descriptorLength != 0, "Invalid ZIP data descriptor");
            dataEnd += descriptorLength;
        }
        return new Entry(central, name, position, data, dataEnd, stored, decoded);
    }

    /// Expands placeholder sizes, offsets, and disk numbers from one optional ZIP64 extra field.
    private static void expand(byte[] bytes, int start, int length, long[] fields) throws IOException {
        int end = start + length;
        boolean expanded = false;
        while (start < end) {
            check(start <= end - 4, "Truncated ZIP extra field");
            int tag = (int) integer(bytes, start, 2);
            int size = (int) integer(bytes, start + 2, 2);
            start += 4;
            check(size <= end - start, "ZIP extra field exceeds header");
            if (tag == 1) {
                check(!expanded, "Duplicate ZIP64 extra field");
                expanded = true;
                int position = start;
                for (int i = 0; i < fields.length; i++) {
                    if (fields[i] == (i == 3 ? 65535 : 0xffffffffL)) {
                        int width = i == 3 ? 4 : 8;
                        check(width <= start + size - position, "Missing ZIP64 extra-field value");
                        fields[i] = integer(bytes, position, width);
                        position += width;
                    }
                }
            }
            start += size;
        }
        for (int i = 0; i < fields.length; i++) {
            check(expanded || fields[i] != (i == 3 ? 65535 : 0xffffffffL), "Missing ZIP64 extra field");
        }
    }

    /// Reads a little-endian field from a bounded record, retaining unsigned 64-bit values.
    static long integer(byte[] bytes, int offset, int width) throws IOException {
        check(offset >= 0 && offset <= bytes.length - width, "Truncated ZIP integer");
        long result = 0;
        for (int i = 0; i < width; i++) {
            result |= (long) (bytes[offset + i] & 255) << (i * 8);
        }
        return result;
    }

    /// Distinguishes malformed archive candidates from I/O failures and resource-limit failures.
    private static void check(boolean condition, String message) throws ZipException {
        if (!condition) {
            throw new ZipException(message);
        }
    }
}

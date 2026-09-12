// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.IOException;
import java.util.*;
import java.util.zip.CRC32;
import java.util.zip.DataFormatException;
import java.util.zip.Inflater;

import static org.janex.format.Input.*;

/// Reads ordinary ZIP central-directory entries without extracting archive paths.
final class JarArchive {
    /// Prevents instantiation.
    private JarArchive() {
    }

    /// One completely decoded entry with its original UTF-8 name and optional Unix mode.
    static final class Entry {
        /// Original name, including a directory's trailing slash.
        final String name;
        /// Decoded bytes verified against the entry size and CRC.
        final byte[] bytes;
        /// Unix mode including file type, or -1 when not recorded.
        final int mode;
        /// Unix file type, or zero when unspecified.
        final int kind;

        /// Retains a validated entry's owned data.
        Entry(String name, byte[] bytes, int mode) {
            this.name = name;
            this.bytes = bytes;
            this.mode = mode;
            this.kind = mode < 0 ? 0 : mode & 0170000;
        }
    }

    /// Reads bounded stored or deflated entries, rejecting duplicate names and overlapping data.
    static List<Entry> read(byte[] bytes) throws IOException {
        size(bytes.length);
        int end = -1;
        for (int offset = bytes.length - 22; offset >= Math.max(0, bytes.length - 65557); offset--) {
            if (integer(bytes, offset, 4) == 0x06054b50 && offset + 22 + integer(bytes, offset + 20, 2) == bytes.length) {
                end = offset;
                break;
            }
        }
        require(end >= 0, "Missing terminal ZIP end record");
        require(integer(bytes, end + 4, 4) == 0, "Multidisk JAR is unsupported");
        int entries = (int) integer(bytes, end + 10, 2);
        require(entries != 65535 && entries == integer(bytes, end + 8, 2), "Invalid or unsupported ZIP64 entry count");
        int directorySize = size(integer(bytes, end + 12, 4));
        int directoryOffset = size(integer(bytes, end + 16, 4));
        int base = end - directorySize - directoryOffset;
        require(base >= 0, "ZIP directory outside input");
        int position = base + directoryOffset;
        int directory = position;
        long total = 0;
        Set<String> names = new HashSet<String>();
        SortedMap<Integer, Integer> ranges = new TreeMap<Integer, Integer>();
        List<Entry> result = new ArrayList<Entry>();
        for (int index = 0; index < entries; index++) {
            require(position <= end - 46 && integer(bytes, position, 4) == 0x02014b50, "Invalid ZIP directory entry");
            int creator = (int) integer(bytes, position + 4, 2);
            int flags = (int) integer(bytes, position + 8, 2);
            int method = (int) integer(bytes, position + 10, 2);
            require((flags & ~0x80e) == 0 && (method == 0 || method == 8), "Unsupported JAR encryption or compression");
            long crc = integer(bytes, position + 16, 4);
            int stored = size(integer(bytes, position + 20, 4));
            int decoded = size(integer(bytes, position + 24, 4));
            total += decoded;
            size(total);
            int nameLength = (int) integer(bytes, position + 28, 2);
            int extraLength = (int) integer(bytes, position + 30, 2);
            int commentLength = (int) integer(bytes, position + 32, 2);
            require(integer(bytes, position + 34, 2) == 0, "Multidisk JAR is unsupported");
            int mode = mode(creator >> 8, integer(bytes, position + 38, 4));
            int local = size((long) base + integer(bytes, position + 42, 4));
            int next = position + 46 + nameLength + extraLength + commentLength;
            require(next <= end, "Truncated ZIP directory entry");
            byte[] rawName = Arrays.copyOfRange(bytes, position + 46, position + 46 + nameLength);
            String name = utf8(rawName);
            require(names.add(name), "Duplicate JAR entry name");
            require(local <= directory - 30 && integer(bytes, local, 4) == 0x04034b50, "Invalid ZIP local header");
            require(integer(bytes, local + 6, 2) == flags && integer(bytes, local + 8, 2) == method,
                    "ZIP local and central headers disagree");
            int localName = (int) integer(bytes, local + 26, 2);
            int localExtra = (int) integer(bytes, local + 28, 2);
            int start = local + 30 + localName + localExtra;
            require(start <= directory && stored <= directory - start, "JAR payload outside archive");
            require(localName == nameLength && Arrays.equals(rawName, Arrays.copyOfRange(bytes, local + 30, local + 30 + localName)),
                    "ZIP local and central names disagree");
            if ((flags & 8) == 0) {
                require(integer(bytes, local + 14, 4) == crc && integer(bytes, local + 18, 4) == stored
                        && integer(bytes, local + 22, 4) == decoded, "ZIP local and central sizes disagree");
            }
            int rangeEnd = start + stored;
            if ((flags & 8) != 0) {
                int descriptor = rangeEnd;
                if (integer(bytes, descriptor, 4) == 0x08074b50) {
                    descriptor += 4;
                }
                require(descriptor <= directory - 12 && integer(bytes, descriptor, 4) == crc
                        && integer(bytes, descriptor + 4, 4) == stored && integer(bytes, descriptor + 8, 4) == decoded,
                        "Invalid ZIP data descriptor");
                rangeEnd = descriptor + 12;
            }
            require(ranges.put(local, rangeEnd) == null, "Shared ZIP local header");
            byte[] content = decode(bytes, start, stored, decoded, method);
            CRC32 checksum = new CRC32();
            checksum.update(content);
            require(checksum.getValue() == crc, "JAR entry CRC mismatch");
            Entry entry = new Entry(name, content, mode);
            require(entry.kind == 0 || entry.kind == 0100000 || entry.kind == 0040000 || entry.kind == 0120000,
                    "Unsupported JAR filesystem node");
            if (name.endsWith("/")) {
                require(content.length == 0 && (entry.kind == 0 || entry.kind == 0040000), "Invalid JAR directory");
            } else {
                require(entry.kind != 0040000, "JAR directory name disagrees with file mode");
            }
            result.add(entry);
            position = next;
        }
        require(position == end, "ZIP directory size or count mismatch");
        int previous = base;
        for (Map.Entry<Integer, Integer> range : ranges.entrySet()) {
            require(range.getKey() >= previous, "Overlapping JAR entries");
            previous = range.getValue();
        }
        return result;
    }

    /// Retains Unix attributes from any producer, otherwise maps DOS mode bits as the Host does.
    private static int mode(int system, long attributes) {
        if (attributes == 0) {
            return -1;
        }
        int unix = (int) (attributes >>> 16);
        if (unix != 0 || system == 3) {
            return unix;
        }
        if (system == 0) {
            int mode = (attributes & 0x10) != 0 ? 0040775 : 0100664;
            return (attributes & 1) != 0 ? mode & ~0222 : mode;
        }
        return -1;
    }

    /// Decodes one exact payload without consuming adjacent ZIP structures.
    private static byte[] decode(byte[] bytes, int start, int stored, int decoded, int method) throws IOException {
        if (method == 0) {
            require(stored == decoded, "Stored JAR entry size mismatch");
            return Arrays.copyOfRange(bytes, start, start + stored);
        }
        Inflater inflater = new Inflater(true);
        try {
            inflater.setInput(bytes, start, stored);
            byte[] result = new byte[decoded];
            int position = 0;
            while (position < decoded && !inflater.finished()) {
                int produced = inflater.inflate(result, position, decoded - position);
                require(produced > 0 || inflater.finished(), "Incomplete JAR deflate stream");
                position += produced;
            }
            require(inflater.inflate(new byte[1]) == 0 && inflater.finished() && position == decoded && inflater.getRemaining() == 0,
                    "JAR deflate size or framing mismatch");
            return result;
        } catch (DataFormatException failure) {
            throw new IOException("Invalid JAR deflate stream", failure);
        } finally {
            inflater.end();
        }
    }

    /// Reads one bounded unsigned ZIP integer.
    private static long integer(byte[] bytes, int offset, int length) throws IOException {
        require(offset >= 0 && offset <= bytes.length - length, "Truncated ZIP structure");
        long result = 0;
        for (int index = 0; index < length; index++) {
            result |= (long) (bytes[offset + index] & 255) << (index * 8);
        }
        return result;
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal;

import java.io.IOException;
import java.util.*;
import java.util.zip.CRC32;
import java.util.zip.DataFormatException;
import java.util.zip.Inflater;

import org.glavo.janex.reader.ReadLimits;

import static org.glavo.janex.reader.internal.Input.*;

/// Reads ZIP and ZIP64 central-directory entries without extracting archive paths.
public final class JarArchive {
    /// Prevents instantiation.
    private JarArchive() {
    }

    /// One completely decoded entry with its original UTF-8 name and optional Unix mode.
    public static final class Entry {
        /// Original name, including a directory's trailing slash.
        private final String name;
        /// Decoded bytes verified against the entry size and CRC.
        private final byte[] bytes;
        /// Unix mode including file type, or -1 when not recorded.
        private final int mode;
        /// Unix file type, or zero when unspecified.
        private final int kind;

        /// Returns original entry name.
        public String name() {
            return name;
        }

        /// Returns an owned copy of the decoded entry bytes.
        public byte[] bytes() {
            return bytes.clone();
        }

        /// Returns Unix mode, or -1 when absent.
        public int mode() {
            return mode;
        }

        /// Returns Unix file type, or zero when unspecified.
        public int kind() {
            return kind;
        }

        /// Retains a validated entry's owned data.
        Entry(String name, byte[] bytes, int mode) {
            this.name = name;
            this.bytes = bytes;
            this.mode = mode;
            this.kind = mode < 0 ? 0 : mode & 0170000;
        }
    }

    /// Reads bounded stored or deflated entries, rejecting duplicate names and overlapping data.
    public static List<Entry> read(byte[] bytes) throws IOException {
        return read(bytes, ReadLimits.DEFAULT);
    }

    /// Reads an archive with the caller's encoded, decoded, aggregate, and entry-count limits.
    public static List<Entry> read(byte[] bytes, ReadLimits limits) throws IOException {
        limits.bytes(bytes.length);
        ZipDirectory.Reader reader = (offset, length) -> {
            require(offset >= 0 && length >= 0 && offset <= bytes.length - length, "ZIP range exceeds input");
            return Arrays.copyOfRange(bytes, (int) offset, (int) (offset + length));
        };
        List<ZipDirectory> directories = ZipDirectory.find(bytes.length, reader, limits);
        require(directories.size() == 1, "Missing or ambiguous ZIP directory");
        ZipDirectory directory = directories.get(0);
        long[] total = {0};
        Set<String> names = new HashSet<String>();
        SortedMap<Long, Long> ranges = new TreeMap<Long, Long>();
        List<Entry> result = new ArrayList<Entry>();
        directory.entries(reader, record -> {
            byte[] header = record.header;
            int creator = (int) ZipDirectory.integer(header, 4, 2);
            int flags = (int) ZipDirectory.integer(header, 8, 2);
            int method = (int) ZipDirectory.integer(header, 10, 2);
            require((flags & ~0x80e) == 0 && (method == 0 || method == 8), "Unsupported JAR encryption or compression");
            long crc = ZipDirectory.integer(header, 16, 4);
            int stored = limits.bytes(record.stored);
            int decoded = limits.bytes(record.decoded);
            total[0] += decoded;
            limits.bytes(total[0]);
            String name = utf8(record.name);
            require(names.add(name), "Duplicate JAR entry name");
            require(ranges.put(record.local, record.end) == null, "Shared ZIP local header");
            byte[] content = decode(bytes, (int) record.data, stored, decoded, method);
            CRC32 checksum = new CRC32();
            checksum.update(content);
            require(checksum.getValue() == crc, "JAR entry CRC mismatch");
            int mode = mode(creator >> 8, ZipDirectory.integer(header, 38, 4));
            Entry entry = new Entry(name, content, mode);
            require(entry.kind == 0 || entry.kind == 0100000 || entry.kind == 0040000 || entry.kind == 0120000,
                    "Unsupported JAR filesystem node");
            if (name.endsWith("/")) {
                require(content.length == 0 && (entry.kind == 0 || entry.kind == 0040000), "Invalid JAR directory");
            } else {
                require(entry.kind != 0040000, "JAR directory name disagrees with file mode");
            }
            result.add(entry);
        });
        long previous = directory.base;
        for (Map.Entry<Long, Long> range : ranges.entrySet()) {
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

}

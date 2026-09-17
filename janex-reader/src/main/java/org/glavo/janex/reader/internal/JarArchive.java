// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal;

import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.file.Path;
import java.util.*;

import org.glavo.janex.reader.ReadLimits;

import static org.glavo.janex.reader.internal.Input.*;

/// Reads ZIP and ZIP64 central-directory entries without extracting archive paths.
public final class JarArchive {
    /// Prevents instantiation.
    private JarArchive() {
    }

    /// One decoded or snapshot-backed entry with its original UTF-8 name and optional Unix mode.
    public static final class Entry {
        /// Original name, including a directory's trailing slash.
        private final String name;
        /// Decoded bytes verified against the entry size and CRC, or null for a deferred entry.
        private final byte[] bytes;
        /// Deferred snapshot payload, or null for a completely decoded entry.
        private final JarSource source;
        /// Unix mode including file type, or -1 when not recorded.
        private final int mode;
        /// Unix file type, or zero when unspecified.
        private final int kind;

        /// Returns original entry name.
        public String name() {
            return name;
        }

        /// Returns independently owned decoded bytes, reading and verifying a deferred payload.
        /// @throws IOException if the snapshot cannot be read or payload validation fails
        public byte[] bytes() throws IOException {
            return source == null ? bytes.clone() : source.read();
        }

        /// Returns the deferred payload descriptor, or null for a completely decoded entry.
        public JarSource source() {
            return source;
        }

        /// Returns Unix mode, or -1 when absent.
        public int mode() {
            return mode;
        }

        /// Returns Unix file type, or zero when unspecified.
        public int kind() {
            return kind;
        }

        /// Retains owned decoded bytes or an immutable deferred payload descriptor.
        Entry(String name, byte[] bytes, JarSource source, int mode) {
            this.name = name;
            this.bytes = bytes;
            this.source = source;
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
        return read(bytes.length, reader, limits, null);
    }

    /// Indexes an unchanged snapshot without decoding ordinary entry payloads.
    /// Encoded size, aggregate decoded size, entry count, names, ranges, and node types are checked
    /// during indexing. Payload framing and CRCs are checked when [Entry#bytes()] is called.
    /// The caller retains ownership of the file and must keep it unchanged while using the entries.
    /// @param path caller-owned JAR snapshot
    /// @param limits per-archive, aggregate decoded, and entry-count bounds
    /// @return entry descriptors independent of the closed indexing handle
    /// @throws IOException if archive metadata is invalid or exceeds a limit
    public static List<Entry> index(Path path, ReadLimits limits) throws IOException {
        try (RandomAccessFile file = new RandomAccessFile(path.toFile(), "r")) {
            long length = file.length();
            limits.bytes(length);
            return read(length, new SnapshotReader(file, length), limits, path);
        }
    }

    /// Buffers two directory/header windows to coalesce alternating central and local record reads.
    private static final class SnapshotReader implements ZipDirectory.Reader {
        /// Caller-owned archive handle, used only during single-threaded indexing.
        private final RandomAccessFile file;
        /// Immutable archive length already checked against the byte policy.
        private final long length;
        /// Reusable 64 KiB windows, bounded by archive length for smaller inputs.
        private final byte[][] windows;
        /// Physical start of each window; -1 denotes an unfilled buffer.
        private final long[] starts = {-1, -1};
        /// Number of valid bytes in each window.
        private final int[] counts = new int[2];
        /// Least recently used window, replaced on a miss.
        private int next;

        /// Retains a bounded archive handle without taking ownership of it.
        SnapshotReader(RandomAccessFile file, long length) {
            this.file = file;
            this.length = length;
            windows = new byte[2][(int) Math.min(length, 65536)];
        }

        /// Returns an owned range, preserving cached central records across local-header reads.
        @Override
        public byte[] read(long offset, long count) throws IOException {
            require(offset >= 0 && count >= 0 && offset <= length - count, "ZIP range exceeds input");
            for (int i = 0; i < 2; i++) {
                if (starts[i] >= 0 && offset >= starts[i] && offset + count <= starts[i] + counts[i]) {
                    next = 1 - i;
                    return Arrays.copyOfRange(windows[i], (int) (offset - starts[i]), (int) (offset - starts[i] + count));
                }
            }
            if (count > windows[0].length) {
                byte[] bytes = new byte[(int) count];
                file.seek(offset);
                file.readFully(bytes);
                return bytes;
            }
            int index = next;
            next = 1 - index;
            starts[index] = offset;
            counts[index] = (int) Math.min(windows[index].length, length - offset);
            file.seek(offset);
            file.readFully(windows[index], 0, counts[index]);
            return Arrays.copyOf(windows[index], (int) count);
        }
    }

    /// Parses shared archive metadata, decoding payloads only when no snapshot path is supplied.
    private static List<Entry> read(long length, ZipDirectory.Reader reader, ReadLimits limits, Path path) throws IOException {
        List<ZipDirectory> directories = ZipDirectory.find(length, reader, limits);
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
            require(method != 0 || stored == decoded, "Stored JAR entry size mismatch");
            byte[] content = path == null
                    ? JarSource.decode(reader.read(record.data, stored), 0, stored, decoded, method, crc) : null;
            JarSource source = path == null ? null : new JarSource(path, record.data, stored, decoded, method, crc);
            int mode = mode(creator >> 8, ZipDirectory.integer(header, 38, 4));
            Entry entry = new Entry(name, content, source, mode);
            require(entry.kind == 0 || entry.kind == 0100000 || entry.kind == 0040000 || entry.kind == 0120000,
                    "Unsupported JAR filesystem node");
            if (name.endsWith("/")) {
                require(decoded == 0 && (entry.kind == 0 || entry.kind == 0040000), "Invalid JAR directory");
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
        // Some Maven JARs store -1 as an unspecified Unix mode.
        if (unix == 0xffff) {
            return -1;
        }
        if (unix != 0 || system == 3) {
            return unix;
        }
        if (system == 0) {
            int mode = (attributes & 0x10) != 0 ? 0040775 : 0100664;
            return (attributes & 1) != 0 ? mode & ~0222 : mode;
        }
        return -1;
    }

}

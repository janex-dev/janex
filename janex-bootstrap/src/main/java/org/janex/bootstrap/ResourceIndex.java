// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.*;
import java.util.*;
import org.janex.bootstrap.internal.zstd.ZstdFrameDecompressor;

/// Reads Host-selected resources from a private snapshot without rebuilding classpath JARs.
///
/// The Host authenticates the complete snapshot before publishing this index. This reader
/// does not establish publisher trust or repeat per-entry checksum verification.
final class ResourceIndex implements Closeable {
    /// Maximum size of an individual decoded value.
    private final int maxBytes;
    /// Maximum number of elements in one collection.
    private final int maxElements;
    /// Open snapshot, retained until this reader is closed or the JVM exits.
    private final RandomAccessFile snapshot;
    /// Topologically ordered byte sources.
    private final Source[] sources;
    /// String pools shared between CLASSFILE transforms.
    private final String[][] pools;
    /// Classpath roots in lookup order.
    final List<Root> roots;
    /// Decoded blob cache in least-recently-used order; values are never exposed mutably.
    private final LinkedHashMap<Integer, byte[]> cache = new LinkedHashMap<Integer, byte[]>(16, 0.75f, true);
    /// Current decoded cache size.
    private long cachedBytes;
    /// Maximum decoded cache retention across all sources.
    private final long cacheLimit;
    /// Whether the owned snapshot has been closed.
    private boolean closed;

    /// Parses bounded launch data, closing the snapshot if construction fails.
    /// @param stream private index bytes supplied by the Host; consumed and closed
    /// @throws IOException if the index or snapshot cannot be read
    ResourceIndex(InputStream stream) throws IOException {
        RandomAccessFile opened = null;
        try (DataInputStream input = new DataInputStream(new BufferedInputStream(stream))) {
            byte[] magic = new byte[8];
            input.readFully(magic);
            if (!Arrays.equals(magic, new byte[] {'J','N','X','R','E','S','0','1'})) throw new IOException("Invalid resource index");
            maxBytes = nonnegative(input.readInt());
            maxElements = nonnegative(input.readInt());
            cacheLimit = Math.min(maxBytes, 64L * 1024 * 1024);
            opened = new RandomAccessFile(text(input), "r");
            snapshot = opened;
            sources = new Source[count(input)];
            for (int i = 0; i < sources.length; i++) sources[i] = new Source(input, i);
            pools = new String[count(input)][];
            for (int i = 0; i < pools.length; i++) {
                pools[i] = new String[count(input)];
                for (int j = 0; j < pools[i].length; j++) pools[i][j] = text(input);
                if (pools[i].length == 0 || !pools[i][0].isEmpty()) throw new IOException("Invalid string pool");
            }
            List<Root> result = new ArrayList<Root>();
            int count = count(input);
            for (int i = 0; i < count; i++) result.add(new Root(input));
            roots = Collections.unmodifiableList(result);
            if (input.read() != -1) throw new IOException("Trailing resource index data");
        } catch (Throwable failure) {
            if (opened != null) try { opened.close(); } catch (IOException close) { failure.addSuppressed(close); }
            throw failure;
        }
    }

    /// Returns a nonnegative integer or rejects its wire representation.
    private static int nonnegative(int value) throws IOException {
        if (value < 0) throw new IOException("Negative resource index length");
        return value;
    }
    /// Reads a bounded collection count.
    private int count(DataInputStream input) throws IOException {
        int count = nonnegative(input.readInt());
        if (count > maxElements) throw new IOException("Resource index element limit exceeded");
        return count;
    }
    /// Reads a bounded byte length.
    private int size(DataInputStream input) throws IOException { return size(input.readInt()); }
    /// Checks a decoded byte length before allocation.
    private int size(long size) throws IOException {
        if (size < 0 || size > maxBytes) throw new IOException("Resource byte limit exceeded");
        return (int) size;
    }
    /// Reads exact UTF-16 code units without normalization.
    private String text(DataInputStream input) throws IOException {
        int length = nonnegative(input.readInt());
        size(length * 2L);
        char[] chars = new char[length];
        for (int i = 0; i < length; i++) chars[i] = input.readChar();
        return new String(chars);
    }

    /// One inline, stored, or extent-assembled source.
    private final class Source {
        /// Inline bytes, or null for a snapshot range or extents.
        final byte[] inline;
        /// Physical snapshot offset; negative for extents.
        final long offset;
        /// Encoded size for a snapshot range.
        final int stored;
        /// Required output sizes of successive reversed Zstd filters.
        final int[] filters;
        /// Extent triples: earlier source ID, decoded offset, decoded length.
        final int[][] extents;
        /// Final decoded byte length.
        final int length;

        /// Parses a source, checking snapshot bounds and strictly earlier extent references.
        Source(DataInputStream input, int index) throws IOException {
            int kind = input.readUnsignedByte();
            if (kind == 0) {
                inline = new byte[size(input)];
                input.readFully(inline);
                offset = -1; stored = 0; filters = new int[0]; extents = null; length = inline.length;
            } else if (kind == 1) {
                inline = null; extents = null;
                offset = input.readLong(); stored = size(input);
                if (offset < 0 || offset > snapshot.length() - stored) throw new IOException("Resource range exceeds snapshot");
                filters = new int[count(input)];
                for (int i = 0; i < filters.length; i++) filters[i] = size(input);
                length = filters.length == 0 ? stored : filters[filters.length - 1];
            } else if (kind == 2) {
                inline = null; offset = -1; stored = 0; filters = new int[0];
                extents = new int[count(input)][3];
                long total = 0;
                for (int[] extent : extents) {
                    extent[0] = nonnegative(input.readInt());
                    extent[1] = size(input); extent[2] = size(input);
                    if (extent[0] >= index || sources[extent[0]].extents != null || extent[2] == 0
                            || extent[1] > sources[extent[0]].length - extent[2]) throw new IOException("Invalid resource extent");
                    total += extent[2];
                }
                length = size(total);
            } else throw new IOException("Unknown resource source");
        }
    }

    /// Returns privately owned decoded bytes; callers must not modify them.
    private synchronized byte[] source(int id) throws IOException {
        if (closed) throw new IOException("Resource reader is closed");
        byte[] bytes = cache.get(id);
        if (bytes != null) return bytes;
        Source source = sources[id];
        if (source.inline != null) return source.inline;
        if (source.extents != null) {
            bytes = new byte[source.length];
            int offset = 0;
            for (int[] extent : source.extents) {
                System.arraycopy(source(extent[0]), extent[1], bytes, offset, extent[2]);
                offset += extent[2];
            }
        } else {
            bytes = new byte[source.stored];
            snapshot.seek(source.offset);
            snapshot.readFully(bytes);
            try {
                for (int length : source.filters) {
                    byte[] output = new byte[length];
                    int written = new ZstdFrameDecompressor().decompress(bytes, 0, bytes.length, output, 0, output.length);
                    if (written != length) throw new IOException("Zstd decoded size mismatch");
                    bytes = output;
                }
            } catch (RuntimeException invalid) { throw new IOException("Invalid Zstd resource", invalid); }
        }
        if (bytes.length <= cacheLimit) {
            Iterator<byte[]> oldest = cache.values().iterator();
            while ((cachedBytes + bytes.length > cacheLimit || cache.size() >= 4096) && oldest.hasNext()) { cachedBytes -= oldest.next().length; oldest.remove(); }
            cache.put(id, bytes); cachedBytes += bytes.length;
        }
        return bytes;
    }

    /// A single logical classpath root, preserving its original resource names.
    final class Root {
        /// Original JAR filename used in resource and code-source URLs.
        final String name;
        /// Resources in Host traversal order, including explicit directories.
        final Map<String, Resource> files;
        /// Reads one root and rejects duplicate names.
        Root(DataInputStream input) throws IOException {
            name = text(input);
            Map<String, Resource> entries = new LinkedHashMap<String, Resource>();
            int count = count(input);
            for (int i = 0; i < count; i++) {
                String path = text(input);
                if (entries.put(path, new Resource(input)) != null) throw new IOException("Duplicate resource name");
            }
            files = Collections.unmodifiableMap(entries);
        }
    }

    /// A directory or a logical file with its selected transform pools.
    final class Resource {
        /// Source ID, or -1 for a directory.
        final int id;
        /// Reversed CLASSFILE transforms as output-size and pool-ID pairs.
        final int[][] transforms;
        /// Logical resource length, zero for directories.
        final int length;
        /// Reads and validates a file descriptor.
        Resource(DataInputStream input) throws IOException {
            id = input.readInt();
            if (id < -1 || id >= sources.length) throw new IOException("Invalid resource source ID");
            transforms = new int[id == -1 ? 0 : count(input)][2];
            for (int[] transform : transforms) {
                transform[0] = size(input); transform[1] = nonnegative(input.readInt());
                if (transform[1] >= pools.length) throw new IOException("Invalid transform pool ID");
            }
            length = id == -1 ? 0 : transforms.length == 0 ? sources[id].length : transforms[transforms.length - 1][0];
        }
        /// Returns privately owned logical bytes; callers must not modify them.
        byte[] read() throws IOException {
            byte[] bytes = id == -1 ? new byte[0] : source(id);
            for (int[] transform : transforms) bytes = ClassFiles.restore(bytes, pools[transform[1]], transform[0]);
            return bytes;
        }
    }

    /// Closes the snapshot and releases cached decoded bytes; repeated calls are harmless.
    @Override public synchronized void close() throws IOException {
        if (!closed) { closed = true; cache.clear(); cachedBytes = 0; snapshot.close(); }
    }
}

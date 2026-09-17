// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap.loader;

import java.io.*;
import java.nio.file.Path;
import java.time.DateTimeException;
import java.time.Instant;
import java.util.*;

import org.glavo.janex.reader.ReadLimits;
import org.glavo.janex.reader.ResourcePlan;
import org.glavo.janex.reader.DataPool;
import org.glavo.janex.reader.internal.JarSource;
import org.glavo.janex.reader.internal.ResourceTable;
import org.glavo.janex.reader.internal.codec.ClassFiles;
import org.glavo.janex.reader.internal.codec.ZstandardFrames;
import org.glavo.janex.reader.internal.codec.zstd.Zstandard;

/// Reads Host-selected resources from unchanged input files without rebuilding classpath JARs.
///
/// The launch preparer verifies the snapshot before publishing this index and applies its own
/// trust policy. This reader does not establish publisher trust or repeat container entry checksums.
/// External JAR payload framing and CRCs are checked when their decoded bytes enter the cache.
public final class ResourceIndex implements Closeable {
    /// Immutable read limits shared by metadata and payload decoders.
    private final ReadLimits limits;
    /// Open snapshot, retained until this reader is closed or the JVM exits.
    private final RandomAccessFile snapshot;
    /// Length of the container when opened, shared by all source bounds checks.
    private final long snapshotLength;
    /// Topologically ordered byte sources.
    private final Source[] sources;
    /// Data pools shared between CLASSFILE transforms.
    private final DataPool[] pools;
    /// Classpath roots in lookup order.
    final List<Root> roots;
    /// Required observable module names and optional exact versions.
    final Map<String, String> requirements;
    /// Decoded blob cache in least-recently-used order; values are never exposed mutably.
    private final LinkedHashMap<Integer, byte[]> cache = new LinkedHashMap<Integer, byte[]>(16, 0.75f, true);
    /// Current decoded cache size.
    private long cachedBytes;
    /// Maximum decoded cache retention across all sources.
    private final long cacheLimit;
    /// External JAR handles opened on first payload access and owned by this index.
    private final Map<Path, RandomAccessFile> jars = new HashMap<Path, RandomAccessFile>();
    /// Whether the owned snapshot has been closed.
    private boolean closed;

    /// Parses bounded launch data, closing the snapshot if construction fails.
    ///
    /// @param stream private index bytes supplied by the Host; consumed and closed
    /// @throws IOException if the index or snapshot cannot be read
    public ResourceIndex(InputStream stream) throws IOException {
        RandomAccessFile opened = null;
        try (DataInputStream input = new DataInputStream(new BufferedInputStream(stream))) {
            byte[] magic = new byte[8];
            input.readFully(magic);
            if (!Arrays.equals(magic, new byte[]{'J', 'N', 'X', 'R', 'E', 'S', '0', '1'})) {
                throw new IOException("Invalid resource index");
            }
            int maxBytes = nonnegative(input.readInt());
            int maxElements = nonnegative(input.readInt());
            limits = new ReadLimits(maxBytes, maxElements, ReadLimits.DEFAULT.maxDepth());
            cacheLimit = Math.min(maxBytes, 64L * 1024 * 1024);
            opened = new RandomAccessFile(text(input), "r");
            snapshot = opened;
            snapshotLength = snapshot.length();
            sources = new Source[count(input)];
            for (int i = 0; i < sources.length; i++) {
                sources[i] = new Source(input, i);
            }
            pools = new DataPool[count(input)];
            for (int i = 0; i < pools.length; i++) {
                pools[i] = DataPool.readIndex(input, limits);
            }
            Map<String, String> required = new LinkedHashMap<String, String>();
            int requiredCount = count(input);
            for (int i = 0; i < requiredCount; i++) {
                String name = text(input);
                String version = text(input);
                String previous = required.get(name);
                if (previous != null && !previous.isEmpty() && !version.isEmpty() && !previous.equals(version)) {
                    throw new IOException("Conflicting required module version: " + name);
                }
                if (previous == null || !version.isEmpty()) {
                    required.put(name, version);
                }
            }
            requirements = Collections.unmodifiableMap(required);
            List<Root> result = new ArrayList<Root>();
            int count = count(input);
            for (int i = 0; i < count; i++) {
                result.add(new Root(input));
            }
            roots = Collections.unmodifiableList(result);
            if (input.read() != -1) {
                throw new IOException("Trailing resource index data");
            }
        } catch (Throwable failure) {
            if (opened != null) {
                try {
                    opened.close();
                } catch (IOException close) {
                    failure.addSuppressed(close);
                }
            }
            throw failure;
        }
    }

    /// Opens an immutable resource plan directly, without serializing an intermediate index.
    /// Open snapshot handles are owned by this index; immutable plan metadata may be shared.
    /// External JAR handles are opened on first payload access and closed with this index.
    /// Construction failure closes the opened snapshot.
    /// @param plan validated resources over an unchanged, caller-owned snapshot file
    /// @throws IOException if the snapshot cannot be opened
    public ResourceIndex(ResourcePlan plan) throws IOException {
        limits = plan.limits();
        cacheLimit = Math.min(limits.maxBytes(), 64L * 1024 * 1024);
        snapshot = new RandomAccessFile(plan.snapshot().toFile(), "r");
        try {
            snapshotLength = snapshot.length();
            sources = new Source[plan.sources().size()];
            for (int i = 0; i < sources.length; i++) sources[i] = new Source(plan.sources().get(i));
            pools = plan.pools();
            requirements = plan.requirements();
            List<Root> result = new ArrayList<Root>();
            for (ResourcePlan.Root root : plan.roots()) result.add(new Root(root));
            roots = Collections.unmodifiableList(result);
        } catch (Throwable failure) {
            try { snapshot.close(); } catch (IOException close) { failure.addSuppressed(close); }
            throw failure;
        }
    }

    /// Returns the immutable root list in launch order; entries share this index's lifetime.
    public List<Root> roots() {
        return roots;
    }

    /// Returns a nonnegative integer or rejects its wire representation.
    private static int nonnegative(int value) throws IOException {
        if (value < 0) {
            throw new IOException("Negative resource index length");
        }
        return value;
    }

    /// Reads a bounded collection count.
    private int count(DataInputStream input) throws IOException {
        int count = nonnegative(input.readInt());
        if (count > limits.maxElements()) {
            throw new IOException("Resource index element limit exceeded");
        }
        return count;
    }

    /// Reads a bounded byte length.
    private int size(DataInputStream input) throws IOException {
        return size(input.readInt());
    }

    /// Checks a decoded byte length before allocation.
    private int size(long size) throws IOException {
        if (size < 0 || size > limits.maxBytes()) {
            throw new IOException("Resource byte limit exceeded");
        }
        return (int) size;
    }

    /// Reads exact UTF-16 code units without normalization.
    private String text(DataInputStream input) throws IOException {
        int length = nonnegative(input.readInt());
        size(length * 2L);
        char[] chars = new char[length];
        for (int i = 0; i < length; i++) {
            chars[i] = input.readChar();
        }
        return new String(chars);
    }

    /// One inline, stored, external JAR, or extent-assembled source.
    private final class Source {
        /// Inline bytes, or null for a snapshot range or extents.
        final byte[] inline;
        /// Deferred external JAR entry, or null for other sources.
        final JarSource jar;
        /// Physical snapshot offset; negative for extents.
        final long offset;
        /// Encoded size for a snapshot range.
        final int stored;
        /// Required output sizes of successive reversed Zstd filters.
        final int[] filters;
        /// Decoded ranges from earlier sources, or null for other source kinds.
        final List<ResourcePlan.Extent> extents;
        /// Final decoded byte length.
        final int length;

        /// Copies a validated source description without an intermediate wire representation.
        Source(ResourcePlan.Source source) throws IOException {
            inline = source.inline();
            jar = source.jar();
            offset = source.offset();
            stored = source.storedLength();
            filters = source.filters();
            extents = inline == null && jar == null && offset < 0 ? source.extents() : null;
            long total = 0;
            if (extents != null) for (ResourcePlan.Extent extent : extents) total += extent.length();
            length = inline != null ? inline.length : jar != null ? size(jar.length()) : extents != null ? size(total)
                    : filters.length == 0 ? stored : filters[filters.length - 1];
        }

        /// Parses a source, checking snapshot bounds and strictly earlier extent references.
        Source(DataInputStream input, int index) throws IOException {
            jar = null;
            int kind = input.readUnsignedByte();
            if (kind == 0) {
                inline = new byte[size(input)];
                input.readFully(inline);
                offset = -1;
                stored = 0;
                filters = new int[0];
                extents = null;
                length = inline.length;
            } else if (kind == 1) {
                inline = null;
                extents = null;
                offset = input.readLong();
                stored = size(input);
                if (offset < 0 || offset > snapshotLength - stored) {
                    throw new IOException("Resource range exceeds snapshot");
                }
                filters = new int[count(input)];
                for (int i = 0; i < filters.length; i++) {
                    filters[i] = size(input);
                }
                length = filters.length == 0 ? stored : filters[filters.length - 1];
            } else if (kind == 2) {
                inline = null;
                offset = -1;
                stored = 0;
                filters = new int[0];
                int extentCount = count(input);
                List<ResourcePlan.Extent> ranges = new ArrayList<>(extentCount);
                long total = 0;
                for (int i = 0; i < extentCount; i++) {
                    int sourceIndex = nonnegative(input.readInt());
                    int offset = size(input);
                    int length = size(input);
                    if (sourceIndex >= index || sources[sourceIndex].extents != null || length == 0
                            || offset > sources[sourceIndex].length - length) {
                        throw new IOException("Invalid resource extent");
                    }
                    ranges.add(new ResourcePlan.Extent(sourceIndex, offset, length));
                    total += length;
                }
                extents = Collections.unmodifiableList(ranges);
                length = size(total);
            } else {
                throw new IOException("Unknown resource source");
            }
        }
    }

    /// Returns privately owned decoded bytes; callers must not modify them.
    private synchronized byte[] source(int id) throws IOException {
        if (closed) {
            throw new IOException("Resource reader is closed");
        }
        byte[] bytes = cache.get(id);
        if (bytes != null) {
            return bytes;
        }
        Source source = sources[id];
        if (source.inline != null) {
            return source.inline;
        }
        if (source.jar != null) {
            RandomAccessFile file = jars.get(source.jar.path());
            if (file == null) {
                file = new RandomAccessFile(source.jar.path().toFile(), "r");
                jars.put(source.jar.path(), file);
            }
            bytes = source.jar.read(file);
        } else if (source.extents != null) {
            bytes = new byte[source.length];
            int offset = 0;
            for (ResourcePlan.Extent extent : source.extents) {
                System.arraycopy(source(extent.sourceIndex()), extent.offset(), bytes, offset, extent.length());
                offset += extent.length();
            }
        } else {
            bytes = new byte[source.stored];
            snapshot.seek(source.offset);
            snapshot.readFully(bytes);
            try {
                for (int length : source.filters) {
                    ZstandardFrames.validate(bytes, limits);
                    byte[] output = new byte[length];
                    int written = Zstandard.decompress(bytes, 0, bytes.length, output, 0, output.length);
                    if (written != length) {
                        throw new IOException("Zstd decoded size mismatch");
                    }
                    bytes = output;
                }
            } catch (RuntimeException invalid) {
                throw new IOException("Invalid Zstd resource", invalid);
            }
        }
        if (bytes.length <= cacheLimit) {
            Iterator<byte[]> oldest = cache.values().iterator();
            while ((cachedBytes + bytes.length > cacheLimit || cache.size() >= 4096) && oldest.hasNext()) {
                cachedBytes -= oldest.next().length;
                oldest.remove();
            }
            cache.put(id, bytes);
            cachedBytes += bytes.length;
        }
        return bytes;
    }

    /// A single logical classpath or module root, preserving its original resource names.
    public final class Root {
        /// Original JAR filename used in resource and code-source URLs.
        final String name;
        /// Whether this root belongs to the module path rather than the classpath.
        final boolean module;
        /// Resources in Host traversal order, including explicit directories.
        final Map<String, Resource> files;

        /// Builds lookup descriptors from a validated resource plan.
        Root(ResourcePlan.Root root) {
            name = root.name();
            module = root.module();
            files = ResourceTable.mapValues(root.files(), Resource::new);
        }

        /// Reads one root and rejects duplicate names.
        Root(DataInputStream input) throws IOException {
            name = text(input);
            module = input.readBoolean();
            int count = count(input);
            String[] names = new String[count];
            Resource[] entries = new Resource[count];
            for (int i = 0; i < count; i++) {
                names[i] = text(input);
                entries[i] = new Resource(input);
            }
            try {
                files = new ResourceTable<Resource>(names, entries);
            } catch (IllegalArgumentException invalid) {
                throw new IOException("Invalid resource names", invalid);
            }
        }

        /// Returns the original JAR filename.
        public String name() {
            return name;
        }

        /// Returns an immutable map of resource names to descriptors borrowing this index.
        public Map<String, Resource> files() {
            return files;
        }

        /// Tests the lifetime of the snapshot containing this root.
        boolean isClosed() {
            return ResourceIndex.this.isClosed();
        }
    }

    /// Reads seconds and nanoseconds, rejecting invalid timestamp components or ranges.
    private static Instant readTime(DataInputStream input) throws IOException {
        long seconds = input.readLong();
        int nanos = input.readInt();
        if (nanos < 0 || nanos >= 1_000_000_000) {
            throw new IOException("Invalid resource timestamp nanoseconds");
        }
        try {
            return Instant.ofEpochSecond(seconds, nanos);
        } catch (DateTimeException invalid) {
            throw new IOException("Resource timestamp out of range", invalid);
        }
    }

    /// A directory or a logical file with its selected transform pools.
    public final class Resource {
        /// Source ID, or -1 for a directory.
        final int id;
        /// Immutable CLASSFILE steps in decoding order.
        final List<ResourcePlan.ClassFileTransform> transforms;
        /// Logical resource length, zero for directories.
        final int length;
        /// Creation instant, or null when absent.
        private final Instant creationTime;
        /// Last modification instant, or null when absent.
        private final Instant lastModifiedTime;
        /// Last access instant, or null when absent.
        private final Instant lastAccessTime;
        /// POSIX permission bits, or -1 when unspecified.
        final int permissions;

        /// Returns whether this entry is a directory.
        public boolean isDirectory() {
            return id == -1;
        }

        /// Returns the logical byte length, or zero for a directory.
        public int length() {
            return length;
        }

        /// Returns the creation instant, or null when absent.
        public Instant creationTime() {
            return creationTime;
        }

        /// Returns the last modification instant, or null when absent.
        public Instant lastModifiedTime() {
            return lastModifiedTime;
        }

        /// Returns the last access instant, or null when absent.
        public Instant lastAccessTime() {
            return lastAccessTime;
        }

        /// Returns POSIX permission bits, or -1 when unspecified.
        public int permissions() {
            return permissions;
        }

        /// Returns a read-only buffer at position zero over restored content, or an empty directory buffer.
        /// The buffer retains immutable bytes independently of later cache eviction or index closure.
        /// Reading fails with IOException if this index is closed or decoding fails.
        public java.nio.ByteBuffer buffer() throws IOException {
            return java.nio.ByteBuffer.wrap(read()).asReadOnlyBuffer();
        }

        /// Returns an owned copy of restored content, or an empty array for a directory.
        /// Reading fails with IOException if this index is closed or decoding fails.
        public byte[] readBytes() throws IOException {
            byte[] bytes = read();
            return id == -1 || !transforms.isEmpty() ? bytes : bytes.clone();
        }

        /// Copies an immutable file descriptor and derives its logical length.
        Resource(ResourcePlan.File file) {
            id = file.source();
            transforms = file.transforms();
            length = id == -1 ? 0 : transforms.isEmpty() ? sources[id].length : transforms.get(transforms.size() - 1).decodedLength();
            creationTime = file.creationTime();
            lastModifiedTime = file.lastModifiedTime();
            lastAccessTime = file.lastAccessTime();
            permissions = file.permissions() == null ? -1 : file.permissions();
        }

        /// Reads and validates a file descriptor.
        Resource(DataInputStream input) throws IOException {
            id = input.readInt();
            if (id < -1 || id >= sources.length) {
                throw new IOException("Invalid resource source ID");
            }
            int transformCount = id == -1 ? 0 : count(input);
            List<ResourcePlan.ClassFileTransform> steps = new ArrayList<>(transformCount);
            for (int i = 0; i < transformCount; i++) {
                int decodedLength = size(input);
                int poolIndex = nonnegative(input.readInt());
                if (poolIndex >= pools.length) {
                    throw new IOException("Invalid transform pool ID");
                }
                steps.add(new ResourcePlan.ClassFileTransform(decodedLength, poolIndex));
            }
            transforms = steps.isEmpty() ? Collections.emptyList() : Collections.unmodifiableList(steps);
            length = id == -1 ? 0 : transforms.isEmpty() ? sources[id].length : transforms.get(transforms.size() - 1).decodedLength();
            int flags = input.readUnsignedByte();
            if ((flags & ~15) != 0) {
                throw new IOException("Invalid resource metadata flags");
            }
            creationTime = (flags & 1) == 0 ? null : readTime(input);
            lastModifiedTime = (flags & 2) == 0 ? null : readTime(input);
            lastAccessTime = (flags & 4) == 0 ? null : readTime(input);
            permissions = (flags & 8) == 0 ? -1 : input.readInt();
            if (((flags & 8) != 0 && permissions < 0) || permissions > 4095) {
                throw new IOException("Invalid resource permissions");
            }
        }

        /// Returns privately owned logical bytes; callers must not modify them.
        byte[] read() throws IOException {
            if (isClosed()) {
                throw new IOException("Resource reader is closed");
            }
            byte[] bytes = id == -1 ? new byte[0] : source(id);
            for (ResourcePlan.ClassFileTransform transform : transforms) {
                bytes = ClassFiles.restore(bytes, pools[transform.dataPoolIndex()], transform.decodedLength(), limits);
            }
            return bytes;
        }
    }

    /// Closes container and external JAR handles and releases cached bytes; repeated calls are harmless.
    /// All handles are attempted even if one close fails; the index remains closed after failure.
    @Override
    public synchronized void close() throws IOException {
        if (!closed) {
            closed = true;
            cache.clear();
            cachedBytes = 0;
            IOException failure = null;
            try {
                snapshot.close();
            } catch (IOException close) {
                failure = close;
            }
            for (RandomAccessFile file : jars.values()) {
                try {
                    file.close();
                } catch (IOException close) {
                    if (failure == null) failure = close;
                    else failure.addSuppressed(close);
                }
            }
            jars.clear();
            if (failure != null) throw failure;
        }
    }

    /// Returns whether the owning resource snapshot has closed.
    public synchronized boolean isClosed() {
        return closed;
    }
}

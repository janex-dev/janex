// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.*;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.*;

import static org.janex.format.Input.*;

/// Reads a Janex 0.1 snapshot independently of the native Host.
///
/// This reader requires SHA-256 or SHA-512 metadata and complete section integrity coverage.
/// It does not establish publisher trust and rejects signed packages, external dictionaries,
/// and unsupported launch requirements. Ordinary resource payloads remain in the snapshot.
/// The caller must keep the snapshot unchanged until the launched application exits.
/// Instances are not thread-safe. Launch selection is single-use; closure is idempotent.
public final class JanexReader implements Closeable {
    /// Owned seekable snapshot handle.
    private final RandomAccessFile file;
    /// Snapshot path encoded into the private resource index.
    private final Path path;
    /// Caller-supplied Zstandard decoder, shared with the runtime resource reader.
    private final BlobDecoder decoder;
    /// Optional acquisition policy for external JARs, never owned or closed by this reader.
    private final DependencyResolver resolver;
    /// Pools indexed by their opaque section IDs.
    private final Map<Long, Pool> pools = new LinkedHashMap<Long, Pool>();
    /// Application descriptors paired with their type information.
    private final List<Map<Object, Object>[]> applications = new ArrayList<Map<Object, Object>[]>();
    /// Sources in dependency order, shared by all selected roots.
    private final List<Source> sources = new ArrayList<Source>();
    /// Decoded string pools in private-index order.
    private final List<String[]> strings = new ArrayList<String[]>();
    /// Interned pool references.
    private final Map<List<Long>, Integer> stringIds = new HashMap<List<Long>, Integer>();
    /// Aggregate logical resource size selected for this launch.
    private long logicalBytes;
    /// Whether launch selection has started, including a failed attempt.
    private boolean selected;
    /// Whether the snapshot handle has been closed.
    private boolean closed;

    /// Opens and checks one immutable snapshot, closing the handle if validation fails.
    ///
    /// @param path local snapshot whose lifetime is controlled by the caller
    /// @param decoder Zstandard decoder producing exactly the requested number of bytes
    /// @throws IOException if framing, recorded integrity, or the supported profile is invalid
    public JanexReader(Path path, BlobDecoder decoder) throws IOException {
        this(path, decoder, null);
    }

    /// Opens a snapshot with a caller-supplied external dependency policy.
    ///
    /// @param path local snapshot kept unchanged until the application exits
    /// @param decoder decoder producing exactly the requested number of bytes
    /// @param resolver resolver called only for selected external entries, or null to reject them
    /// @throws IOException if opening or validating the snapshot fails
    public JanexReader(Path path, BlobDecoder decoder, DependencyResolver resolver) throws IOException {
        this.path = path;
        this.decoder = Objects.requireNonNull(decoder);
        this.resolver = resolver;
        file = new RandomAccessFile(path.toFile(), "r");
        try {
            container();
        } catch (Throwable failure) {
            try {
                file.close();
            } catch (IOException close) {
                failure.addSuppressed(close);
            }
            throw failure;
        }
    }

    /// Copies an exact physical range after checking file bounds and buffer limits.
    private byte[] read(long offset, long length) throws IOException {
        int size = size(length);
        require(offset >= 0 && offset <= file.length() - size, "Janex range exceeds snapshot");
        byte[] result = new byte[size];
        file.seek(offset);
        file.readFully(result);
        return result;
    }

    /// Locates the footer before an ordinary JAR tail, or at the physical end.
    private long boundary() throws IOException {
        long length = file.length();
        Set<Long> candidates = new LinkedHashSet<Long>();
        candidates.add(length);
        long start = Math.max(0, length - 65557);
        byte[] tail = read(start, length - start);
        for (int i = 0; i <= tail.length - 22; i++) {
            if (tail[i] != 'P' || tail[i + 1] != 'K' || tail[i + 2] != 5 || tail[i + 3] != 6) {
                continue;
            }
            Input end = new Input(Arrays.copyOfRange(tail, i + 4, tail.length));
            long disk = end.little(2);
            long directoryDisk = end.little(2);
            long diskEntries = end.little(2);
            long entries = end.little(2);
            long size = end.little(4);
            long offset = end.little(4);
            long comment = end.little(2);
            if (i + 22 + comment != tail.length || disk != 0 || directoryDisk != 0 || entries != diskEntries
                    || entries == 65535 || size == 0xffffffffL || offset == 0xffffffffL) {
                continue;
            }
            long jar = start + i - size - offset;
            if (jar >= 0 && size > 0 && offset > 0
                    && Arrays.equals(read(jar, 4), new byte[]{'P', 'K', 3, 4})
                    && Arrays.equals(read(jar + offset, 4), new byte[]{'P', 'K', 1, 2})) {
                candidates.add(jar);
            }
        }
        long result = -1;
        for (long end : candidates) {
            if (end >= 24 && Arrays.equals(read(end - 24, 8), "JANEXEND".getBytes(StandardCharsets.US_ASCII))) {
                require(result == -1, "Ambiguous Janex boundary");
                result = end;
            }
        }
        require(result >= 0, "No Janex footer before an ordinary JAR tail");
        return result;
    }

    /// Parses framing and verifies metadata, every section, and both external regions.
    @SuppressWarnings("unchecked")
    private void container() throws IOException {
        long end = boundary();
        Input footer = new Input(read(end - 16, 16));
        long metadataLength = footer.little(8);
        long length = footer.little(8);
        require(length >= 32 && metadataLength >= 24 && metadataLength <= length - 8 && length <= end, "Invalid Janex footer lengths");
        long start = end - length;
        require(Arrays.equals(read(start, 8), new byte[]{'J', 'A', 'N', 'E', 'X', 0, 0, 0}), "Invalid Janex magic");
        Input metadata = new Input(read(end - metadataLength, metadataLength - 24));
        require(Arrays.equals(metadata.take(8), "METADATA".getBytes(StandardCharsets.US_ASCII)), "Invalid metadata magic");
        require(metadata.little(4) == 0 && metadata.little(4) == 1, "Unsupported Janex version");
        Map<Object, Object> values = metadata.map();
        int verification = metadata.u8();
        byte[] input = Arrays.copyOf(metadata.bytes, metadata.position);
        require(verification == 1, "Standalone Java launch requires Checksum verification; use Janex Host for signed packages");
        verify(metadata.sized(), input);
        metadata.end();
        region(get(values, 1), 0, start);
        region(get(values, 2), end, file.length() - end);
        long offset = start + 8;
        Set<Long> ids = new HashSet<Long>();
        Set<String> applicationIds = new HashSet<String>();
        for (Object item : list(get(values, 0))) {
            Map<Object, Object> section = integers(map(item));
            long type = number(get(section, 0));
            long id = number(get(section, 1));
            long size = number(get(section, 2));
            require(size >= 0 && offset <= end - metadataLength && size <= end - metadataLength - offset,
                    "Section exceeds Janex body");
            require(ids.add(id), "Duplicate section ID");
            verifyRange(binary(get(section, 3)), offset, size);
            if (type == 0x4c4f4f50424f4c42L || type == 0x50504158454e414aL) {
                require(size >= 8 && new Input(read(offset, 8)).little(8) == type, "Incorrect section magic");
                Map<Object, Object> info = integers(map(get(section, 4)));
                if (type == 0x4c4f4f50424f4c42L) {
                    pools.put(id, new Pool(offset + 8, size - 8, info));
                } else {
                    Input body = new Input(read(offset + 8, size - 8));
                    require(applicationIds.add(Conditions.nonempty(get(info, 0))), "Duplicate application ID");
                    Conditions.nonempty(get(info, 1));
                    applications.add(new Map[]{info, integers(body.map())});
                    body.end();
                }
            }
            offset += size;
        }
        require(offset == end - metadataLength, "Section lengths do not cover Janex body");
    }

    /// Requires exact external-region lengths and secure checksums for nonempty regions.
    private void region(Object value, long offset, long length) throws IOException {
        Map<Object, Object> region = integers(map(value));
        require(number(get(region, 0)) == length, "External region length mismatch");
        if (length != 0 || has(region, 1)) {
            verifyRange(binary(get(region, 1)), offset, length);
        }
    }

    /// Verifies exact stored bytes using a fixed-size buffer.
    private void verifyRange(byte[] checksum, long offset, long length) throws IOException {
        require(checksum.length > 0, "Empty checksum");
        String algorithm = checksum[0] == 0x21 ? "SHA-256" : checksum[0] == 0x22 ? "SHA-512" : null;
        require(algorithm != null, "Standalone verification requires SHA-256 or SHA-512");
        require(offset >= 0 && length >= 0 && offset <= file.length() - length, "Checksum range exceeds file");
        try {
            MessageDigest digest = MessageDigest.getInstance(algorithm);
            file.seek(offset);
            byte[] buffer = new byte[32768];
            while (length != 0) {
                int count = (int) Math.min(length, buffer.length);
                file.readFully(buffer, 0, count);
                digest.update(buffer, 0, count);
                length -= count;
            }
            require(MessageDigest.isEqual(digest.digest(), Arrays.copyOfRange(checksum, 1, checksum.length)), "Janex checksum mismatch");
        } catch (NoSuchAlgorithmException failure) {
            throw new IOException("Required digest is unavailable", failure);
        }
    }

    /// Verifies a supported secure checksum over exact bytes.
    private static void verify(byte[] checksum, byte[] bytes) throws IOException {
        require(checksum.length > 0, "Empty checksum");
        String algorithm = checksum[0] == 0x21 ? "SHA-256" : checksum[0] == 0x22 ? "SHA-512" : null;
        require(algorithm != null, "Standalone verification requires SHA-256 or SHA-512");
        try {
            byte[] actual = MessageDigest.getInstance(algorithm).digest(bytes);
            require(MessageDigest.isEqual(actual, Arrays.copyOfRange(checksum, 1, checksum.length)), "Janex checksum mismatch");
        } catch (NoSuchAlgorithmException failure) {
            throw new IOException("Required digest is unavailable", failure);
        }
    }

    /// Describes an independently decoded range and its reversed filter output lengths.
    private final class Encoding {
        /// Number of stored bytes.
        final int stored;
        /// Filter output sizes in decoding order.
        final int[] filters;

        /// Reads the binary BlobEncoding schema, rejecting unsupported methods and dictionaries.
        Encoding(Input input) throws IOException {
            stored = size(input.uint());
            filters = new int[count(input.uint())];
            for (int i = filters.length - 1; i >= 0; i--) {
                filters[i] = size(input.uint());
                require(input.u8() == 1, "Unsupported blob filter");
                require(!has(integers(input.map()), 0), "External Zstd dictionaries require Janex Host");
            }
        }

        /// Returns the final decoded length.
        int length() {
            return filters.length == 0 ? stored : filters[filters.length - 1];
        }

        /// Decodes all filter stages with exact output-size checks.
        byte[] decode(byte[] bytes) throws IOException {
            for (int length : filters) {
                try {
                    bytes = decoder.decode(bytes, length);
                    require(bytes.length == length, "Zstd decoded length mismatch");
                } catch (RuntimeException failure) {
                    throw new IOException("Invalid Zstd blob", failure);
                }
            }
            return bytes;
        }
    }

    /// One pool's page directory and lazily registered entries.
    private final class Pool {
        /// Physical start of pool bytes after the magic.
        final long start;
        /// Encoded pool byte length.
        final long length;
        /// Page entry shift.
        final int shift;
        /// Blob count.
        final int count;
        /// Page descriptors in index order.
        final List<Object> pages;
        /// Decoded entry payloads, indexed by blob ID.
        final Map<Integer, Input> entries = new HashMap<Integer, Input>();
        /// Registered source IDs, indexed by blob ID.
        final Map<Integer, Integer> sourceIds = new HashMap<Integer, Integer>();
        /// Registered physical ranges, used to reject overlap.
        final TreeMap<Long, Long> ranges = new TreeMap<Long, Long>();

        /// Validates the page directory without reading stored resource data.
        Pool(long start, long length, Map<Object, Object> info) throws IOException {
            this.start = start;
            this.length = length;
            count = count(number(get(info, 0)));
            long shiftValue = number(get(info, 1));
            require(shiftValue >= 8 && shiftValue <= 12, "Invalid page entry shift");
            shift = (int) shiftValue;
            pages = list(get(info, 2));
            require(pages.size() == (count == 0 ? 0 : 1 + ((count - 1) >> shift)), "Incorrect page count");
            for (Object page : pages) {
                List<Object> fields = list(page);
                require(fields.size() == 2 || fields.size() == 3, "Invalid page descriptor");
                Input bytes = new Input(binary(fields.get(1)));
                Encoding encoding = new Encoding(bytes);
                bytes.end();
                range(number(fields.get(0)), encoding.stored);
            }
        }

        /// Registers a nonoverlapping physical range relative to this pool.
        void range(long offset, long size) throws IOException {
            require(offset >= 0 && size >= 0 && offset <= length - size, "Blob range exceeds pool");
            if (size == 0) {
                return;
            }
            Map.Entry<Long, Long> before = ranges.floorEntry(offset);
            Map.Entry<Long, Long> after = ranges.ceilingEntry(offset);
            require((before == null || before.getValue() <= offset) && (after == null || offset + size <= after.getKey()),
                    "Overlapping blob ranges");
            ranges.put(offset, offset + size);
        }

        /// Loads the containing page and returns a fresh entry cursor.
        Input entry(int index) throws IOException {
            require(index >= 0 && index < count, "Blob index exceeds pool");
            if (!entries.containsKey(index)) {
                int first = (index >> shift) << shift;
                List<Object> fields = list(pages.get(index >> shift));
                Input encoded = new Input(binary(fields.get(1)));
                Encoding encoding = new Encoding(encoded);
                byte[] decoded = encoding.decode(read(start + number(fields.get(0)), encoding.stored));
                if (fields.size() == 3) {
                    verify(binary(fields.get(2)), decoded);
                }
                Input page = new Input(decoded);
                for (int i = first; i < Math.min(count, first + (1 << shift)); i++) {
                    int tag = page.u8();
                    byte[] payload = page.sized();
                    byte[] record = new byte[payload.length + 1];
                    record[0] = (byte) tag;
                    System.arraycopy(payload, 0, record, 1, payload.length);
                    Input entry = new Input(record);
                    if (tag == 0) {
                        entry.u8();
                        long offset = entry.uint();
                        Encoding stored = new Encoding(entry);
                        range(offset, stored.stored);
                        entry.end();
                    }
                    entries.put(i, new Input(record));
                }
                page.end();
            }
            return new Input(entries.get(index).bytes);
        }

        /// Registers a source in dependency order, permitting only stored extent targets.
        int source(int index, boolean storedOnly) throws IOException {
            Input entry = entry(index);
            int kind = entry.u8();
            require(!storedOnly || kind == 0, "Extent target must be stored");
            Integer existing = sourceIds.get(index);
            if (existing != null) {
                return existing;
            }
            Source result = new Source();
            if (kind == 0) {
                result.offset = start + entry.uint();
                result.encoding = new Encoding(entry);
                result.length = result.encoding.length();
            } else {
                require(kind == 1, "Unsupported blob entry");
                result.extents = new int[count(entry.uint())][3];
                require(result.extents.length != 0, "Empty extents");
                long total = 0;
                for (int[] extent : result.extents) {
                    extent[0] = source(count(entry.uint()), true);
                    extent[1] = size(entry.uint());
                    extent[2] = size(entry.uint());
                    require(extent[2] > 0 && extent[1] <= sources.get(extent[0]).length - extent[2], "Invalid extent range");
                    total += extent[2];
                }
                result.length = size(total);
            }
            entry.end();
            int id = add(result);
            sourceIds.put(index, id);
            return id;
        }
    }

    /// One source shared by private-index files and structural decoding.
    private static final class Source {
        /// Inline bytes, or null for a stored or extents source.
        byte[] inline;
        /// Physical stored offset.
        long offset;
        /// Stored encoding, or null for inline and extents sources.
        Encoding encoding;
        /// Extent triples, or null for inline and stored sources.
        int[][] extents;
        /// Decoded byte length.
        int length;
    }

    /// Appends a bounded source descriptor.
    private int add(Source source) throws IOException {
        count(sources.size() + 1L);
        int index = sources.size();
        sources.add(source);
        return index;
    }

    /// Registers owned inline bytes.
    private int inline(byte[] bytes) throws IOException {
        Source source = new Source();
        source.inline = bytes;
        source.length = bytes.length;
        return add(source);
    }

    /// Resolves a binary BlobRef to a source ID.
    private int reference(Input input) throws IOException {
        return reference(input.uint(), input.uint());
    }

    /// Resolves a CBOR BlobRef to a source ID.
    private int reference(Object value) throws IOException {
        List<Object> ref = list(value);
        require(ref.size() == 2, "Invalid BlobRef");
        return reference(number(ref.get(0)), number(ref.get(1)));
    }

    /// Checks section identity and resolves one blob source.
    private int reference(long pool, long index) throws IOException {
        require(pools.containsKey(pool), "Unknown BlobPool section");
        return pools.get(pool).source(count(index), false);
    }

    /// Reads a complete logical blob; ordinary file data is instead read lazily by ResourceIndex.
    private byte[] bytes(int index) throws IOException {
        Source source = sources.get(index);
        if (source.inline != null) {
            return source.inline;
        }
        if (source.encoding != null) {
            return source.encoding.decode(read(source.offset, source.encoding.stored));
        }
        byte[] result = new byte[source.length];
        int offset = 0;
        for (int[] extent : source.extents) {
            System.arraycopy(bytes(extent[0]), extent[1], result, offset, extent[2]);
            offset += extent[2];
        }
        return result;
    }

    /// Resolves and interns a complete string pool.
    private int stringPool(long pool, long index) throws IOException {
        List<Long> key = Arrays.asList(pool, index);
        Integer previous = stringIds.get(key);
        if (previous != null) {
            return previous;
        }
        Input input = new Input(bytes(reference(pool, index)));
        String[] values = new String[count(input.uint())];
        Set<String> unique = new HashSet<String>();
        for (int i = 0; i < values.length; i++) {
            values[i] = input.string();
            require(unique.add(values[i]), "Duplicate string-pool entry");
        }
        input.end();
        require(values.length != 0 && values[0].isEmpty(), "String pool must start with empty string");
        int id = strings.size();
        strings.add(values);
        stringIds.put(key, id);
        return id;
    }

    /// Looks up a root string-pool entry.
    private String string(int pool, long index) throws IOException {
        String[] values = strings.get(pool);
        require(index >= 0 && index < values.length, "Invalid string-pool index");
        return values[(int) index];
    }

    /// Reads a nonempty indexed, inline, or concatenated string.
    private String name(Input input, int pool) throws IOException {
        long index = input.uint();
        if (index != 0) {
            return string(pool, index);
        }
        String value = input.string();
        if (!value.isEmpty()) {
            return value;
        }
        int count = count(input.uint());
        require(count >= 2, "Concatenation needs at least two strings");
        StringBuilder result = new StringBuilder();
        for (int i = 0; i < count; i++) {
            result.append(string(pool, input.uint()));
            size(result.length() * 2L);
        }
        require(result.length() != 0, "Empty resource name");
        return result.toString();
    }

    /// One merged directory, file, link, or tombstone.
    private static final class Node {
        /// Source ID, or -1 for a directory, -2 for a link, and -3 for a tombstone.
        int source = -1;
        /// Reversed transform output-size and string-pool pairs.
        int[][] transforms = new int[0][2];
        /// Relative symbolic-link target, when present.
        String target;
        /// Exact resource metadata.
        Map<Object, Object> metadata = Collections.emptyMap();
    }

    /// Reads content descriptors and resolves explicit transform pools.
    private Node content(Input input, int pool, boolean directory) throws IOException {
        Node node = new Node();
        int kind = input.u8();
        require(kind == 0 || kind == 1, "Unsupported content source");
        node.source = kind == 0 ? inline(input.sized()) : reference(input);
        node.transforms = new int[count(input.uint())][2];
        require(!directory || node.transforms.length == 0, "Directory content cannot have transforms");
        for (int i = node.transforms.length - 1; i >= 0; i--) {
            node.transforms[i][0] = size(input.uint());
            require(input.u8() == 1, "Unsupported content transform");
            Map<Object, Object> properties = integers(input.map());
            if (has(properties, 0)) {
                List<Object> ref = list(get(properties, 0));
                require(ref.size() == 2, "Invalid transform pool reference");
                node.transforms[i][1] = stringPool(number(ref.get(0)), number(ref.get(1)));
            } else {
                node.transforms[i][1] = pool;
            }
        }
        return node;
    }

    /// Requires a normalized root-relative resource path or one valid entry component.
    private static void path(String path, boolean component) throws IOException {
        require(!component || !path.isEmpty(), "Empty entry name");
        if (path.isEmpty()) {
            return;
        }
        String[] parts = path.split("/", -1);
        require(!component || parts.length == 1, "Entry name contains slash");
        for (String part : parts) {
            require(!part.isEmpty() && !part.equals(".") && !part.equals(".."), "Invalid resource path");
        }
    }

    /// Returns a child path without adding a leading slash.
    private static String join(String parent, String name) {
        return parent.isEmpty() ? name : parent + "/" + name;
    }

    /// Inserts missing directory parents while rejecting file/directory conflicts.
    private static void directory(Map<String, Node> tree, String path) throws IOException {
        for (String current = path;;) {
            Node existing = tree.get(current);
            require(existing == null || existing.source == -1, "Resource file/directory conflict");
            if (existing == null) {
                tree.put(current, new Node());
            }
            int slash = current.lastIndexOf('/');
            if (slash < 0) {
                break;
            }
            current = current.substring(0, slash);
        }
    }

    /// Reads and merges all layers, validating unmatched layers as well.
    private Root root(Object reference, boolean module) throws IOException {
        Input input = new Input(bytes(reference(reference)));
        int pool = stringPool(input.uint(), input.uint());
        Map<Object, Object> metadata = input.map();
        for (Object key : metadata.keySet()) {
            Conditions.nonempty(key);
        }
        String jarName = metadata.containsKey("janex.java.jar_name") ? text(metadata.get("janex.java.jar_name")) : "resources.jar";
        require(jarName.endsWith(".jar") && jarName.indexOf('/') < 0 && jarName.indexOf('\\') < 0 && jarName.indexOf(0) < 0,
                "Invalid root JAR filename");
        Map<String, Node> tree = new TreeMap<String, Node>();
        tree.put("", new Node());
        int layers = count(input.uint());
        for (int layer = 0; layer < layers; layer++) {
            boolean matches = Conditions.matches(input.map());
            Map<String, Node> records = new LinkedHashMap<String, Node>();
            Set<String> tombstones = new LinkedHashSet<String>();
            Set<String> directories = new HashSet<String>();
            String previousDirectory = null;
            int directoryCount = count(input.uint());
            for (int d = 0; d < directoryCount; d++) {
                String dir = string(pool, input.uint());
                path(dir, false);
                require(previousDirectory == null || compare(previousDirectory.getBytes(StandardCharsets.UTF_8), dir.getBytes(StandardCharsets.UTF_8)) < 0,
                        "Directories are not sorted and unique");
                previousDirectory = dir;
                Node directory = new Node();
                directory.metadata = resourceMetadata(input.map(), -1);
                require(records.put(dir, directory) == null, "Directory conflicts with entry");
                directories.add(dir);
                int entries = count(input.uint());
                Node content = content(input, pool, true);
                Input data = new Input(bytes(content.source));
                String previousName = null;
                for (int e = 0; e < entries; e++) {
                    long type = data.little(4);
                    String name = name(data, pool);
                    path(name, true);
                    require(previousName == null || compare(previousName.getBytes(StandardCharsets.UTF_8), name.getBytes(StandardCharsets.UTF_8)) < 0,
                            "Directory entries are not sorted and unique");
                    previousName = name;
                    Node node;
                    if (type == 0x00534552) {
                        node = content(data, pool, false);
                        node.metadata = resourceMetadata(data.map(), node.source);
                    } else {
                        node = new Node();
                        require(type == 0x4c4d5953 || type == 0x424d4f54, "Unsupported directory entry");
                        node.source = type == 0x4c4d5953 ? -2 : -3;
                        if (node.source == -2) {
                            node.target = name(data, pool);
                            for (String component : node.target.split("/", -1)) {
                                require(!component.isEmpty(), "Invalid symbolic-link target");
                            }
                            node.metadata = resourceMetadata(data.map(), -2);
                        }
                    }
                    if (node.source == -3) {
                        tombstones.add(join(dir, name));
                    } else {
                        require(records.put(join(dir, name), node) == null, "Conflicting resource paths");
                    }
                }
                data.end();
            }
            // Validate implicit parents within the layer independently of its condition.
            for (String dir : directories) {
                for (String current = dir; !current.isEmpty();) {
                    int slash = current.lastIndexOf('/');
                    current = slash < 0 ? "" : current.substring(0, slash);
                    Node parent = records.get(current);
                    require(parent == null || parent.source == -1, "Conflicting implicit directory");
                }
            }
            if (matches) {
                for (String name : tombstones) {
                    Node old = tree.get(name);
                    if (old != null && old.source != -1) {
                        tree.remove(name);
                    }
                }
                for (Map.Entry<String, Node> record : records.entrySet()) {
                    String name = record.getKey();
                    if (record.getValue().source == -1) {
                        directory(tree, name);
                    } else {
                        int slash = name.lastIndexOf('/');
                        directory(tree, slash < 0 ? "" : name.substring(0, slash));
                        require(!tree.containsKey(name) || tree.get(name).source != -1, "Entry conflicts with directory");
                    }
                    tree.put(name, record.getValue());
                    count(tree.size());
                }
            }
        }
        input.end();
        return finishRoot(jarName, module, tree);
    }

    /// Rewrites manifests and expands aliases for an imported or embedded root.
    private Root finishRoot(String jarName, boolean module, Map<String, Node> tree) throws IOException {
        runtimeManifests(tree);
        Map<String, List<String>> children = new HashMap<String, List<String>>();
        for (String name : tree.keySet()) {
            if (!name.isEmpty()) {
                int slash = name.lastIndexOf('/');
                String parent = slash < 0 ? "" : name.substring(0, slash);
                children.computeIfAbsent(parent, ignored -> new ArrayList<String>()).add(name);
            }
        }
        Map<String, Node> expanded = new LinkedHashMap<String, Node>();
        expand(tree, children, "", "", expanded, new HashSet<String>());
        return new Root(jarName, module, expanded);
    }

    /// Removes implicit JAR paths and stale signature attributes, matching native launch preparation.
    private void runtimeManifests(Map<String, Node> tree) throws IOException {
        for (Map.Entry<String, Node> entry : tree.entrySet()) {
            Node node = entry.getValue();
            if (entry.getKey().equalsIgnoreCase("META-INF/MANIFEST.MF") && node.source >= 0) {
                require(node.transforms.length == 0, "Manifest cannot contain class-file transforms");
                java.util.jar.Manifest manifest = new java.util.jar.Manifest(new ByteArrayInputStream(bytes(node.source)));
                java.util.jar.Attributes main = manifest.getMainAttributes();
                main.remove(java.util.jar.Attributes.Name.CLASS_PATH);
                if (main.getValue(java.util.jar.Attributes.Name.MANIFEST_VERSION) == null) {
                    main.put(java.util.jar.Attributes.Name.MANIFEST_VERSION, "1.0");
                }
                List<java.util.jar.Attributes> attributes = new ArrayList<java.util.jar.Attributes>(manifest.getEntries().values());
                attributes.add(main);
                for (java.util.jar.Attributes section : attributes) {
                    Iterator<Object> keys = section.keySet().iterator();
                    while (keys.hasNext()) {
                        String key = keys.next().toString().toLowerCase(Locale.ROOT);
                        if (key.equals("signature-version") || key.equals("magic") || key.endsWith("-digest") || key.contains("-digest-")) {
                            keys.remove();
                        }
                    }
                }
                ByteArrayOutputStream output = new ByteArrayOutputStream();
                manifest.write(output);
                node.source = inline(output.toByteArray());
            }
        }
    }

    /// Checks resource metadata, preserving exact timestamps and mode zero.
    private static Map<Object, Object> resourceMetadata(Map<Object, Object> metadata, int kind) throws IOException {
        integers(metadata);
        if (has(metadata, 0)) {
            require(kind >= 0, "Checksum is only valid for files");
            byte[] checksum = binary(get(metadata, 0));
            require(checksum.length > 0, "Empty file checksum");
            int length;
            switch (checksum[0]) {
                case 0x11:
                    length = 8;
                    break;
                case 0x12:
                    length = 16;
                    break;
                case 0x21:
                case 0x31:
                    length = 32;
                    break;
                case 0x22:
                    length = 64;
                    break;
                default:
                    throw new IOException("Unsupported file checksum algorithm");
            }
            require(checksum.length == length + 1, "Invalid file checksum length");
        }
        if (has(metadata, 1)) {
            text(get(metadata, 1));
        }
        for (int key = 2; key <= 4; key++) {
            if (has(metadata, key)) {
                Object value = get(metadata, key);
                if (value instanceof Opaque) {
                    Opaque tag = (Opaque) value;
                    require(BigInteger.valueOf(2).equals(tag.type) || BigInteger.valueOf(3).equals(tag.type), "Invalid timestamp tag");
                    byte[] magnitude = binary(tag.value);
                    require(magnitude.length > 8 && magnitude.length <= 16 && magnitude[0] != 0, "Invalid timestamp bignum");
                    BigInteger integer = new BigInteger(1, magnitude);
                    value = BigInteger.valueOf(2).equals(tag.type) ? integer : integer.negate().subtract(BigInteger.ONE);
                    metadata.put(BigInteger.valueOf(key), value);
                }
                require(value instanceof BigInteger && ((BigInteger) value).bitLength() <= 127, "Invalid resource timestamp");
            }
        }
        if (has(metadata, 5)) {
            long mode = number(get(metadata, 5));
            require(kind >= -1 && mode >= 0 && mode <= 4095, "Invalid resource permissions");
        }
        return metadata;
    }

    /// Resolves links component by component without escaping the resource root.
    private static String resolve(Map<String, Node> tree, String path, Set<String> links) throws IOException {
        String current = "";
        for (String component : path.split("/", -1)) {
            Node parent = tree.get(current);
            require(parent != null && parent.source == -1, "Symbolic link traverses a file");
            if (component.equals(".")) {
                continue;
            }
            if (component.equals("..")) {
                require(!current.isEmpty(), "Symbolic link escapes resource root");
                int slash = current.lastIndexOf('/');
                current = slash < 0 ? "" : current.substring(0, slash);
                continue;
            }
            if (component.isEmpty() && path.isEmpty()) {
                continue;
            }
            String next = join(current, component);
            Node node = tree.get(next);
            require(node != null, "Dangling symbolic link");
            if (node.source == -2) {
                require(links.size() < 64 && links.add(next), "Symbolic-link cycle");
                current = resolve(tree, join(current, node.target), links);
                links.remove(next);
            } else {
                current = next;
            }
        }
        return current;
    }

    /// Expands directory aliases into the shared resource-index representation.
    private void expand(Map<String, Node> tree, Map<String, List<String>> children, String canonical, String alias,
                        Map<String, Node> output, Set<String> active) throws IOException {
        require(active.size() < 256, "Resource directory depth limit exceeded");
        Node node = tree.get(canonical);
        if (node.source == -2) {
            canonical = resolve(tree, canonical, new HashSet<String>());
            node = tree.get(canonical);
        }
        require(output.put(alias, node) == null, "Duplicate expanded resource");
        count(output.size());
        if (node.source >= 0) {
            logicalBytes += node.transforms.length == 0 ? sources.get(node.source).length : node.transforms[node.transforms.length - 1][0];
            size(logicalBytes);
            return;
        }
        require(active.add(canonical), "Directory-link cycle");
        String prefix = canonical.isEmpty() ? "" : canonical + "/";
        for (String child : children.getOrDefault(canonical, Collections.emptyList())) {
            String name = child.substring(prefix.length());
            expand(tree, children, child, join(alias, name), output, active);
        }
        active.remove(canonical);
    }

    /// One selected root in classpath or module-path order.
    private static final class Root {
        /// Original JAR filename.
        final String name;
        /// Whether this root belongs to the module path.
        final boolean module;
        /// Fully merged and expanded resource paths.
        final Map<String, Node> files;

        /// Retains one selected resource tree.
        Root(String name, boolean module, Map<String, Node> files) {
            this.name = name;
            this.module = module;
            this.files = files;
        }
    }

    /// Complete launch data selected for the current runtime.
    public static final class Launch {
        /// Binary class name, or empty when supplied by a module.
        public String mainClass = "";
        /// Main module, or empty for a classpath launch.
        public String mainModule = "";
        /// Preset arguments before user arguments.
        public final List<String> arguments = new ArrayList<String>();
        /// JVM options in application order.
        public final List<String> options = new ArrayList<String>();
        /// Host-compatible resource index referencing the snapshot.
        public byte[] resources;
        /// Selected physical and external classpath entries.
        private final List<Object> classPath = new ArrayList<Object>();
        /// Selected physical and external module-path entries.
        private final List<Object> modulePath = new ArrayList<Object>();
        /// Selected agents, checked after overlays are applied.
        private final List<Object> agents = new ArrayList<Object>();
        /// Whether an entry point remains selected.
        private boolean entryPoint;

        /// Creates an initially empty launch selection.
        private Launch() {
        }
    }

    /// Selects an application and creates the existing private resource-index representation.
    ///
    /// This method may be called once, including calls that fail. Returned data is caller-owned.
    /// The snapshot must remain unchanged and present while the returned index is used.
    ///
    /// @param application explicit application ID, or null to require exactly one application
    /// @return owned launch data for the current Java runtime
    /// @throws IOException if closed, already selected, or selection, parsing, or a launch requirement fails
    public Launch launch(String application) throws IOException {
        require(!closed && !selected, "Janex reader is closed or already selected");
        selected = true;
        Map<Object, Object>[] selected = null;
        for (Map<Object, Object>[] candidate : applications) {
            if (application == null || application.equals(text(get(candidate[0], 0)))) {
                require(selected == null, "Multiple applications; set -Djanex.application=ID");
                selected = candidate;
            }
        }
        require(selected != null, "No matching Janex application");
        require(text(get(selected[0], 1)).equals("janex.java"), "Unsupported application type");
        Map<Object, Object> descriptor = integers(map(get(selected[1], 0)));
        Map<Object, Object> configuration = integers(map(get(descriptor, 0)));
        Launch launch = new Launch();
        overlay(configuration, launch, true, 0);
        require(launch.entryPoint, "No entry point for the current Java runtime");
        require(launch.agents.isEmpty(), "Standalone Java agents are not yet supported; use Janex Host");
        List<Root> roots = new ArrayList<Root>();
        for (Object entry : launch.classPath) {
            roots.add(pathEntry(entry, false));
        }
        for (Object entry : launch.modulePath) {
            roots.add(pathEntry(entry, true));
        }
        ByteArrayOutputStream buffer = new ByteArrayOutputStream();
        DataOutputStream output = new DataOutputStream(buffer);
        output.writeBytes("JNXRES01");
        output.writeInt(MAX_BYTES);
        output.writeInt(MAX_ELEMENTS);
        string(output, path.toString());
        output.writeInt(sources.size());
        for (Source source : sources) {
            if (source.inline != null) {
                output.writeByte(0);
                output.writeInt(source.inline.length);
                output.write(source.inline);
            } else if (source.encoding != null) {
                output.writeByte(1);
                output.writeLong(source.offset);
                output.writeInt(source.encoding.stored);
                output.writeInt(source.encoding.filters.length);
                for (int length : source.encoding.filters) {
                    output.writeInt(length);
                }
            } else {
                output.writeByte(2);
                output.writeInt(source.extents.length);
                for (int[] extent : source.extents) {
                    for (int value : extent) {
                        output.writeInt(value);
                    }
                }
            }
            size(buffer.size());
        }
        output.writeInt(strings.size());
        for (String[] pool : strings) {
            output.writeInt(pool.length);
            for (String value : pool) {
                string(output, value);
            }
            size(buffer.size());
        }
        output.writeInt(0);
        output.writeInt(roots.size());
        for (Root root : roots) {
            string(output, root.name);
            output.writeBoolean(root.module);
            output.writeInt(root.files.size());
            for (Map.Entry<String, Node> entry : root.files.entrySet()) {
                string(output, entry.getKey());
                Node node = entry.getValue();
                output.writeInt(node.source);
                if (node.source >= 0) {
                    output.writeInt(node.transforms.length);
                    for (int[] transform : node.transforms) {
                        output.writeInt(transform[0]);
                        output.writeInt(transform[1]);
                    }
                }
                int flags = 0;
                for (int key = 2; key <= 5; key++) {
                    if (has(node.metadata, key)) {
                        flags |= 1 << (key - 2);
                    }
                }
                output.writeByte(flags);
                for (int key = 2; key <= 4; key++) {
                    if (has(node.metadata, key)) {
                        BigInteger value = (BigInteger) get(node.metadata, key);
                        byte[] raw = value.toByteArray();
                        byte[] padded = new byte[16];
                        Arrays.fill(padded, value.signum() < 0 ? (byte) -1 : 0);
                        System.arraycopy(raw, 0, padded, 16 - raw.length, raw.length);
                        output.write(padded);
                    }
                }
                if (has(node.metadata, 5)) {
                    output.writeInt((int) number(get(node.metadata, 5)));
                }
                size(buffer.size());
            }
        }
        launch.resources = buffer.toByteArray();
        return launch;
    }

    /// Resolves an embedded root or imports a caller-acquired external JAR.
    private Root pathEntry(Object value, boolean module) throws IOException {
        Map<Object, Object> entry = integers(map(value));
        long kind = number(get(entry, 0));
        if (kind == 0) {
            return root(get(entry, 1), module);
        }
        require(kind == 1, "Unsupported Java path entry");
        String uri = Conditions.nonempty(get(entry, 1));
        byte[] checksum = has(entry, 2) ? binary(get(entry, 2)) : null;
        require(resolver != null, "External dependencies require a dependency resolver");
        Dependency dependency = Objects.requireNonNull(resolver.resolve(uri, checksum));
        require(dependency.jarName.endsWith(".jar") && dependency.jarName.indexOf('/') < 0
                && dependency.jarName.indexOf('\\') < 0 && dependency.jarName.indexOf(0) < 0, "Invalid dependency JAR filename");
        return jarRoot(dependency, module);
    }

    /// Imports bounded JAR entries and applies increasing multi-release layers for this runtime.
    private Root jarRoot(Dependency dependency, boolean module) throws IOException {
        List<JarArchive.Entry> entries = JarArchive.read(dependency.bytes);
        boolean multiRelease = false;
        for (JarArchive.Entry entry : entries) {
            if (entry.name.equals("META-INF/MANIFEST.MF")) {
                require(entry.kind != 0120000, "JAR manifest must be a regular file");
                java.util.jar.Manifest manifest = new java.util.jar.Manifest(new ByteArrayInputStream(entry.bytes));
                multiRelease = "true".equalsIgnoreCase(manifest.getMainAttributes().getValue("Multi-Release"));
            }
        }
        SortedMap<Integer, Map<String, Node>> layers = new TreeMap<Integer, Map<String, Node>>();
        for (JarArchive.Entry entry : entries) {
            boolean directory = entry.name.endsWith("/");
            String name = directory ? entry.name.substring(0, entry.name.length() - 1) : entry.name;
            require(!name.isEmpty() && name.indexOf(0) < 0, "Invalid JAR resource path");
            path(name, false);
            int version = 0;
            if (multiRelease && name.startsWith("META-INF/versions/")) {
                String tail = name.substring("META-INF/versions/".length());
                int slash = tail.indexOf('/');
                String number = slash < 0 ? tail : tail.substring(0, slash);
                if (number.matches("[1-9][0-9]*") && number.length() <= 10) {
                    long parsed = Long.parseLong(number);
                    if (parsed >= 9 && parsed <= Integer.MAX_VALUE) {
                        version = (int) parsed;
                        name = slash < 0 ? "" : tail.substring(slash + 1);
                        require(!name.equals("META-INF") && !name.startsWith("META-INF/"), "Multi-Release JAR cannot version META-INF");
                        require(!name.isEmpty() || directory, "Multi-Release version root is not a directory");
                    }
                }
            }
            Node node = new Node();
            if (entry.kind == 0120000) {
                node.source = -2;
                node.target = utf8(entry.bytes);
                require(node.target.indexOf(0) < 0, "Invalid JAR symbolic-link target");
                for (String component : node.target.split("/", -1)) {
                    require(!component.isEmpty(), "Invalid JAR symbolic-link target");
                }
            } else if (!directory) {
                node.source = inline(entry.bytes);
            }
            if (entry.mode >= 0 && node.source >= -1) {
                node.metadata = new LinkedHashMap<Object, Object>();
                node.metadata.put(BigInteger.valueOf(5), BigInteger.valueOf(entry.mode & 07777));
            }
            Map<String, Node> layer = layers.computeIfAbsent(version, ignored -> new TreeMap<String, Node>());
            require(layer.put(name, node) == null, "Conflicting JAR resource paths");
        }
        Map<String, Node> tree = new TreeMap<String, Node>();
        Map<String, Node> validation = new TreeMap<String, Node>();
        tree.put("", new Node());
        validation.put("", new Node());
        for (Map.Entry<Integer, Map<String, Node>> layer : layers.entrySet()) {
            mergeJarLayer(validation, layer.getValue());
            if (layer.getKey() <= feature()) {
                mergeJarLayer(tree, layer.getValue());
            }
        }
        return finishRoot(dependency.jarName, module, tree);
    }

    /// Merges one JAR layer without permitting implicit file/directory replacement.
    private static void mergeJarLayer(Map<String, Node> tree, Map<String, Node> layer) throws IOException {
        for (Map.Entry<String, Node> entry : layer.entrySet()) {
            String name = entry.getKey();
            if (entry.getValue().source == -1) {
                directory(tree, name);
            } else {
                int slash = name.lastIndexOf('/');
                directory(tree, slash < 0 ? "" : name.substring(0, slash));
                require(!tree.containsKey(name) || tree.get(name).source != -1, "JAR file/directory conflict");
            }
            tree.put(name, entry.getValue());
            count(tree.size());
        }
    }

    /// Visits all configuration nodes for validation and applies matching overlays in order.
    private void overlay(Map<Object, Object> config, Launch launch, boolean parent, int depth) throws IOException {
        require(depth <= 64, "Launch overlay depth limit exceeded");
        integers(config);
        boolean matches = has(config, 0) ? Conditions.matches(map(get(config, 0))) : true;
        matches &= parent;
        if (has(config, 1)) {
            Object value = get(config, 1);
            String mainClass = "";
            String mainModule = "";
            if (value != null) {
                Map<Object, Object> point = integers(map(value));
                mainClass = has(point, 0) ? Conditions.nonempty(get(point, 0)) : "";
                mainModule = has(point, 1) ? Conditions.nonempty(get(point, 1)) : "";
                require(!mainClass.isEmpty() || !mainModule.isEmpty(), "Empty Java entry point");
            }
            if (matches) {
                launch.entryPoint = value != null;
                launch.mainClass = mainClass;
                launch.mainModule = mainModule;
            }
        }
        append(config, 2, launch.modulePath, matches);
        append(config, 3, launch.classPath, matches);
        append(config, 4, launch.agents, matches);
        appendStrings(config, 5, launch.options, matches);
        appendStrings(config, 7, launch.arguments, matches);
        if (has(config, 6)) {
            for (Object child : list(get(config, 6))) {
                overlay(map(child), launch, matches, depth + 1);
            }
        }
    }

    /// Appends or clears a selected list while validating its outer structure.
    private static void append(Map<Object, Object> config, int key, List<Object> target, boolean matches) throws IOException {
        if (has(config, key)) {
            Object value = get(config, key);
            List<Object> items = value == null ? Collections.emptyList() : list(value);
            if (matches) {
                if (value == null) {
                    target.clear();
                } else {
                    target.addAll(items);
                    count(target.size());
                }
            }
        }
    }

    /// Appends or clears a string list without splitting argument values.
    private static void appendStrings(Map<Object, Object> config, int key, List<String> target, boolean matches) throws IOException {
        if (has(config, key)) {
            Object value = get(config, key);
            List<String> items = new ArrayList<String>();
            if (value != null) {
                for (Object item : list(value)) {
                    items.add(text(item));
                }
            }
            if (matches) {
                if (value == null) {
                    target.clear();
                } else {
                    target.addAll(items);
                    count(target.size());
                }
            }
        }
    }

    /// Writes exact UTF-16 code units in the existing private-index encoding.
    ///
    /// @param output destination stream, not closed
    /// @param value string whose code units are preserved
    /// @throws IOException if the string exceeds limits or writing fails
    public static void string(DataOutputStream output, String value) throws IOException {
        size(value.length() * 2L);
        output.writeInt(value.length());
        output.writeChars(value);
    }

    /// Returns the current Java feature version used to select launch capabilities.
    ///
    /// @return the current runtime's feature version, at least 8
    /// @throws IOException if the runtime reports an invalid or unsupported Java version
    public static int feature() throws IOException {
        return Conditions.feature();
    }

    /// Decodes a complete Zstandard input using the caller's codec implementation.
    @FunctionalInterface
    public interface BlobDecoder {
        /// Decodes all frames, consuming the complete input and checking frame integrity.
        ///
        /// @param input encoded bytes that must not be modified
        /// @param length required decoded byte length, between zero and the reader's byte limit
        /// @return a new array containing exactly the decoded bytes
        /// @throws IOException if the input is invalid or cannot produce the required output
        byte[] decode(byte[] input, int length) throws IOException;
    }

    /// Acquires an external JAR under the caller's network, cache, and integrity policy.
    @FunctionalInterface
    public interface DependencyResolver {
        /// Resolves one selected external path entry; failure aborts launch preparation.
        ///
        /// @param uri external URI from the verified application descriptor
        /// @param checksum encoded algorithm byte and digest, or null when absent; must not be modified
        /// @return owned JAR bytes whose declared checksum has been verified when present
        /// @throws IOException if acquisition, validation, or the caller's policy fails
        Dependency resolve(String uri, byte[] checksum) throws IOException;
    }

    /// Owns acquired JAR bytes and their original filename for automatic-module naming.
    public static final class Dependency {
        /// Original filename ending in .jar, without path separators.
        public final String jarName;
        /// Complete archive bytes; must remain unchanged while the reader imports them.
        public final byte[] bytes;

        /// Retains the supplied array without copying it.
        ///
        /// @param jarName original JAR filename, validated during import
        /// @param bytes acquired archive bytes, validated during import
        /// @throws NullPointerException if either argument is null
        public Dependency(String jarName, byte[] bytes) {
            this.jarName = Objects.requireNonNull(jarName);
            this.bytes = Objects.requireNonNull(bytes);
        }
    }

    /// Closes the owned handle; the caller retains ownership of the snapshot file.
    @Override
    public void close() throws IOException {
        if (!closed) {
            closed = true;
            file.close();
        }
    }
}

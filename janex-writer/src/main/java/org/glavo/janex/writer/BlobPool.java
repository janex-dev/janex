// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.*;

import com.github.luben.zstd.ZstdCompressCtx;
import org.glavo.janex.reader.Checksum;

/// Holds one encoded blob pool with a data pool scoped to its resource root.
final class BlobPool {
    /// Section identifier assigned by the container writer.
    final long id;
    /// Index of the complete resource-root blob.
    final int root;
    /// Complete encoded BLOBPOOL section.
    final byte[] bytes;
    /// Page-directory metadata stored in the section table.
    final Map<Integer, Object> info;

    /// Retains the completed encoding without copying its arrays.
    private BlobPool(long id, int root, byte[] bytes, Map<Integer, Object> info) {
        this.id = id;
        this.root = root;
        this.bytes = bytes;
        this.info = info;
    }

    /// Builds one resource root with its own data pool.
    static BlobPool local(long id, Resources resources, PackOptions options, boolean transform) throws IOException {
        try (ZstdCompressCtx compressor = StoredBlob.compressor(options)) {
            Builder builder = new Builder(id, resources, options, transform);
            return builder.finish(compressor);
        }
    }

    /// Returns section bytes, the integrity-covered section-table row, and the application's root reference size.
    long encodedSize() throws IOException {
        return (long) bytes.length + Encoding.cbor(Map.of(0, 0x4c4f4f50424f4c42L, 1, id,
                2, bytes.length, 3, new byte[1 + Checksum.Algorithm.SHA256.digestLength()], 4, info)).length
                + Encoding.cbor(List.of(id, root)).length;
    }

    /// Collects file blobs and strings for one resource root.
    private static final class Builder {
        /// Section identifier assigned by the container writer.
        private final long id;
        /// Raw blobs in stable index order.
        private final List<byte[]> blobs = new ArrayList<>();
        /// Shared path and class-constant bytes in insertion order, beginning with empty bytes.
        private final DataPool strings;
        /// Digest buckets retaining exact bytes to distinguish hash collisions.
        private final Map<ByteBuffer, List<SharedFile>> shared = new HashMap<>();
        /// Whether this candidate uses class transforms and split class resource names.
        private final boolean transform;
        /// Per-write policy and input limits.
        private final PackOptions options;
        /// Index of the complete resource-root blob.
        final int root;

        /// Builds one root with stable references to its own data pool.
        Builder(long id, Resources resources, PackOptions options, boolean transform) throws IOException {
            this.id = id;
            this.options = options;
            this.transform = transform;
            strings = new DataPool(options.limits);
            append(new byte[0]);
            Encoding layers = new Encoding();
            layers.uint(resources.layers.size());
            for (var layer : resources.layers.entrySet()) {
                layers.map(layer.getKey() == 0 ? Map.of() : JanexWriter.condition("vers:jep322/>=" + layer.getKey()));
                SortedMap<String, SortedMap<String, Resources.Node>> directories = Resources.tree();
                Map<String, Integer> modes = new HashMap<>();
                for (var entry : layer.getValue().entrySet()) {
                    if (entry.getValue().directory()) {
                        directories.computeIfAbsent(entry.getKey(), ignored -> Resources.tree());
                        modes.put(entry.getKey(), entry.getValue().mode());
                    } else {
                        int slash = entry.getKey().lastIndexOf('/');
                        String parent = slash < 0 ? "" : entry.getKey().substring(0, slash);
                        String name = entry.getKey().substring(slash + 1);
                        directories.computeIfAbsent(parent, ignored -> Resources.tree()).put(name, entry.getValue());
                    }
                }
                layers.uint(directories.size());
                for (var directory : directories.entrySet()) {
                    layers.uint(intern(directory.getKey()));
                    layers.map(mode(modes.getOrDefault(directory.getKey(), -1)));
                    layers.uint(directory.getValue().size());
                    Encoding entries = new Encoding();
                    for (var entry : directory.getValue().entrySet()) {
                        Resources.Node node = entry.getValue();
                        entries.little(node.target() == null ? 0x00534552 : 0x4c4d5953, 4);
                        name(entries, entry.getKey());
                        Map<Integer, Object> metadata = mode(node.mode());
                        if (node.target() != null) {
                            entries.uint(intern(node.target()));
                        } else {
                            byte[] checksum = Checksum.compute(Checksum.Algorithm.XXH3_64, node.bytes()).encode();
                            if (node.bytes().length == 0) entries.inline(node.bytes());
                            else entries.writeBytes(file(entry.getKey(), node.bytes(), checksum));
                            metadata.put(0, checksum);
                        }
                        entries.map(metadata);
                        options.limits.bytes(entries.size());
                    }
                    layers.inline(entries.toByteArray());
                    options.limits.bytes(layers.size());
                }
            }
            Encoding resource = new Encoding();
            resource.uint(id);
            resource.uint(0);
            resource.map(Map.of("janex.java.jar_name", resources.name));
            resource.writeBytes(layers.toByteArray());
            root = append(resource.toByteArray());
        }

        /// Finalizes this root's data pool and encodes its complete blob section.
        BlobPool finish(ZstdCompressCtx compressor) throws IOException {
            blobs.set(0, strings.encode());
            Encoding data = new Encoding();
            List<byte[]> descriptions = new ArrayList<>();
            for (byte[] blob : blobs) {
                StoredBlob stored = StoredBlob.encode(blob, compressor, data.size(), false);
                Encoding description = new Encoding();
                description.uint(data.size());
                description.writeBytes(stored.encoding());
                descriptions.add(description.toByteArray());
                data.writeBytes(stored.bytes());
            }
            List<Object> pages = new ArrayList<>();
            for (int start = 0; start < blobs.size(); start += 256) {
                Encoding page = new Encoding();
                for (int index = start; index < Math.min(start + 256, blobs.size()); index++) {
                    page.write(0);
                    page.sized(descriptions.get(index));
                }
                options.limits.bytes(page.size());
                byte[] decoded = page.toByteArray();
                StoredBlob stored = StoredBlob.encode(decoded, compressor, data.size(), true);
                pages.add(List.of(data.size(), stored.encoding(), JanexWriter.sha256(decoded)));
                data.writeBytes(stored.bytes());
            }
            Encoding section = new Encoding();
            section.little(0x4c4f4f50424f4c42L, 8);
            section.writeBytes(data.toByteArray());
            return new BlobPool(id, root, section.toByteArray(), Map.of(0, blobs.size(), 1, 8, 2, pages));
        }

        /// Interns one string without changing existing indices.
        private int intern(String value) throws IOException {
            return strings.intern(Encoding.utf8(value));
        }

        /// Encodes class filenames by sharing their basename with class constants.
        private void name(Encoding output, String name) throws IOException {
            if (transform && name.endsWith(".class") && name.length() > 6) {
                output.write(0);
                output.write(0);
                output.uint(2);
                output.uint(intern(name.substring(0, name.length() - 6)));
                output.uint(intern(".class"));
            } else output.uint(intern(name));
        }

        /// Reuses the same content and transforms only when complete original bytes are equal.
        private byte[] file(String name, byte[] bytes, byte[] checksum) throws IOException {
            List<SharedFile> matches = shared.computeIfAbsent(ByteBuffer.wrap(checksum), ignored -> new ArrayList<>());
            for (SharedFile previous : matches) if (Arrays.equals(previous.original, bytes)) return previous.content;
            byte[] transformed = transform && name.endsWith(".class")
                    ? ClassFileEncoder.transform(bytes, strings, options.limits) : null;
            Encoding content = new Encoding();
            content.write(1);
            content.uint(id);
            content.uint(append(transformed == null ? bytes : transformed));
            content.uint(transformed == null ? 0 : 1);
            if (transformed != null) {
                content.uint(bytes.length);
                content.write(1);
                content.write(0);
            }
            byte[] result = content.toByteArray();
            matches.add(new SharedFile(bytes, result));
            return result;
        }

        /// Retains exact original bytes and their reusable content descriptor.
        /// @param original unmodified imported file bytes
        /// @param content encoded Content value
        private record SharedFile(byte[] original, byte[] content) {
            /// Retains the arrays for the lifetime of this builder.
            private SharedFile { }
        }

        /// Adds one bounded blob and returns its stable index.
        private int append(byte[] bytes) throws IOException {
            options.limits.bytes(bytes.length);
            options.limits.elements((long) blobs.size() + 1);
            int index = blobs.size();
            blobs.add(bytes);
            return index;
        }

        /// Creates mutable optional permission metadata.
        private static Map<Integer, Object> mode(int mode) {
            Map<Integer, Object> result = new HashMap<>();
            if (mode >= 0) result.put(5, mode);
            return result;
        }
    }
}

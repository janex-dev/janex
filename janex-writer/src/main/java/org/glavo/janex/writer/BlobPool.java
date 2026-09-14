// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.*;

import org.glavo.janex.reader.Checksum;

/// Builds one pool with shared file blobs and independently indexed table pages.
final class BlobPool {
    /// Section identifier assigned by the container writer.
    private final long id;
    /// Raw blobs in stable index order.
    private final List<byte[]> blobs = new ArrayList<>();
    /// Distinct path strings in insertion order, beginning with the empty string.
    private final Map<String, Integer> strings = new LinkedHashMap<>();
    /// Digest buckets retaining exact bytes to distinguish hash collisions.
    private final Map<ByteBuffer, List<Integer>> shared = new HashMap<>();
    /// Per-write policy and input limits.
    private final PackOptions options;
    /// Index of the complete resource-root blob.
    final int root;
    /// Complete encoded BLOBPOOL section.
    final byte[] bytes;
    /// Page-directory metadata stored in the section table.
    final Map<Integer, Object> info;

    /// Builds a resource root, string pool, and blob table from imported layers.
    BlobPool(long id, Resources resources, PackOptions options) throws IOException {
        this.id = id;
        this.options = options;
        intern("");
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
                    entries.uint(intern(entry.getKey()));
                    Map<Integer, Object> metadata = mode(node.mode());
                    if (node.target() != null) {
                        entries.uint(intern(node.target()));
                    } else {
                        byte[] checksum = Checksum.compute(Checksum.Algorithm.XXH3_64, node.bytes()).encode();
                        if (node.bytes().length == 0) entries.inline(node.bytes());
                        else entries.blob(id, file(node.bytes(), checksum));
                        metadata.put(0, checksum);
                    }
                    entries.map(metadata);
                    options.limits.bytes(entries.size());
                }
                layers.inline(entries.toByteArray());
                options.limits.bytes(layers.size());
            }
        }
        Encoding stringPool = new Encoding();
        stringPool.uint(strings.size());
        for (String string : strings.keySet()) stringPool.sized(Encoding.utf8(string));
        int pool = append(stringPool.toByteArray());
        Encoding resource = new Encoding();
        resource.uint(id);
        resource.uint(pool);
        resource.map(Map.of("janex.java.jar_name", resources.name));
        resource.writeBytes(layers.toByteArray());
        root = append(resource.toByteArray());

        Encoding data = new Encoding();
        List<byte[]> descriptions = new ArrayList<>();
        for (byte[] blob : blobs) {
            StoredBlob stored = StoredBlob.encode(blob, options.compression, data.size(), false);
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
            StoredBlob stored = StoredBlob.encode(decoded, options.compression, data.size(), true);
            pages.add(List.of(data.size(), stored.encoding(), JanexWriter.sha256(decoded)));
            data.writeBytes(stored.bytes());
        }
        Encoding section = new Encoding();
        section.little(0x4c4f4f50424f4c42L, 8);
        section.writeBytes(data.toByteArray());
        bytes = section.toByteArray();
        info = Map.of(0, blobs.size(), 1, 8, 2, pages);
    }

    /// Interns one string without changing existing indices.
    private int intern(String value) throws IOException {
        Encoding.utf8(value);
        Integer index = strings.get(value);
        if (index != null) return index;
        options.limits.elements((long) strings.size() + 1);
        index = strings.size();
        strings.put(value, index);
        return index;
    }

    /// Reuses a file blob only when its complete bytes are equal.
    private int file(byte[] bytes, byte[] checksum) throws IOException {
        List<Integer> matches = shared.computeIfAbsent(ByteBuffer.wrap(checksum), ignored -> new ArrayList<>());
        for (int index : matches) if (Arrays.equals(blobs.get(index), bytes)) return index;
        int index = append(bytes);
        matches.add(index);
        return index;
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

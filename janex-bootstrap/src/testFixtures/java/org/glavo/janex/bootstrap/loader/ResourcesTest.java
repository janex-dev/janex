// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap.loader;

import java.io.*;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

import org.glavo.janex.reader.JanexReader;
import org.glavo.janex.reader.ReadLimits;
import org.glavo.janex.reader.internal.codec.zstd.Zstandard;

/// Checks expanded resources against native format decoding and symbolic-link resolution.
public final class ResourcesTest {
    /// Prevents instantiation.
    private ResourcesTest() {
    }

    /// Reads trusted native vectors and compares the complete expanded resource map.
    public static void main(String[] arguments) throws Exception {
        Path snapshot = Paths.get(arguments[0]).resolveSibling("resource.janex");
        try (DataInputStream input = new DataInputStream(new BufferedInputStream(new FileInputStream(arguments[0])))) {
            int count = input.readInt();
            for (int vector = 0; vector < count; vector++) {
                ReadLimits limits = new ReadLimits(input.readInt(), input.readInt(), input.readInt());
                Files.write(snapshot, bytes(input));
                boolean valid = input.readBoolean();
                String diagnostic = new String(bytes(input), StandardCharsets.UTF_8);
                Map<String, byte[]> expected = new LinkedHashMap<String, byte[]>();
                Map<String, List<Object>> expectedMetadata = new LinkedHashMap<String, List<Object>>();
                if (valid) {
                    int entries = input.readInt();
                    for (int i = 0; i < entries; i++) {
                        String name = new String(bytes(input), StandardCharsets.UTF_8);
                        expected.put(name, bytes(input));
                        List<Object> metadata = new ArrayList<Object>();
                        for (int time = 0; time < 3; time++) {
                            BigInteger value = null;
                            if (input.readBoolean()) {
                                byte[] magnitude = new byte[16];
                                input.readFully(magnitude);
                                value = new BigInteger(magnitude);
                            }
                            metadata.add(value);
                        }
                        metadata.add(input.readInt());
                        expectedMetadata.put(name, metadata);
                    }
                }
                Map<String, byte[]> actual = new LinkedHashMap<String, byte[]>();
                Map<String, List<Object>> actualMetadata = new LinkedHashMap<String, List<Object>>();
                try (JanexReader reader = new JanexReader(snapshot, (data, length, dictionary) -> {
                    byte[] output = new byte[length];
                    if (Zstandard.decompress(data, 0, data.length, output, 0, length, dictionary) != length) {
                        throw new IOException("Decoded length mismatch");
                    }
                    return output;
                }, null, null, limits)) {
                    JanexReader.Launch launch = reader.launch("main");
                    try (ResourceIndex index = new ResourceIndex(new ByteArrayInputStream(ResourceIndexes.encode(launch.resources)))) {
                        if (index.roots.size() != 1) {
                            throw new AssertionError("Expected exactly one root");
                        }
                        for (Map.Entry<String, ResourceIndex.Resource> entry : index.roots.get(0).files.entrySet()) {
                            actual.put(entry.getKey(), entry.getValue().read());
                            ResourceIndex.Resource resource = entry.getValue();
                            if (!resource.buffer().isReadOnly()) {
                                throw new AssertionError("Writable shared resource buffer");
                            }
                            actualMetadata.put(entry.getKey(), Arrays.<Object>asList(resource.times[0], resource.times[1],
                                    resource.times[2], resource.permissions));
                        }
                    }
                    if (!valid) {
                        throw new AssertionError("Invalid resource vector accepted: " + vector + ": " + diagnostic
                                + " (limits " + limits.maxBytes() + "/" + limits.maxElements() + "/" + limits.maxDepth() + ")");
                    }
                } catch (IOException failure) {
                    if (valid) {
                        throw new AssertionError("Valid resource vector rejected: " + vector, failure);
                    }
                    continue;
                }
                if (!new ArrayList<String>(actual.keySet()).equals(new ArrayList<String>(expected.keySet()))) {
                    throw new AssertionError("Resource paths differ at vector " + vector + ": " + actual.keySet() + " != " + expected.keySet());
                }
                for (String name : expected.keySet()) {
                    if (!Arrays.equals(expected.get(name), actual.get(name))) {
                        throw new AssertionError("Resource bytes differ at vector " + vector + ": " + name);
                    }
                }
                if (!expectedMetadata.equals(actualMetadata)) {
                    throw new AssertionError("Resource metadata differs at vector " + vector + ": " + actualMetadata + " != " + expectedMetadata);
                }
            }
        }
        Files.delete(snapshot);
    }

    /// Reads one trusted harness byte sequence.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] bytes = new byte[input.readInt()];
        input.readFully(bytes);
        return bytes;
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap.loader;

import java.io.*;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.*;

import org.glavo.janex.reader.*;

/// Transfers selected root references while resource expansion remains in the application JVM.
public final class ResourceHandoff {
    /// Prevents instantiation.
    private ResourceHandoff() {
    }

    /// Prepares the resource index from references selected and verified by the Host.
    /// The stream is consumed and closed; the returned index owns its container handle.
    /// Referenced files must remain unchanged until resource consumers finish reading them.
    /// @param stream trusted handoff referring to files already verified by the parent
    /// @return resource index retaining the container handle
    /// @throws IOException if framing, parsing, or opening a referenced file fails
    public static ResourceIndex read(InputStream stream) throws IOException {
        try (DataInputStream input = new DataInputStream(stream)) {
            if (input.readLong() != 0x4a4e58524f4f5432L) throw new IOException("Invalid root handoff");
            int maxBytes = nonnegative(input.readInt());
            int maxElements = nonnegative(input.readInt());
            int maxDepth = nonnegative(input.readInt());
            ReadLimits limits = new ReadLimits(maxBytes, maxElements, maxDepth);
            long logicalLimit = input.readLong();
            Path path = Paths.get(text(input, limits));
            String[] context = new String[5];
            for (int i = 0; i < context.length; i++) context[i] = text(input, limits);
            Map<String, String> requirements = new LinkedHashMap<String, String>();
            int count = limits.elements(input.readInt());
            for (int i = 0; i < count; i++) {
                String name = text(input, limits);
                String version = text(input, limits);
                String previous = requirements.get(name);
                if (previous != null && !previous.isEmpty() && !version.isEmpty() && !previous.equals(version)) {
                    throw new IOException("Conflicting module requirement");
                }
                if (previous == null || !version.isEmpty()) requirements.put(name, version);
            }
            List<ResourceRequest> requests = new ArrayList<ResourceRequest>();
            count = limits.elements(input.readInt());
            for (int i = 0; i < count; i++) {
                boolean module = input.readBoolean();
                int kind = input.readUnsignedByte();
                if (kind == 0) {
                    requests.add(new ResourceRequest(module, input.readLong(), input.readLong()));
                } else if (kind == 1) {
                    String name = text(input, limits);
                    requests.add(new ResourceRequest(module, Paths.get(text(input, limits)), name));
                } else {
                    throw new IOException("Invalid resource request kind");
                }
            }
            if (input.read() != -1) throw new IOException("Trailing resource handoff data");
            return new ResourceIndex(JanexReader.prepareResources(path, requests, requirements, context, limits, logicalLimit));
        }
    }

    /// Encodes selected references without parsing or copying resource contents.
    /// Referenced files remain caller-owned and must stay unchanged until the child exits.
    /// @param source nonnull absolute path to the verified container
    /// @param context five nonnull strings: OS, architecture, invocation, Java version, and vendor
    /// @param requests selected roots in lookup order
    /// @param requirements module names and optional exact versions
    /// @param limits read limits inherited by the child
    /// @param logicalLimit aggregate expanded byte allowance, nonnegative
    /// @return an independent encoded handoff
    /// @throws IOException if the encoded handoff exceeds the byte limit
    public static byte[] encode(Path source, String[] context, List<ResourceRequest> requests,
            Map<String, String> requirements, ReadLimits limits, long logicalLimit) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        DataOutputStream output = new DataOutputStream(bytes);
        output.writeLong(0x4a4e58524f4f5432L);
        output.writeInt(limits.maxBytes());
        output.writeInt(limits.maxElements());
        output.writeInt(limits.maxDepth());
        output.writeLong(logicalLimit);
        ResourceIndexes.string(output, source.toString());
        for (String value : context) ResourceIndexes.string(output, value);
        output.writeInt(requirements.size());
        for (Map.Entry<String, String> entry : requirements.entrySet()) {
            ResourceIndexes.string(output, entry.getKey());
            ResourceIndexes.string(output, entry.getValue());
        }
        output.writeInt(requests.size());
        for (ResourceRequest request : requests) {
            output.writeBoolean(request.module);
            output.writeByte(request.path == null ? 0 : 1);
            if (request.path == null) {
                output.writeLong(request.pool);
                output.writeLong(request.index);
            } else {
                ResourceIndexes.string(output, request.jarName);
                ResourceIndexes.string(output, request.path.toString());
            }
        }
        limits.bytes(bytes.size());
        return bytes.toByteArray();
    }

    /// Reads a bounded string preserving every UTF-16 code unit.
    private static String text(DataInputStream input, ReadLimits limits) throws IOException {
        int length = nonnegative(input.readInt());
        limits.bytes(length * 2L);
        char[] units = new char[length];
        for (int i = 0; i < length; i++) units[i] = input.readChar();
        return new String(units);
    }

    /// Rejects negative count encodings.
    private static int nonnegative(int value) throws IOException {
        if (value < 0) throw new IOException("Negative handoff length");
        return value;
    }
}

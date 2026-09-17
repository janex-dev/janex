// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap.loader;

import java.io.*;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.MessageDigest;
import java.util.*;

import org.glavo.janex.reader.*;

/// Transfers selected root references while resource expansion remains in the application JVM.
public final class ResourceHandoff {
    /// Prevents instantiation.
    private ResourceHandoff() {
    }

    /// Verifies borrowed files against Host-selected content identities and prepares the index.
    /// The stream is consumed and closed; the returned index owns its container handle.
    /// Referenced files must remain unchanged until resource consumers finish reading them.
    /// @param stream trusted handoff binding paths to the parent's verified bytes
    /// @return resource index retaining the container handle
    /// @throws IOException if a file differs from its identity, or framing, parsing, or opening fails
    public static ResourceIndex read(InputStream stream) throws IOException {
        try (DataInputStream input = new DataInputStream(stream)) {
            if (input.readLong() != 0x4a4e58524f4f5432L) throw new IOException("Invalid root handoff");
            int maxBytes = nonnegative(input.readInt());
            int maxElements = nonnegative(input.readInt());
            int maxDepth = nonnegative(input.readInt());
            ReadLimits limits = new ReadLimits(maxBytes, maxElements, maxDepth);
            long logicalLimit = input.readLong();
            Map<Path, FileIdentity> verified = new HashMap<Path, FileIdentity>();
            Path path = file(input, limits, verified);
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
                    requests.add(new ResourceRequest(module, file(input, limits, verified), name));
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
    /// @param source container identity captured before launch preparation
    /// @param context five nonnull strings: OS, architecture, invocation, Java version, and vendor
    /// @param requests selected roots in lookup order
    /// @param requirements module names and optional exact versions
    /// @param limits read limits inherited by the child
    /// @param logicalLimit aggregate expanded byte allowance, nonnegative
    /// @return an independent encoded handoff
    /// @throws IOException if the encoded handoff exceeds the byte limit or an external request lacks an identity
    public static byte[] encode(FileIdentity source, String[] context, List<ResourceRequest> requests,
            Map<String, String> requirements, ReadLimits limits, long logicalLimit) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        DataOutputStream output = new DataOutputStream(bytes);
        output.writeLong(0x4a4e58524f4f5432L);
        output.writeInt(limits.maxBytes());
        output.writeInt(limits.maxElements());
        output.writeInt(limits.maxDepth());
        output.writeLong(logicalLimit);
        source.write(output);
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
                if (request.checksum == null) throw new IOException("External request has no content identity");
                new FileIdentity(request.path, request.length, request.checksum.digest()).write(output);
            }
        }
        limits.bytes(bytes.size());
        return bytes.toByteArray();
    }

    /// Reads one identity, reusing successful verification for repeated paths within this handoff.
    private static Path file(DataInputStream input, ReadLimits limits, Map<Path, FileIdentity> verified) throws IOException {
        Path path = Paths.get(text(input, limits));
        long length = input.readLong();
        if (length < 0) throw new IOException("Negative handoff file length");
        byte[] digest = new byte[32];
        input.readFully(digest);
        FileIdentity actual = verified.get(path);
        if (actual == null) {
            actual = FileIdentity.capture(path, length);
            verified.put(path, actual);
        }
        if (actual.length != length || !MessageDigest.isEqual(digest, actual.digest)) {
            throw new IOException("Launch file changed after verification: " + path);
        }
        return path;
    }

    /// Binds a borrowed file to a byte length and SHA-256 digest.
    /// The file must remain unchanged throughout preparation and application execution.
    public static final class FileIdentity {
        /// Absolute file path retained without acquiring ownership of the file.
        private final Path path;
        /// Exact byte length observed while hashing.
        private final long length;
        /// Owned SHA-256 digest.
        private final byte[] digest;

        /// Retains a validated length and owned digest.
        private FileIdentity(Path path, long length, byte[] digest) {
            this.path = path.toAbsolutePath();
            this.length = length;
            this.digest = digest;
        }

        /// Reads a content identity without buffering or copying the complete file.
        /// @param path existing file, which must remain unchanged while read
        /// @param limit maximum file size in bytes, nonnegative
        /// @return observed length and SHA-256 identity
        /// @throws IOException if reading fails or the file exceeds the limit
        /// @throws IllegalArgumentException if limit is negative
        public static FileIdentity capture(Path path, long limit) throws IOException {
            if (limit < 0) throw new IllegalArgumentException("Negative file limit");
            MessageDigest digest = Checksum.Algorithm.SHA256.newDigest();
            long length = 0;
            try (InputStream input = Files.newInputStream(path)) {
                byte[] buffer = new byte[32768];
                for (int count; (count = input.read(buffer)) != -1;) {
                    if (count > limit - length) throw new IOException("Launch file exceeds byte limit: " + path);
                    length += count;
                    digest.update(buffer, 0, count);
                }
            }
            return new FileIdentity(path, length, digest.digest());
        }

        /// Appends the private path, length, and fixed-width digest representation.
        private void write(DataOutputStream output) throws IOException {
            ResourceIndexes.string(output, path.toString());
            output.writeLong(length);
            output.write(digest);
        }
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

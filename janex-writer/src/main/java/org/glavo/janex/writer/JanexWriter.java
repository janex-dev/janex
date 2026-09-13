// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.*;
import java.lang.module.ModuleDescriptor;
import java.nio.channels.FileChannel;
import java.nio.file.*;
import java.nio.file.attribute.PosixFilePermissions;
import java.util.*;

import org.glavo.janex.reader.Application;
import org.glavo.janex.reader.Checksum;

import static org.glavo.janex.reader.internal.Input.require;

/// Writes Janex 0.1 application packages with shared raw blobs and SHA-256 integrity coverage.
/// File checksums use XXH3-64. This implementation emits no compression, CLASSFILE transforms,
/// or publisher signatures. The optional JAR tail contains the bundled portable Java launcher.
public final class JanexWriter {
    /// Prevents instantiation.
    private JanexWriter() { }

    /// Writes a new package after importing every input and validating its application descriptor.
    /// Inputs and options must remain unchanged during the call. No network requests are made.
    /// The destination must not exist. Temporary files are removed on failure; bytes are synced
    /// before publication. Concurrent writers to the same destination are not supported.
    ///
    /// @param options nonnull input paths, launch settings, and allocation limits
    /// @throws IOException if inputs, metadata, limits, wrappers, or filesystem operations are invalid
    public static void write(PackOptions options) throws IOException {
        Objects.requireNonNull(options);
        Objects.requireNonNull(options.limits);
        require(options.maxTotalBytes >= 0, "Negative aggregate byte limit");
        Path output = options.output.toAbsolutePath().normalize();
        if (Files.exists(output, LinkOption.NOFOLLOW_LINKS)) throw new FileAlreadyExistsException(output.toString());
        List<Path> paths = new ArrayList<>();
        paths.add(options.source);
        paths.addAll(options.classPath);
        paths.addAll(options.modulePath);
        options.limits.elements(paths.size());
        List<Resources> roots = new ArrayList<>();
        long[] total = {0};
        for (Path path : paths) roots.add(new Resources(path, options, total));
        String mainClass = options.mainClass;
        if (mainClass == null && roots.get(0).manifest != null) mainClass = roots.get(0).manifest.getMainAttributes().getValue("Main-Class");
        if (mainClass == null && options.mainModule == null) {
            for (var layer : roots.get(0).layers.values()) {
                Resources.Node module = layer.get("module-info.class");
                if (module != null && module.bytes() != null) {
                    String inferred;
                    try {
                        inferred = ModuleDescriptor.read(new ByteArrayInputStream(module.bytes())).mainClass().orElse(null);
                    } catch (java.lang.module.InvalidModuleDescriptorException | IllegalArgumentException failure) {
                        throw new IOException("Invalid module descriptor", failure);
                    }
                    require(mainClass == null || inferred == null || mainClass.equals(inferred), "Module main class varies by layer");
                    if (inferred != null) mainClass = inferred;
                }
            }
        }
        require(mainClass != null || options.mainModule != null, "Cannot determine application entry point");
        Map<Integer, Object> entry = new HashMap<>();
        if (mainClass != null) entry.put(0, mainClass);
        if (options.mainModule != null) entry.put(1, options.mainModule);
        List<Object> classPath = new ArrayList<>();
        List<Object> modulePath = new ArrayList<>();
        List<BlobPool> pools = new ArrayList<>();
        for (int index = 0; index < roots.size(); index++) {
            BlobPool pool = new BlobPool(index + 1L, roots.get(index), options);
            pools.add(pool);
            boolean modular = index == 0 ? options.mainModule != null : index > options.classPath.size();
            (modular ? modulePath : classPath).add(Map.of(0, 0, 1, List.of(index + 1, pool.root)));
        }
        for (var dependency : options.externalClassPath) classPath.add(external(dependency));
        for (var dependency : options.externalModulePath) modulePath.add(external(dependency));
        Map<Integer, Object> launch = new HashMap<>();
        launch.put(1, entry);
        launch.put(2, modulePath);
        launch.put(3, classPath);
        launch.put(5, List.copyOf(options.jvmOptions));
        launch.put(7, List.copyOf(options.arguments));
        if (options.javaVersion != null) launch.put(0, condition(options.javaVersion));
        Encoding application = new Encoding();
        application.little(0x50504158454e414aL, 8);
        application.map(Map.of(0, Map.of(0, launch)));
        Map<Integer, Object> type = Map.of(0, options.applicationId, 1, "janex.java");
        new Application(application.toByteArray(), Encoding.cbor(type), options.limits);
        byte[] header = NativePrefix.create(options);
        byte[] tail = new byte[0];
        if (options.withLauncher) {
            try (InputStream input = JanexWriter.class.getResourceAsStream("janex-bootstrap.jar")) {
                require(input != null, "Bundled Java launcher is unavailable");
                tail = input.readAllBytes();
            }
        }
        Files.createDirectories(output.getParent());
        Path temporary = Files.createTempFile(output.getParent(), ".janex-writer-", ".tmp");
        try {
            try (OutputStream stream = new BufferedOutputStream(Files.newOutputStream(temporary))) {
                stream.write(header);
                stream.write(Encoding.utf8("JANEX\0\0\0"));
                long length = 8;
                List<Object> sections = new ArrayList<>();
                for (int index = 0; index < pools.size(); index++) {
                    BlobPool pool = pools.get(index);
                    stream.write(pool.bytes);
                    sections.add(section(index + 1, 0x4c4f4f50424f4c42L, pool.bytes, pool.info));
                    length += pool.bytes.length;
                }
                byte[] app = application.toByteArray();
                stream.write(app);
                sections.add(section(0, 0x50504158454e414aL, app, type));
                length += app.length;
                Encoding metadata = new Encoding();
                metadata.writeBytes(Encoding.utf8("METADATA"));
                metadata.little(0, 4);
                metadata.little(1, 4);
                metadata.map(Map.of(0, sections, 1, region(header), 2, region(tail)));
                metadata.write(1);
                byte[] checksum = sha256(metadata.toByteArray());
                metadata.sized(checksum);
                int metadataLength = metadata.size() + 24;
                options.limits.bytes(metadataLength);
                metadata.writeBytes(Encoding.utf8("JANEXEND"));
                metadata.little(metadataLength, 8);
                metadata.little(length + metadataLength, 8);
                stream.write(metadata.toByteArray());
                stream.write(tail);
            }
            if (options.nativeLauncher != null && temporary.getFileSystem().supportedFileAttributeViews().contains("posix")) {
                Files.setPosixFilePermissions(temporary, PosixFilePermissions.fromString("rwxr-xr-x"));
            }
            try (FileChannel channel = FileChannel.open(temporary, StandardOpenOption.WRITE)) { channel.force(true); }
            Files.move(temporary, output);
        } finally {
            Files.deleteIfExists(temporary);
        }
    }

    /// Encodes one SHA-256 ChecksumValue in its canonical byte representation.
    static byte[] sha256(byte[] bytes) throws IOException {
        return Checksum.compute(Checksum.Algorithm.SHA256, bytes).encode();
    }

    /// Creates a Java runtime version condition.
    static Map<Integer, Object> condition(String version) {
        return Map.of(5, Map.of(0, "janex.java", 1, Map.of(0, version)));
    }

    /// Creates an integrity-covered section-table row.
    private static Map<Integer, Object> section(int id, long type, byte[] bytes, Map<Integer, Object> info) throws IOException {
        return Map.of(0, type, 1, id, 2, bytes.length, 3, sha256(bytes), 4, info);
    }

    /// Constrains an external region to its exact size and optional checksum.
    private static Map<Integer, Object> region(byte[] bytes) throws IOException {
        return bytes.length == 0 ? Map.of(0, 0) : Map.of(0, bytes.length, 1, sha256(bytes));
    }

    /// Encodes an external declaration for validation by the shared application reader.
    private static Map<Integer, Object> external(PackOptions.ExternalDependency dependency) {
        Map<Integer, Object> result = new HashMap<>();
        result.put(0, 1);
        result.put(1, dependency.uri());
        if (dependency.checksum() != null) result.put(2, dependency.checksum().encode());
        return result;
    }
}

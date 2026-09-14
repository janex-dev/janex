// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.glavo.janex.reader.*;
import org.glavo.janex.reader.internal.Input;
import org.glavo.janex.reader.internal.codec.ZstandardFrames;
import org.glavo.janex.reader.internal.codec.zstd.Zstandard;

/// Exercises portable writing, resource layers, integrity, bounds, and deterministic output.
public final class WriterTest {
    /// Prevents instantiation.
    private WriterTest() { }

    /// Runs isolated fixtures with no native tools or network access.
    /// @param args unused arguments
    /// @throws Exception if a fixture or interoperability assertion fails
    public static void main(String[] args) throws Exception {
        Path root = Files.createTempDirectory("janex-writer-");
        try {
            jar(root);
            directory(root);
            compression(root);
            external(root);
            failures(root);
            System.out.println("Java writer checks passed.");
        } finally {
            try (var paths = Files.walk(root)) {
                for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) Files.deleteIfExists(path);
            }
        }
    }

    /// Checks paging, duplicate blobs, Multi-Release selection, inference, and both wrapper forms.
    private static void jar(Path root) throws Exception {
        Path jar = root.resolve("library-1.jar");
        try (ZipOutputStream zip = new ZipOutputStream(Files.newOutputStream(jar))) {
            entry(zip, "META-INF/MANIFEST.MF", "Manifest-Version: 1.0\r\nMain-Class: demo.Main\r\nMulti-Release: true\r\n\r\n");
            entry(zip, "value.txt", "base");
            entry(zip, "META-INF/versions/9/value.txt", "selected");
            entry(zip, "META-INF/versions/999/value.txt", "future");
            entry(zip, "same.txt", "base");
            entry(zip, "empty.txt", "");
            entry(zip, "\uE000.txt", "bmp");
            entry(zip, "\uD83D\uDE80.txt", "unicode");
            for (int index = 0; index < 270; index++) entry(zip, "data/" + index, "content-" + index);
        }
        PackOptions options = new PackOptions(jar, root.resolve("first.janex"));
        options.arguments.addAll(List.of("", "two words", "\uD83D\uDE80"));
        options.withLauncher = true;
        JanexWriter.write(options);
        try (JanexReader reader = new JanexReader(options.output)) {
            require(reader.integrity().completeSecureCoverage(), "Missing integrity coverage");
            var launch = reader.launch("main");
            require(launch.mainClass.equals("demo.Main"), "Manifest main class was lost");
            require(launch.arguments.equals(options.arguments), "Arguments changed");
            require(launch.resources.roots().get(0).name().equals("library-1.jar"), "JAR name changed");
            require(text(launch.resources, "value.txt").equals("selected"), "Wrong Multi-Release layer");
            require(text(launch.resources, "data/269").equals("content-269"), "Second table page failed");
            require(text(launch.resources, "empty.txt").isEmpty(), "Empty content changed");
            require(text(launch.resources, "\uD83D\uDE80.txt").equals("unicode"), "UTF-8 order changed");
        }
        PackOptions duplicate = new PackOptions(jar, root.resolve("second.janex"));
        duplicate.arguments.addAll(options.arguments);
        duplicate.withLauncher = true;
        JanexWriter.write(duplicate);
        require(Arrays.equals(Files.readAllBytes(options.output), Files.readAllBytes(duplicate.output)), "Non-reproducible package");
        byte[] bytes = Files.readAllBytes(options.output);
        bytes[16] ^= 1;
        Files.write(root.resolve("damaged.janex"), bytes);
        fails(() -> { try (JanexReader ignored = new JanexReader(root.resolve("damaged.janex"))) { } });
    }

    /// Checks directory input, empty directories, and retained native permission bits.
    private static void directory(Path root) throws Exception {
        Path source = root.resolve("directory");
        Files.createDirectories(source.resolve("empty"));
        Files.writeString(source.resolve("value.txt"), "directory");
        if (source.getFileSystem().supportedFileAttributeViews().contains("posix")) {
            Files.setPosixFilePermissions(source.resolve("value.txt"), java.nio.file.attribute.PosixFilePermissions.fromString("rw-r-----"));
            Files.createSymbolicLink(source.resolve("alias.txt"), Path.of("value.txt"));
        }
        PackOptions options = new PackOptions(source, root.resolve("directory.janex"));
        options.mainClass = "demo.Main";
        JanexWriter.write(options);
        try (JanexReader reader = new JanexReader(options.output)) {
            ResourcePlan plan = reader.launch("main").resources;
            require(text(plan, "value.txt").equals("directory"), "Directory content changed");
            require(plan.roots().get(0).files().containsKey("empty/"), "Empty directory was lost");
            if (Files.isSymbolicLink(source.resolve("alias.txt"))) {
                require(text(plan, "alias.txt").equals("directory"), "Symbolic link changed");
                require(plan.roots().get(0).files().get("value.txt").permissions() == 0640, "Permissions changed");
            }
        }
    }

    /// Checks that external dependencies are recorded without resolving them during writing.
    private static void external(Path root) throws Exception {
        byte[] jar = Files.readAllBytes(root.resolve("library-1.jar"));
        PackOptions options = new PackOptions(root.resolve("directory"), root.resolve("external.janex"));
        options.mainClass = "demo.Main";
        options.externalClassPath.add(new PackOptions.ExternalDependency("pkg:maven/org.example/library@1",
                Checksum.compute(Checksum.Algorithm.SHA256, jar)));
        JanexWriter.write(options);
        int[] requests = {0};
        try (JanexReader reader = new JanexReader(options.output, (JanexReader.DependencyResolver) (uri, checksum) -> {
            require(uri.equals("pkg:maven/org.example/library@1"), "External URI changed");
            Checksum.decode(checksum).verify(jar);
            requests[0]++;
            return new JanexReader.Dependency("library-1.jar", jar);
        })) {
            require(requests[0] == 0, "Dependency resolved before launch selection");
            require(reader.launch("main").resources.roots().size() == 2, "Missing external root");
            require(requests[0] == 1, "Unexpected dependency resolution count");
        }
    }

    /// Checks that malformed input and invalid limits never replace an existing output.
    private static void failures(Path root) throws Exception {
        Path source = root.resolve("directory");
        PackOptions options = new PackOptions(source, root.resolve("invalid.janex"));
        options.mainClass = "demo.Main";
        options.javaVersion = "invalid";
        fails(() -> JanexWriter.write(options));
        require(!Files.exists(options.output), "Invalid configuration published output");
        options.javaVersion = null;
        options.maxTotalBytes = 1;
        fails(() -> JanexWriter.write(options));
        options.maxTotalBytes = 1024;
        options.arguments.add("\uD800");
        fails(() -> JanexWriter.write(options));
        options.arguments.clear();
        Files.writeString(options.output, "previous");
        fails(() -> JanexWriter.write(options));
        require(Files.readString(options.output).equals("previous"), "Existing output changed");
        Path invalid = root.resolve("traversal.jar");
        try (ZipOutputStream zip = new ZipOutputStream(Files.newOutputStream(invalid))) { entry(zip, "../escape", "bad"); }
        fails(() -> JanexWriter.write(new PackOptions(invalid, root.resolve("traversal.janex"))));
    }

    /// Writes one fixture ZIP entry with UTF-8 content.
    private static void entry(ZipOutputStream zip, String name, String text) throws IOException {
        zip.putNextEntry(new ZipEntry(name));
        zip.write(text.getBytes(StandardCharsets.UTF_8));
        zip.closeEntry();
    }

    /// Reads a selected text resource through the reader's source description and decoder.
    private static String text(ResourcePlan plan, String name) throws IOException {
        return new String(content(plan, name), StandardCharsets.UTF_8);
    }

    /// Reads and decompresses a stored or inline resource using the independent reader decoder.
    private static byte[] content(ResourcePlan plan, String name) throws IOException {
        var file = plan.roots().get(0).files().get(name);
        require(file != null, "Missing resource: " + name);
        var source = plan.sources().get(file.source());
        byte[] bytes = source.inline();
        if (bytes == null) {
            bytes = new byte[source.storedLength()];
            try (RandomAccessFile input = new RandomAccessFile(plan.snapshot().toFile(), "r")) {
                input.seek(source.offset());
                input.readFully(bytes);
            }
        }
        for (int length : source.filters()) {
            ZstandardFrames.validate(bytes, plan.limits());
            byte[] decoded = new byte[length];
            require(Zstandard.decompress(bytes, 0, bytes.length, decoded, 0, length) == length,
                    "Wrong decoded resource length");
            bytes = decoded;
        }
        return bytes;
    }

    /// Checks block boundaries, incompressible input, decoded page checksums, and reproducibility.
    private static void compression(Path root) throws Exception {
        Path directory = Files.createDirectory(root.resolve("compression"));
        Map<String, byte[]> contents = new LinkedHashMap<>();
        for (int size : new int[]{0, 1, 16, 127, 128, 255, 256, 1024, 131071, 131072, 131073, 524288}) {
            byte[] bytes = new byte[size];
            for (int i = 0; i < size; i++) bytes[i] = (byte) (i % 31);
            contents.put("pattern-" + size, bytes);
        }
        byte[] noise = new byte[131073];
        new Random(42).nextBytes(noise);
        contents.put("random", noise);
        contents.put("overhead-tie", new byte[24]);
        contents.put("overhead-saving", new byte[25]);
        for (int i = 0; i < 270; i++) {
            contents.put("small-" + i, ("content-" + i).getBytes(StandardCharsets.UTF_8));
        }
        for (var entry : contents.entrySet()) Files.write(directory.resolve(entry.getKey()), entry.getValue());
        for (boolean enabled : new boolean[]{true, false}) {
            PackOptions options = new PackOptions(directory, root.resolve("compression-" + enabled + ".janex"));
            options.mainClass = "demo.Main";
            options.compression = enabled;
            JanexWriter.write(options);
            try (JanexReader reader = new JanexReader(options.output)) {
                ResourcePlan plan = reader.launch("main").resources;
                for (var entry : contents.entrySet()) {
                    require(Arrays.equals(content(plan, entry.getKey()), entry.getValue()),
                            "Compression changed " + entry.getKey());
                    var file = plan.roots().get(0).files().get(entry.getKey());
                    int filters = plan.sources().get(file.source()).filters().length;
                    if (!enabled || entry.getKey().equals("random") || entry.getKey().equals("overhead-tie")
                            || entry.getValue().length <= 16) {
                        require(filters == 0, "Unprofitable or disabled compression was used");
                    } else if (entry.getValue().length >= 1024 || entry.getKey().equals("overhead-saving")) {
                        require(filters == 1, "Compressible resource was stored raw");
                    }
                }
            }
            PackOptions duplicate = new PackOptions(directory, root.resolve("compression-copy-" + enabled + ".janex"));
            duplicate.mainClass = options.mainClass;
            duplicate.compression = enabled;
            JanexWriter.write(duplicate);
            require(Arrays.equals(Files.readAllBytes(options.output), Files.readAllBytes(duplicate.output)),
                    "Compression mode is not reproducible");

            BlobPool pool = new BlobPool(1, new Resources(directory, options, new long[]{0}), options);
            boolean compressedPage = false;
            for (Object value : (List<?>) pool.info.get(2)) {
                List<?> page = (List<?>) value;
                int offset = (Integer) page.get(0) + 8;
                Input encoding = new Input((byte[]) page.get(1));
                int stored = Math.toIntExact(encoding.uint());
                byte[] decoded = Arrays.copyOfRange(pool.bytes, offset, offset + stored);
                int filters = Math.toIntExact(encoding.uint());
                require(filters == 0 || enabled, "Disabled page compression was used");
                if (filters != 0) {
                    compressedPage = true;
                    int length = Math.toIntExact(encoding.uint());
                    require(filters == 1 && encoding.u8() == 1 && encoding.sized().length == 0,
                            "Unexpected page filter or dictionary");
                    ZstandardFrames.validate(decoded, options.limits);
                    byte[] output = new byte[length];
                    require(Zstandard.decompress(decoded, 0, decoded.length, output, 0, length) == length,
                            "Wrong decoded page length");
                    decoded = output;
                }
                encoding.end();
                Checksum.decode((byte[]) page.get(2)).verify(decoded);
            }
            require(compressedPage == enabled, "Table page compression was not exercised");
        }
        require(Files.size(root.resolve("compression-true.janex")) < Files.size(root.resolve("compression-false.janex")),
                "Compression did not reduce package size");
    }

    /// One fixture action expected to fail with an I/O error.
    private interface Action {
        /// Performs the operation.
        void run() throws IOException;
    }

    /// Requires a checked failure rather than an accepted invalid package.
    private static void fails(Action action) throws IOException {
        try { action.run(); } catch (IOException expected) { return; }
        throw new AssertionError("Invalid operation succeeded");
    }

    /// Reports an assertion independently of JVM assertion flags.
    private static void require(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
}

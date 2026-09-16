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
            mergedDirectories(root);
            compression(root);
            compressionLevels(root);
            classfiles(root);
            rootStrings(root);
            external(root);
            failures(root);
            System.out.println("Java writer checks passed.");
        } finally {
            try (var paths = Files.walk(root)) {
                for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) Files.deleteIfExists(path);
            }
        }
    }

    /// Checks direct directory merging, generated manifests, normalized modes, and conflict failures.
    private static void mergedDirectories(Path root) throws Exception {
        Path classes = Files.createDirectories(root.resolve("merged-classes/shared"));
        Path resources = Files.createDirectories(root.resolve("merged-resources/shared"));
        Files.writeString(classes.resolve("class.txt"), "compiled");
        Files.writeString(resources.resolve("resource.txt"), "resource");
        Path versioned = Files.createDirectories(resources.getParent().resolve("META-INF/versions/9/shared"));
        Files.writeString(versioned.resolve("resource.txt"), "versioned");
        Path manifestFile = resources.getParent().resolve("META-INF/MANIFEST.MF");
        Files.writeString(manifestFile, "Manifest-Version: 1.0\r\nOriginal: retained\r\n\r\nName: shared/class.txt\r\nCustom: section\r\n\r\n");
        PackOptions options = new PackOptions(List.of(classes.getParent(), resources.getParent()), root.resolve("merged.janex"));
        options.sourceName = "direct.jar";
        options.normalizeSourcePermissions = true;
        options.manifestAttributes.putAll(Map.of("Main-Class", "demo.Main", "Multi-Release", "true"));
        JanexWriter.write(options);
        try (JanexReader reader = new JanexReader(options.output)) {
            var launch = reader.launch("main");
            require(launch.mainClass.equals("demo.Main"), "Generated manifest inference failed");
            require(launch.resources.roots().size() == 1, "Primary directories became separate roots");
            require(launch.resources.roots().get(0).name().equals("direct.jar"), "Primary root name changed");
            require(text(launch.resources, "shared/class.txt").equals("compiled"), "Classes input was lost");
            require(text(launch.resources, "shared/resource.txt").equals("versioned"), "Manifest override did not enable version layers");
            var manifest = new java.util.jar.Manifest(new ByteArrayInputStream(text(launch.resources, "META-INF/MANIFEST.MF").getBytes(StandardCharsets.UTF_8)));
            require("retained".equals(manifest.getMainAttributes().getValue("Original")), "Existing attributes were lost");
            require("section".equals(manifest.getAttributes("shared/class.txt").getValue("Custom")), "Named manifest section was lost");
            require(launch.resources.roots().get(0).files().get("shared/class.txt").permissions() == 0644, "File modes were not normalized");
        }
        PackOptions duplicate = new PackOptions(options.sourceDirectories, root.resolve("merged-copy.janex"));
        duplicate.sourceName = options.sourceName;
        duplicate.normalizeSourcePermissions = true;
        duplicate.manifestAttributes.putAll(options.manifestAttributes);
        JanexWriter.write(duplicate);
        require(Arrays.equals(Files.readAllBytes(options.output), Files.readAllBytes(duplicate.output)), "Merged output is not reproducible");
        Files.writeString(resources.resolve("class.txt"), "compiled");
        PackOptions conflict = new PackOptions(options.sourceDirectories, root.resolve("merged-conflict.janex"));
        conflict.mainClass = "demo.Main";
        fails(() -> JanexWriter.write(conflict));
        require(!Files.exists(conflict.output), "Duplicate input published a package");
        Files.delete(resources.resolve("class.txt"));
        PackOptions invalid = new PackOptions(options.sourceDirectories, root.resolve("merged-invalid.janex"));
        invalid.manifestAttributes.put("Main-Class", "demo.Main\r\nInjected: true");
        fails(() -> JanexWriter.write(invalid));
        invalid.manifestAttributes.clear();
        invalid.manifestAttributes.putAll(Map.of("Main-Class", "demo.Main", "main-class", "demo.Other"));
        fails(() -> JanexWriter.write(invalid));
        invalid.sourceName = "../bad.jar";
        fails(() -> JanexWriter.write(invalid));
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
        return content(plan, plan.roots().get(0), name);
    }

    /// Reads one root without flattening resources that share a path across dependencies.
    private static byte[] content(ResourcePlan plan, ResourcePlan.Root root, String name) throws IOException {
        var file = root.files().get(name);
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
        int[][] transforms = file.transforms();
        for (int i = transforms.length - 1; i >= 0; i--) {
            bytes = ClassFile.restore(bytes, plan.pools()[transforms[i][1]], transforms[i][0], plan.limits());
        }
        return bytes;
    }

    /// Checks exact CLASSFILE restoration, shared names, pool boundaries, and malformed fallback.
    private static void classfiles(Path root) throws Exception {
        Path sources = Files.createDirectory(root.resolve("class-sources"));
        Path classes = Files.createDirectory(root.resolve("classes"));
        List<String> arguments = new ArrayList<>(List.of("--release", "8", "-encoding", "UTF-8", "-d", classes.toString()));
        for (int index = 0; index < 60; index++) {
            Path source = sources.resolve("Example" + index + ".java");
            Files.writeString(source, """
                    package shared;
                    /// Exercises string constants, descriptors, and Modified UTF-8 round trips.
                    public class Example%d {
                        /// Shared text reused by every class.
                        public static final String TEXT = "A shared string constant repeated across class files";
                        /// NUL, an unpaired surrogate, and a supplementary character.
                        public static final String EDGE = "\\000\\uD800\\uD83D\\uDE80";
                        /// Creates the fixture.
                        public Example%d() { }
                        /// Preserves a method descriptor containing an array class reference.
                        public Object[] identity(Object[] value) { return value; }
                    }
                    """.formatted(index, index));
            arguments.add(source.toString());
        }
        require(javax.tools.ToolProvider.getSystemJavaCompiler().run(null, null, null,
                arguments.toArray(String[]::new)) == 0, "Class fixture compilation failed");
        byte[] original = Files.readAllBytes(classes.resolve("shared/Example0.class"));
        StringPool strings = new StringPool(ReadLimits.DEFAULT);
        for (int i = 0; i < 130; i++) strings.intern("preexisting-" + i);
        byte[] transformed = ClassFileEncoder.transform(original, strings, ReadLimits.DEFAULT);
        require(transformed != null, "Class transform was not exercised");
        Input encodedPool = new Input(strings.encode());
        String[] values = new String[Math.toIntExact(encodedPool.uint())];
        for (int i = 0; i < values.length; i++) values[i] = new String(encodedPool.sized(), StandardCharsets.UTF_8);
        require(Arrays.equals(original, ClassFile.restore(transformed, values, original.length)), "CLASSFILE bytes changed");
        require(Arrays.asList(values).contains("shared") && Arrays.asList(values).contains("Example0"),
                "Class name was not split into shared components");
        int checkpoint = strings.size();
        require(ClassFileEncoder.transform(new byte[]{1, 2, 3}, strings, ReadLimits.DEFAULT) == null,
                "Malformed class was transformed");
        require(strings.size() == checkpoint, "Rejected class polluted the string pool");
        Files.write(classes.resolve("broken.class"), new byte[]{1, 2, 3});
        for (boolean compression : new boolean[]{true, false}) {
            long rawSize = 0;
            for (boolean enabled : new boolean[]{false, true}) {
                PackOptions options = new PackOptions(classes, root.resolve("classes-" + compression + "-" + enabled + ".janex"));
                options.mainClass = "shared.Example0";
                options.compression = compression;
                options.transformClassfiles = enabled;
                JanexWriter.write(options);
                if (!enabled) rawSize = Files.size(options.output);
                else require(Files.size(options.output) <= rawSize, "CLASSFILE increased the package size");
                try (JanexReader reader = new JanexReader(options.output)) {
                    ResourcePlan plan = reader.launch("main").resources;
                    boolean any = false;
                    try (var paths = Files.walk(classes)) {
                        for (Path file : paths.filter(Files::isRegularFile).toList()) {
                            String name = classes.relativize(file).toString().replace('\\', '/');
                            require(Arrays.equals(Files.readAllBytes(file), content(plan, name)), "Class resource changed: " + name);
                            any |= plan.roots().get(0).files().get(name).transforms().length != 0;
                        }
                    }
                    require(any == enabled, "CLASSFILE switch was not exercised");
                }
                if (enabled) {
                    PackOptions copy = new PackOptions(classes, root.resolve("classes-copy-" + compression + ".janex"));
                    copy.mainClass = options.mainClass;
                    copy.compression = compression;
                    JanexWriter.write(copy);
                    require(Arrays.equals(Files.readAllBytes(options.output), Files.readAllBytes(copy.output)),
                            "CLASSFILE packaging is not reproducible");
                }
            }
        }
    }

    /// Checks native compression levels against the portable decoder and preserves failure boundaries.
    private static void compressionLevels(Path root) throws Exception {
        Path directory = root.resolve("compression");
        for (int level : new int[]{-3, 0, 9, 15, 19, 22}) {
            PackOptions options = new PackOptions(directory, root.resolve("level-" + level + ".janex"));
            options.mainClass = "example.Main";
            options.compressionLevel = level;
            JanexWriter.write(options);
            try (JanexReader reader = new JanexReader(options.output)) {
                ResourcePlan plan = reader.launch("main").resources;
                try (var paths = Files.list(directory)) {
                    for (Path file : paths.toList()) {
                        require(Arrays.equals(Files.readAllBytes(file), content(plan, file.getFileName().toString())),
                                "Portable decoder failed at compression level " + level + ": " + file);
                    }
                }
            }
            PackOptions copy = new PackOptions(directory, root.resolve("level-copy-" + level + ".janex"));
            copy.mainClass = options.mainClass;
            copy.compressionLevel = level;
            JanexWriter.write(copy);
            require(Arrays.equals(Files.readAllBytes(options.output), Files.readAllBytes(copy.output)),
                    "Compression level is not reproducible: " + level);
        }
        for (int invalid : new int[]{Integer.MIN_VALUE, Integer.MAX_VALUE}) {
            PackOptions options = new PackOptions(directory, root.resolve("invalid-level-" + invalid + ".janex"));
            options.mainClass = "example.Main";
            options.compressionLevel = invalid;
            fails(() -> JanexWriter.write(options));
            require(!Files.exists(options.output), "Invalid compression level published output");
            options.compression = false;
            JanexWriter.write(options);
            try (JanexReader reader = new JanexReader(options.output)) {
                require(Arrays.equals(Files.readAllBytes(directory.resolve("random")), content(reader.launch("main").resources, "random")),
                        "Disabled compression did not ignore its unused level");
            }
        }
    }

    /// Checks independent root pools, exact restoration, and deterministic output.
    private static void rootStrings(Path root) throws Exception {
        List<Path> jars = new ArrayList<>();
        for (int part = 0; part < 3; part++) {
            Path jar = root.resolve("shared-" + part + ".jar");
            jars.add(jar);
            try (ZipOutputStream zip = new ZipOutputStream(Files.newOutputStream(jar))) {
                entry(zip, "META-INF/MANIFEST.MF", "Manifest-Version: 1.0\r\nMulti-Release: true\r\n\r\n");
                entry(zip, "root.txt", "root-" + part);
                for (int index = part * 20; index < (part + 1) * 20; index++) {
                    String name = "shared/Example" + index + ".class";
                    zip.putNextEntry(new ZipEntry(name));
                    zip.write(Files.readAllBytes(root.resolve("classes").resolve(name)));
                    zip.closeEntry();
                }
                entry(zip, "META-INF/versions/9/version.txt", "version-" + part);
                entry(zip, "META-INF/versions/99/future.txt", "unique-future-string-" + part);
            }
        }
        for (boolean compression : new boolean[]{false, true}) {
            PackOptions options = rootOptions(jars, root.resolve("root-pool-" + compression + ".janex"), compression);
            JanexWriter.write(options);
            try (JanexReader reader = new JanexReader(options.output)) {
                ResourcePlan plan = reader.launch("main").resources;
                require(plan.roots().size() == 3 && plan.pools().length == 3, "Roots did not retain independent string pools");
                Set<Integer> poolIds = new HashSet<>();
                for (int part = 0; part < jars.size(); part++) {
                    String name = jars.get(part).getFileName().toString();
                    var selected = plan.roots().stream().filter(value -> value.name().equals(name)).findFirst().orElseThrow();
                    int poolId = selected.files().get("shared/Example" + part * 20 + ".class").transforms()[0][1];
                    require(poolIds.add(poolId), "Distinct roots reused a string pool");
                    String[] strings = plan.pools()[poolId];
                    require(strings[0].isEmpty() && new HashSet<>(Arrays.asList(strings)).size() == strings.length,
                            "Root pool contains duplicate strings or a nonempty index zero");
                    require(Arrays.stream(strings).filter("A shared string constant repeated across class files"::equals).count() == 1,
                            "Class constant was not interned once within its root");
                    require(selected.module() == (part == 2), "Root pooling changed module-path membership");
                    require(new String(content(plan, selected, "root.txt"), StandardCharsets.UTF_8).equals("root-" + part),
                            "Root pooling merged duplicate resource paths");
                    require(new String(content(plan, selected, "version.txt"), StandardCharsets.UTF_8).equals("version-" + part),
                            "Root pooling changed Multi-Release selection");
                    require(!selected.files().containsKey("future.txt"), "Future resource layer was selected");
                    for (int index = part * 20; index < (part + 1) * 20; index++) {
                        String path = "shared/Example" + index + ".class";
                        require(Arrays.equals(Files.readAllBytes(root.resolve("classes").resolve(path)), content(plan, selected, path)),
                                "Cross-root CLASSFILE bytes changed: " + path);
                        int[][] transforms = selected.files().get(path).transforms();
                        require(transforms.length == 1 && transforms[0][1] == poolId, "Class did not use its root pool");
                    }
                }
            }
            options = rootOptions(jars, root.resolve("root-pool-copy-" + compression + ".janex"), compression);
            JanexWriter.write(options);
            require(Arrays.equals(Files.readAllBytes(root.resolve("root-pool-" + compression + ".janex")), Files.readAllBytes(options.output)),
                    "Root string pool output is not reproducible");
            options = rootOptions(jars, root.resolve("root-pool-no-transform-" + compression + ".janex"), compression);
            options.transformClassfiles = false;
            JanexWriter.write(options);
            try (JanexReader reader = new JanexReader(options.output)) {
                for (var selected : reader.launch("main").resources.roots()) {
                    require(selected.files().values().stream().allMatch(file -> file.transforms().length == 0),
                            "Root string pooling ignored disabled CLASSFILE transforms");
                }
            }
        }
        rootPoolLimits(root);
    }

    /// Creates equivalent options for repeated cross-root fixture writes.
    private static PackOptions rootOptions(List<Path> jars, Path output, boolean compression) {
        PackOptions options = new PackOptions(jars.get(0), output);
        options.mainClass = "shared.Example0";
        options.classPath.add(jars.get(1));
        options.modulePath.add(jars.get(2));
        options.compression = compression;
        return options;
    }

    /// Checks that collection limits apply independently to each resource root string pool.
    private static void rootPoolLimits(Path root) throws Exception {
        Path first = Files.createDirectory(root.resolve("bounded-first"));
        Path second = Files.createDirectory(root.resolve("bounded-second"));
        for (int index = 0; index < 24; index++) {
            Files.writeString(first.resolve("first-" + index + ".txt"), "first");
            Files.writeString(second.resolve("second-" + index + ".txt"), "second");
        }
        PackOptions options = new PackOptions(first, root.resolve("bounded-roots.janex"));
        options.mainClass = "example.Main";
        options.classPath.add(second);
        options.limits = new ReadLimits(1024 * 1024, 40, 64);
        JanexWriter.write(options);
        try (ContainerReader reader = new ContainerReader(options.output)) {
            for (int index = 0; index < 2; index++) {
                Path source = index == 0 ? first : second;
                BlobPool local = BlobPool.local(index + 1L, new Resources(source, options, new long[]{0}), options, false);
                require(Arrays.equals(local.bytes, reader.readSectionRange(index + 1L, 0, local.bytes.length)),
                        "Root pool differs from its independent encoding");
            }
        }
        try (JanexReader reader = new JanexReader(options.output)) {
            ResourcePlan plan = reader.launch("main").resources;
            require(new String(content(plan, plan.roots().get(1), "second-23.txt"), StandardCharsets.UTF_8).equals("second"),
                    "Independent root pooling changed resource content");
        }
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
        contents.put("overhead-tie", new byte[20]);
        contents.put("overhead-saving", new byte[21]);
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

            BlobPool pool = BlobPool.local(1, new Resources(directory, options, new long[]{0}), options, false);
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

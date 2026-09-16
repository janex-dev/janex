// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.*;
import java.nio.file.*;
import java.nio.file.attribute.BasicFileAttributes;
import java.nio.file.attribute.PosixFilePermission;
import java.util.*;
import java.util.jar.Manifest;

import org.glavo.janex.reader.internal.Input;
import org.glavo.janex.reader.internal.JarArchive;

/// Imports bounded resource trees without extracting archives or following directory links.
final class Resources {
    /// Original JAR filename, or resources.jar for a directory.
    final String name;
    /// Base manifest, or null when absent.
    final Manifest manifest;
    /// Unconditional tree followed by increasing Java feature-version trees.
    final SortedMap<Integer, SortedMap<String, Node>> layers = new TreeMap<>();

    /// One original resource node.
    /// @param bytes file contents, null for directories and links
    /// @param target relative link target, null for regular files and directories
    /// @param mode permission bits, or -1 when unspecified
    record Node(byte[] bytes, String target, int mode) {
        /// Returns whether this node is a directory.
        boolean directory() { return bytes == null && target == null; }
    }

    /// Imports one input and updates the shared aggregate byte counter.
    Resources(Path path, PackOptions options, long[] total) throws IOException {
        this(importPath(path, options, total), options, total, Map.of());
    }

    /// Imported names and bytes before manifest-driven Multi-Release layer interpretation.
    /// @param name root JAR identity
    /// @param entries owned mutable unlayered resource map
    private record Imported(String name, SortedMap<String, Node> entries) { }

    /// Imports the primary input, merging directory outputs and applying manifest overrides.
    static Resources primary(PackOptions options, long[] total) throws IOException {
        Imported imported;
        if (options.source != null) {
            imported = importPath(options.source, options, total);
        } else {
            Input.require(options.sourceName != null && options.sourceName.endsWith(".jar")
                    && !options.sourceName.contains("/") && !options.sourceName.contains("\\")
                    && options.sourceName.indexOf(0) < 0, "Invalid primary JAR filename");
            options.limits.elements(options.sourceDirectories.size());
            SortedMap<String, Node> entries = tree();
            for (Path path : options.sourceDirectories) {
                Input.require(Files.isDirectory(path, LinkOption.NOFOLLOW_LINKS), "Primary input must be a directory: " + path);
                Imported input = importPath(path, options, total);
                for (var entry : input.entries.entrySet()) {
                    Node previous = entries.get(entry.getKey());
                    if (previous != null && previous.directory() && entry.getValue().directory()) continue;
                    put(entries, entry.getKey(), entry.getValue(), options);
                }
            }
            imported = new Imported(options.sourceName, entries);
        }
        if (options.normalizeSourcePermissions) {
            imported.entries.replaceAll((path, node) -> node.target == null
                    ? new Node(node.bytes, null, node.directory() ? 0755 : 0644) : node);
        }
        return new Resources(imported, options, total, options.manifestAttributes);
    }

    /// Imports unlayered resources from one directory or archive.
    private static Imported importPath(Path path, PackOptions options, long[] total) throws IOException {
        SortedMap<String, Node> entries = tree();
        String name;
        BasicFileAttributes attributes = Files.readAttributes(path, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
        if (attributes.isDirectory()) {
            name = "resources.jar";
            directory(path, "", entries, options, total, 0);
        } else {
            Input.require(attributes.isRegularFile(), "Input must be a directory or regular JAR");
            name = path.getFileName().toString();
            Input.require(name.endsWith(".jar") && !name.contains("\\") && name.indexOf(0) < 0, "Invalid input JAR filename");
            for (JarArchive.Entry entry : JarArchive.read(read(path, options.limits.maxBytes()), options.limits)) {
                String resource = entry.name();
                boolean directory = resource.endsWith("/");
                if (directory) resource = resource.substring(0, resource.length() - 1);
                validatePath(resource, options);
                byte[] bytes = entry.bytes();
                addSize(total, bytes.length, options);
                String target = entry.kind() == 0120000 ? Input.utf8(bytes) : null;
                if (target != null) validateTarget(target);
                put(entries, resource, new Node(directory || target != null ? null : bytes, target,
                        target != null || entry.mode() < 0 ? -1 : entry.mode() & 07777), options);
            }
        }
        return new Imported(name, entries);
    }

    /// Applies primary manifest attributes, then resolves Multi-Release layers and path conflicts.
    private Resources(Imported imported, PackOptions options, long[] total,
                      Map<String, String> attributes) throws IOException {
        name = imported.name;
        SortedMap<String, Node> entries = imported.entries;
        Node main = entries.get("META-INF/MANIFEST.MF");
        Input.require(main == null || main.bytes != null, "JAR manifest must be a regular file");
        Manifest parsedManifest = main == null ? null : new Manifest(new ByteArrayInputStream(main.bytes));
        if (!attributes.isEmpty()) {
            if (parsedManifest == null) parsedManifest = new Manifest();
            Set<String> names = new TreeSet<>(String.CASE_INSENSITIVE_ORDER);
            for (var entry : new TreeMap<>(attributes).entrySet()) {
                String value = entry.getValue();
                Input.require(names.add(entry.getKey()), "Duplicate manifest attribute: " + entry.getKey());
                Input.require(value != null && value.indexOf('\r') < 0 && value.indexOf('\n') < 0
                        && value.indexOf(0) < 0, "Invalid manifest attribute value");
                try {
                    parsedManifest.getMainAttributes().putValue(entry.getKey(), value);
                } catch (IllegalArgumentException failure) {
                    throw new IOException("Invalid manifest attribute name: " + entry.getKey(), failure);
                }
            }
            if (parsedManifest.getMainAttributes().getValue("Manifest-Version") == null) {
                parsedManifest.getMainAttributes().putValue("Manifest-Version", "1.0");
            }
            ByteArrayOutputStream buffer = new ByteArrayOutputStream();
            parsedManifest.write(buffer);
            byte[] bytes = buffer.toByteArray();
            options.limits.bytes(bytes.length);
            if (main != null) total[0] -= main.bytes.length;
            addSize(total, bytes.length, options);
            entries.put("META-INF/MANIFEST.MF", new Node(bytes, null,
                    options.normalizeSourcePermissions ? 0644 : main == null ? -1 : main.mode));
            options.limits.elements(entries.size());
        }
        manifest = parsedManifest;
        boolean multi = manifest != null && "true".equalsIgnoreCase(manifest.getMainAttributes().getValue("Multi-Release"));
        layers.put(0, tree());
        for (var entry : entries.entrySet()) {
            String resource = entry.getKey();
            int version = 0;
            if (multi && resource.startsWith("META-INF/versions/")) {
                String suffix = resource.substring(18);
                int slash = suffix.indexOf('/');
                String number = slash < 0 ? suffix : suffix.substring(0, slash);
                if (number.matches("[1-9][0-9]*")) {
                    try {
                        int parsed = Integer.parseInt(number);
                        if (parsed >= 9) {
                            version = parsed;
                            resource = slash < 0 ? "" : suffix.substring(slash + 1);
                            Input.require(!resource.equals("META-INF") && !resource.startsWith("META-INF/"),
                                    "Multi-Release JAR cannot version META-INF resources");
                            Input.require(!resource.isEmpty() || entry.getValue().directory(), "Invalid version root");
                        }
                    } catch (NumberFormatException ignored) {
                        // Names outside the supported Java feature range remain ordinary resources.
                    }
                }
            }
            put(layers.computeIfAbsent(version, ignored -> tree()), resource, entry.getValue(), options);
        }
        SortedMap<String, Node> merged = tree();
        Set<String> directories = new HashSet<>();
        for (var layer : layers.values()) {
            merged.putAll(layer);
            for (var entry : layer.entrySet()) {
                String resource = entry.getKey();
                if (entry.getValue().directory()) directories.add(resource);
                while (resource.contains("/")) {
                    resource = resource.substring(0, resource.lastIndexOf('/'));
                    directories.add(resource);
                }
            }
            for (String resource : directories) {
                Node node = merged.get(resource);
                Input.require(node == null || node.directory(), "Conflicting resource paths: " + resource);
            }
        }
        options.limits.elements(layers.size());
    }

    /// Creates a map ordered by encoded UTF-8 path bytes.
    static <T> SortedMap<String, T> tree() { return new TreeMap<>(Encoding::compare); }

    /// Reads one complete file with one byte of lookahead beyond its limit.
    static byte[] read(Path path, int limit) throws IOException {
        try (InputStream input = Files.newInputStream(path)) {
            byte[] bytes = input.readNBytes(limit == Integer.MAX_VALUE ? limit : limit + 1);
            Input.require(bytes.length <= limit, "Input byte limit exceeded");
            return bytes;
        }
    }

    /// Walks one directory without following links and retains explicit empty directories.
    private static void directory(Path path, String relative, SortedMap<String, Node> entries,
                                  PackOptions options, long[] total, int depth) throws IOException {
        options.limits.depth(depth);
        put(entries, relative, new Node(null, null, permissions(path)), options);
        try (DirectoryStream<Path> children = Files.newDirectoryStream(path)) {
            for (Path child : children) {
                String resource = relative.isEmpty() ? child.getFileName().toString() : relative + "/" + child.getFileName();
                validatePath(resource, options);
                BasicFileAttributes attributes = Files.readAttributes(child, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
                if (attributes.isSymbolicLink()) {
                    Path link = Files.readSymbolicLink(child);
                    Input.require(!link.isAbsolute(), "Absolute symbolic-link target");
                    String target = link.toString();
                    if (File.separatorChar == '\\') target = target.replace('\\', '/');
                    validateTarget(target);
                    put(entries, resource, new Node(null, target, -1), options);
                } else if (attributes.isDirectory()) {
                    directory(child, resource, entries, options, total, depth + 1);
                } else {
                    Input.require(attributes.isRegularFile(), "Unsupported filesystem node");
                    long remaining = options.maxTotalBytes - total[0];
                    byte[] bytes = read(child, (int) Math.min(options.limits.maxBytes(), remaining));
                    addSize(total, bytes.length, options);
                    put(entries, resource, new Node(bytes, null, permissions(child)), options);
                }
            }
        }
    }

    /// Retains Unix special bits where available and POSIX permissions on other POSIX providers.
    private static int permissions(Path path) throws IOException {
        if (path.getFileSystem().supportedFileAttributeViews().contains("unix")) {
            return (Integer) Files.getAttribute(path, "unix:mode", LinkOption.NOFOLLOW_LINKS) & 07777;
        }
        if (!path.getFileSystem().supportedFileAttributeViews().contains("posix")) return -1;
        Set<PosixFilePermission> permissions = Files.getPosixFilePermissions(path, LinkOption.NOFOLLOW_LINKS);
        int bits = 0;
        for (PosixFilePermission permission : permissions) bits |= 1 << (8 - permission.ordinal());
        return bits;
    }

    /// Inserts one distinct node after checking the collection bound.
    private static void put(Map<String, Node> entries, String path, Node node, PackOptions options) throws IOException {
        options.limits.elements((long) entries.size() + 1);
        Input.require(entries.putIfAbsent(path, node) == null, "Duplicate resource path: " + path);
    }

    /// Counts imported bytes across every input before retaining them.
    private static void addSize(long[] total, int bytes, PackOptions options) throws IOException {
        Input.require(bytes <= options.maxTotalBytes - total[0], "Aggregate input byte limit exceeded");
        total[0] += bytes;
    }

    /// Validates a nonempty canonical resource path without native path normalization.
    private static void validatePath(String path, PackOptions options) throws IOException {
        options.limits.bytes(Encoding.utf8(path).length);
        Input.require(path.indexOf(0) < 0, "NUL in resource path");
        String[] components = path.split("/", -1);
        options.limits.depth(components.length);
        for (String component : components) {
            Input.require(!component.isEmpty() && !component.equals(".") && !component.equals(".."), "Invalid resource path");
        }
    }

    /// Checks the syntax of a root-relative symbolic-link target before reader resolution.
    private static void validateTarget(String target) throws IOException {
        Encoding.utf8(target);
        Input.require(target.indexOf(0) < 0, "NUL in symbolic-link target");
        for (String component : target.split("/", -1)) Input.require(!component.isEmpty(), "Invalid symbolic-link target");
    }
}

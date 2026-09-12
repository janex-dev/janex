// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.IOException;
import java.net.URI;
import java.net.URL;
import java.nio.ByteBuffer;
import java.nio.channels.*;
import java.nio.file.*;
import java.nio.file.attribute.*;
import java.util.*;
import java.util.stream.Stream;

/// Exercises the installed provider from an application through public Java 8 APIs.
public final class FileSystemTest {
    /// Prevents instantiation.
    private FileSystemTest() {}
    /// Requires a contract assertion to hold.
    private static void check(boolean condition) { if (!condition) throw new AssertionError("NIO contract mismatch"); }
    /// Runs an operation which can report an I/O failure.
    private interface Operation {
        /// Executes the operation under test.
        void run() throws Exception;
    }
    /// Requires the expected failure category without accepting unrelated exceptions.
    private static void fails(Class<? extends Exception> type, Operation operation) throws Exception {
        try { operation.run(); } catch (Exception failure) { if (type.isInstance(failure)) return; throw failure; }
        throw new AssertionError("Expected " + type.getSimpleName());
    }
    /// Tests URI conversion, path syntax, traversal, attributes, channels, mutation rejection, and remounting.
    /// @param resource URL of the seven-byte escaped-name resource in the integration fixture
    /// @throws Exception if a filesystem contract fails
    public static void run(URL resource) throws Exception {
        URI uri = resource.toURI();
        Path path = Paths.get(uri);
        FileSystem fs = path.getFileSystem();
        Path root = path.getParent();
        check(fs.isReadOnly() && fs.isOpen());
        check(path.equals(Paths.get(path.toUri())));
        check(Arrays.equals(Files.readAllBytes(path), "escaped".getBytes("UTF-8")));
        try (java.io.InputStream input = path.toUri().toURL().openStream()) { check(input.read() == 'e'); }
        check(Files.isDirectory(root));
        try (java.io.InputStream input = root.toUri().toURL().openStream()) { check(input.read() == -1); }
        check(Files.isDirectory(root.resolve("sample")));
        check(Files.size(path) == 7);
        check(Files.isSameFile(path, root.resolve("./" + path.getFileName())));
        check(Files.isSameFile(root.resolve("missing"), root.resolve("missing")));
        check(!Files.isSameFile(path, Paths.get("native")));
        check(root.resolve("sample/../" + path.getFileName()).toRealPath().equals(path));
        check(fs.getPath("a/b").relativize(fs.getPath("a/c")).toString().equals("../c"));
        check(fs.getPath("/").getNameCount() == 0 && fs.getPath("").getNameCount() == 1);
        check(fs.getPath("a//b/").toString().equals("a/b"));
        check(fs.getPath("a\\b").getNameCount() == 1);
        check(fs.getPath("/../../").normalize().toString().equals("/"));
        check(fs.getPath("a/b").startsWith("a") && !fs.getPath("a/b").startsWith("ab"));
        check(fs.getPath("a/b").endsWith("b") && !fs.getPath("a/b").endsWith("/b"));
        check(fs.getPathMatcher("glob:**/*.{txt,bin}").matches(path));
        check(fs.getPathMatcher("glob:[!x]?").matches(fs.getPath("ab")));
        check(!fs.getPathMatcher("glob:*").matches(fs.getPath("a/b")));
        check(fs.getPathMatcher("regex:.*\\.txt").matches(path));
        fails(UnsupportedOperationException.class, () -> path.toFile());
        fails(ClassCastException.class, () -> path.compareTo(Paths.get("native")));
        fails(IllegalArgumentException.class, () -> Paths.get(new URI("janex://host/0/x")));
        fails(IllegalArgumentException.class, () -> Paths.get(new URI(uri + "?query")));
        fails(IllegalArgumentException.class, () -> Paths.get(new URI(uri + "#fragment")));
        fails(FileSystemAlreadyExistsException.class, () -> FileSystems.newFileSystem(uri, Collections.emptyMap()));
        fails(NoSuchFileException.class, () -> Files.readAllBytes(root.resolve("missing")));
        fails(NotDirectoryException.class, () -> Files.newDirectoryStream(path));
        fails(IOException.class, () -> Files.newByteChannel(root));
        fails(ReadOnlyFileSystemException.class, () -> Files.write(path, new byte[0]));
        fails(ReadOnlyFileSystemException.class, () -> Files.createDirectory(root.resolve("new")));
        fails(ReadOnlyFileSystemException.class, () -> Files.delete(path));
        fails(ReadOnlyFileSystemException.class, () -> Files.copy(path, root.resolve("copy")));
        fails(ReadOnlyFileSystemException.class, () -> Files.setLastModifiedTime(path, FileTime.fromMillis(1)));
        check(!Files.isWritable(path) && !Files.isExecutable(path));
        BasicFileAttributes attrs = Files.readAttributes(path, BasicFileAttributes.class);
        check(attrs.isRegularFile() && !attrs.isDirectory() && !attrs.isSymbolicLink() && attrs.size() == 7);
        check(Files.getFileAttributeView(path, BasicFileAttributeView.class).readAttributes().size() == 7);
        check(Files.readAttributes(path, "basic:size,isRegularFile").size() == 2);
        check(Files.readAttributes(path, "janex:*").containsKey("permissions"));
        fails(IllegalArgumentException.class, () -> Files.readAttributes(path, "basic:unknown"));
        check(Files.getFileStore(path).isReadOnly() && Files.getFileStore(path).getUsableSpace() == 0);
        try (Stream<Path> walk = Files.walk(root)) {
            List<Path> paths = walk.collect(java.util.stream.Collectors.toList());
            check(paths.contains(path) && new HashSet<Path>(paths).size() == paths.size());
        }
        try (DirectoryStream<Path> listing = Files.newDirectoryStream(root, "*.txt")) {
            Iterator<Path> iterator = listing.iterator();
            check(iterator.hasNext());
            fails(IllegalStateException.class, () -> listing.iterator());
            while (iterator.hasNext()) check(iterator.next().getFileName().toString().endsWith(".txt"));
        }
        try (DirectoryStream<Path> listing = Files.newDirectoryStream(root, candidate -> { throw new IOException("filter failure"); })) {
            fails(DirectoryIteratorException.class, () -> listing.iterator().hasNext());
        }
        DirectoryStream<Path> prefetched = Files.newDirectoryStream(root);
        Iterator<Path> remaining = prefetched.iterator();
        check(remaining.hasNext());
        prefetched.close();
        check(remaining.next() != null && !remaining.hasNext());
        SeekableByteChannel channel = Files.newByteChannel(path);
        ByteBuffer buffer = ByteBuffer.allocate(2);
        channel.position(2); check(channel.read(buffer) == 2 && channel.position() == 4);
        check(buffer.array()[0] == 'c' && buffer.array()[1] == 'a');
        channel.position(Long.MAX_VALUE); buffer.clear(); check(channel.read(buffer) == -1);
        check(channel.read(ByteBuffer.allocate(0)) == 0);
        fails(NonWritableChannelException.class, () -> channel.write(ByteBuffer.allocate(1)));
        Path copy = Files.createTempFile("janex-nio-", ".txt");
        try { Files.copy(path, copy, StandardCopyOption.REPLACE_EXISTING); check(Arrays.equals(Files.readAllBytes(copy), Files.readAllBytes(path))); }
        finally { Files.delete(copy); }
        fs.close(); fs.close();
        check(!fs.isOpen() && !channel.isOpen());
        fails(ClosedChannelException.class, () -> channel.position()); channel.close();
        fails(ClosedFileSystemException.class, () -> Files.readAllBytes(path));
        fails(FileSystemNotFoundException.class, () -> Paths.get(uri));
        try (java.io.InputStream input = resource.openStream()) { check(input.read() == 'e'); }
        try (FileSystem mounted = FileSystems.newFileSystem(uri, Collections.emptyMap())) {
            check(mounted != fs && Arrays.equals(Files.readAllBytes(Paths.get(uri)), "escaped".getBytes("UTF-8")));
        }
    }
}

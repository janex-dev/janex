// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.File;
import java.io.IOException;
import java.net.URI;
import java.net.URISyntaxException;
import java.nio.file.*;
import java.util.*;

/// An immutable, case-sensitive path whose only separator is `/`.
final class JanexPath implements Path {
    /// File-system identity retained even after closure.
    final JanexFileSystem fileSystem;
    /// Path with redundant separators removed, retaining dot components.
    private final String value;

    /// Creates a lexical path, rejecting NUL and preserving backslashes as name characters.
    JanexPath(JanexFileSystem fileSystem, String value) {
        if (value.indexOf('\0') >= 0) throw new InvalidPathException(value, "NUL in path");
        this.fileSystem = fileSystem;
        this.value = value.replaceAll("/+", "/").replaceAll("(?<!^)/$", "");
    }

    /// Returns path components; an empty relative path has one empty name.
    private String[] names() {
        return value.equals("/") ? new String[0] : (isAbsolute() ? value.substring(1) : value).split("/", -1);
    }

    /// Requires a path from this exact file-system instance.
    private JanexPath same(Path other) {
        Objects.requireNonNull(other);
        if (!(other instanceof JanexPath) || other.getFileSystem() != fileSystem) throw new ProviderMismatchException();
        return (JanexPath) other;
    }

    /// Returns the owning file system.
    @Override public JanexFileSystem getFileSystem() { return fileSystem; }
    /// Tests whether the path starts at the virtual root.
    @Override public boolean isAbsolute() { return value.startsWith("/"); }
    /// Returns the virtual root for an absolute path, otherwise null.
    @Override public Path getRoot() { return isAbsolute() ? fileSystem.getPath("/") : null; }
    /// Returns the final name, or null for the virtual root.
    @Override public Path getFileName() { return value.equals("/") ? null : fileSystem.getPath(value.substring(value.lastIndexOf('/') + 1)); }
    /// Returns the lexical parent, or null when no parent is present.
    @Override public Path getParent() {
        int slash = value.lastIndexOf('/');
        return value.equals("/") || slash < 0 ? null : fileSystem.getPath(slash == 0 ? "/" : value.substring(0, slash));
    }
    /// Returns the number of name components.
    @Override public int getNameCount() { return names().length; }
    /// Returns one relative name component.
    @Override public Path getName(int index) { return subpath(index, index + 1); }
    /// Returns a nonempty range of relative name components.
    @Override public Path subpath(int begin, int end) {
        String[] names = names();
        if (begin < 0 || end > names.length || begin >= end) throw new IllegalArgumentException("Invalid subpath range");
        return fileSystem.getPath(String.join("/", Arrays.copyOfRange(names, begin, end)));
    }
    /// Tests a lexical component prefix within this file system.
    @Override public boolean startsWith(Path other) {
        if (!(other instanceof JanexPath) || other.getFileSystem() != fileSystem || isAbsolute() != other.isAbsolute()) return false;
        String prefix = other.toString();
        return value.equals(prefix) || (!prefix.isEmpty() && value.startsWith(prefix.endsWith("/") ? prefix : prefix + "/"));
    }
    /// Tests a parsed component prefix.
    @Override public boolean startsWith(String other) { return startsWith(fileSystem.getPath(other)); }
    /// Tests a lexical component suffix within this file system.
    @Override public boolean endsWith(Path other) {
        if (!(other instanceof JanexPath) || other.getFileSystem() != fileSystem) return false;
        if (other.isAbsolute()) return equals(other);
        String suffix = other.toString();
        return value.equals(suffix) || (!suffix.isEmpty() && value.endsWith("/" + suffix));
    }
    /// Tests a parsed component suffix.
    @Override public boolean endsWith(String other) { return endsWith(fileSystem.getPath(other)); }
    /// Removes dot components lexically; absolute paths cannot ascend above the virtual root.
    @Override public JanexPath normalize() {
        List<String> result = new ArrayList<String>();
        for (String name : names()) {
            if (name.isEmpty() || name.equals(".")) continue;
            if (name.equals("..") && !result.isEmpty() && !result.get(result.size() - 1).equals("..")) result.remove(result.size() - 1);
            else if (!name.equals("..") || !isAbsolute()) result.add(name);
        }
        return fileSystem.getPath((isAbsolute() ? "/" : "") + String.join("/", result));
    }
    /// Resolves a path lexically, returning an absolute argument unchanged.
    @Override public Path resolve(Path other) {
        JanexPath path = same(other);
        if (path.isAbsolute() || value.isEmpty()) return path;
        if (path.value.isEmpty()) return this;
        return fileSystem.getPath(value + "/" + path.value);
    }
    /// Resolves a parsed path.
    @Override public Path resolve(String other) { return resolve(fileSystem.getPath(other)); }
    /// Resolves against the parent, or returns the argument when no parent exists.
    @Override public Path resolveSibling(Path other) { same(other); Path parent = getParent(); return parent == null ? other : parent.resolve(other); }
    /// Resolves a parsed sibling path.
    @Override public Path resolveSibling(String other) { return resolveSibling(fileSystem.getPath(other)); }
    /// Returns a lexical relative path between normalized paths of the same kind.
    @Override public Path relativize(Path other) {
        JanexPath target = same(other).normalize();
        JanexPath base = normalize();
        if (base.isAbsolute() != target.isAbsolute()) throw new IllegalArgumentException("Different path kinds");
        if (base.value.isEmpty()) return target;
        String[] left = base.names();
        String[] right = target.value.isEmpty() ? new String[0] : target.names();
        int common = 0;
        while (common < left.length && common < right.length && left[common].equals(right[common])) common++;
        List<String> parts = new ArrayList<String>();
        for (int i = common; i < left.length; i++) {
            if (left[i].equals("..")) throw new IllegalArgumentException("Unresolved parent in base path");
            parts.add("..");
        }
        parts.addAll(Arrays.asList(right).subList(common, right.length));
        return fileSystem.getPath(String.join("/", parts));
    }
    /// Returns an escaped absolute URI recognized by the Janex provider and URL handler.
    @Override public URI toUri() {
        try { return new URI("janex", null, toAbsolutePath().toString(), null); }
        catch (URISyntaxException invalid) { throw new IllegalArgumentException(invalid); }
    }
    /// Resolves relative paths against the virtual root, independently of the native working directory.
    @Override public JanexPath toAbsolutePath() { return isAbsolute() ? this : fileSystem.getPath("/" + value); }
    /// Returns the normalized existing path; Host-resolved aliases have no remaining symbolic links.
    @Override public Path toRealPath(LinkOption... options) throws IOException {
        JanexFileSystemProvider.links(options);
        JanexPath path = toAbsolutePath().normalize();
        fileSystem.node(path);
        return path;
    }
    /// Rejects conversion because resources have no native file representation.
    @Override public File toFile() { throw new UnsupportedOperationException("Janex paths are not native files"); }
    /// Rejects watching an immutable snapshot.
    @Override public WatchKey register(WatchService watcher, WatchEvent.Kind<?>[] events, WatchEvent.Modifier... modifiers) { throw new UnsupportedOperationException("Immutable file system"); }
    /// Rejects watching an immutable snapshot.
    @Override public WatchKey register(WatchService watcher, WatchEvent.Kind<?>... events) { throw new UnsupportedOperationException("Immutable file system"); }
    /// Iterates immutable relative name paths.
    @Override public Iterator<Path> iterator() {
        List<Path> paths = new ArrayList<Path>();
        for (String name : names()) paths.add(fileSystem.getPath(name));
        return Collections.unmodifiableList(paths).iterator();
    }
    /// Compares paths lexically after requiring file-system identity.
    @Override public int compareTo(Path other) {
        Objects.requireNonNull(other);
        if (!(other instanceof JanexPath) || other.getFileSystem() != fileSystem) throw new ClassCastException("Different file system");
        return value.compareTo(((JanexPath) other).value);
    }
    /// Tests file-system identity and the unnormalized lexical representation.
    @Override public boolean equals(Object other) { return other instanceof JanexPath && ((JanexPath) other).fileSystem == fileSystem && value.equals(((JanexPath) other).value); }
    /// Hashes file-system identity and lexical representation.
    @Override public int hashCode() { return 31 * System.identityHashCode(fileSystem) + value.hashCode(); }
    /// Returns the slash-separated lexical representation.
    @Override public String toString() { return value; }
}

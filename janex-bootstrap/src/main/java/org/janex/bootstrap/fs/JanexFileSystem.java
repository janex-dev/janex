// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap.fs;

import java.io.IOException;
import java.nio.file.*;
import java.nio.file.attribute.UserPrincipalLookupService;
import java.util.*;
import java.util.regex.Pattern;

import org.janex.bootstrap.loader.ResourceIndex;

/// A closeable read-only view of the launch snapshot; closing it does not close the loader.
final class JanexFileSystem extends FileSystem {
    /// Provider which created this view.
    private final JanexFileSystemProvider provider;
    /// Snapshot owner, shared with class and module loading.
    final ResourceIndex index;
    /// Absolute normalized names and their resource descriptors; null descriptors denote synthetic directories.
    final Map<String, ResourceIndex.Resource> entries = new LinkedHashMap<String, ResourceIndex.Resource>();
    /// Direct child names for each directory.
    private final Map<String, List<String>> children = new HashMap<String, List<String>>();
    /// Whether this view remains usable for I/O.
    private volatile boolean open = true;

    /// Creates the namespace used by resource URLs without decoding file contents.
    JanexFileSystem(JanexFileSystemProvider provider, ResourceIndex index) {
        this.provider = provider;
        this.index = index;
        entries.put("/", null);
        for (int id = 0; id < index.roots().size(); id++) {
            ResourceIndex.Root root = index.roots().get(id);
            String prefix = "/" + id + "/" + root.name();
            entries.put("/" + id, null);
            for (Map.Entry<String, ResourceIndex.Resource> entry : root.files().entrySet()) {
                String name = entry.getKey();
                if (entry.getValue().isDirectory() && name.endsWith("/")) {
                    name = name.substring(0, name.length() - 1);
                }
                entries.put(prefix + (name.isEmpty() ? "" : "/" + name), entry.getValue());
            }
        }
        for (String name : new ArrayList<String>(entries.keySet())) {
            String parent = name;
            while ((parent = parentName(parent)) != null) {
                if (!entries.containsKey(parent)) {
                    entries.put(parent, null);
                }
            }
        }
        for (Map.Entry<String, ResourceIndex.Resource> entry : entries.entrySet()) {
            if (entry.getValue() == null || entry.getValue().isDirectory()) {
                children.put(entry.getKey(), new ArrayList<String>());
            }
        }
        for (String name : entries.keySet()) {
            String parent = parentName(name);
            if (parent != null) {
                children.get(parent).add(name.substring(parent.equals("/") ? 1 : parent.length() + 1));
            }
        }
    }

    /// Returns the parent of a normalized absolute name, or null for the root.
    private static String parentName(String name) {
        int slash = name.lastIndexOf('/');
        return name.equals("/") ? null : slash == 0 ? "/" : name.substring(0, slash);
    }

    /// Rejects I/O after either this view or its underlying loader has closed.
    void ensureOpen() {
        if (!open || index.isClosed()) {
            throw new ClosedFileSystemException();
        }
    }

    /// Resolves an existing path, returning null for synthetic directories.
    ResourceIndex.Resource node(JanexPath path) throws IOException {
        ensureOpen();
        String name = path.toAbsolutePath().normalize().toString();
        if (!entries.containsKey(name)) {
            throw new NoSuchFileException(path.toString());
        }
        return entries.get(name);
    }

    /// Returns a snapshot of direct child names, rejecting non-directories.
    List<String> children(JanexPath path) throws IOException {
        node(path);
        List<String> names = children.get(path.toAbsolutePath().normalize().toString());
        if (names == null) {
            throw new NotDirectoryException(path.toString());
        }
        return new ArrayList<String>(names);
    }

    /// Returns this view's provider.
    @Override
    public JanexFileSystemProvider provider() {
        return provider;
    }

    /// Closes this view and its I/O handles; resources owned by the loader remain available.
    @Override
    public void close() {
        open = false;
    }

    /// Tests whether both this view and the snapshot remain open.
    @Override
    public boolean isOpen() {
        return open && !index.isClosed();
    }

    /// Returns true because the launch snapshot cannot be modified through this view.
    @Override
    public boolean isReadOnly() {
        return true;
    }

    /// Returns the platform-independent resource separator.
    @Override
    public String getSeparator() {
        return "/";
    }

    /// Returns the single virtual root.
    @Override
    public Iterable<Path> getRootDirectories() {
        ensureOpen();
        return Collections.<Path>singleton(getPath("/"));
    }

    /// Returns the single logical resource store.
    @Override
    public Iterable<FileStore> getFileStores() {
        ensureOpen();
        return Collections.<FileStore>singleton(new JanexFileSystemProvider.Store(this));
    }

    /// Returns basic attributes and Janex metadata attributes.
    @Override
    public Set<String> supportedFileAttributeViews() {
        return Collections.unmodifiableSet(new HashSet<String>(Arrays.asList("basic", "janex")));
    }

    /// Parses a resource path without touching its contents or requiring it to exist.
    @Override
    public JanexPath getPath(String first, String... more) {
        StringBuilder path = new StringBuilder(Objects.requireNonNull(first));
        for (String part : more) {
            Objects.requireNonNull(part);
            if (!part.isEmpty()) {
                if (path.length() > 0) {
                    path.append('/');
                }
                path.append(part);
            }
        }
        return new JanexPath(this, path.toString());
    }

    /// Creates a case-sensitive regex or slash-based glob matcher.
    @Override
    public PathMatcher getPathMatcher(String expression) {
        int colon = expression.indexOf(':');
        if (colon <= 0) {
            throw new IllegalArgumentException("Missing matcher syntax");
        }
        String syntax = expression.substring(0, colon);
        String pattern = expression.substring(colon + 1);
        final Pattern compiled;
        if (syntax.equals("regex")) {
            compiled = Pattern.compile(pattern);
        } else if (syntax.equals("glob")) {
            compiled = Pattern.compile(glob(pattern));
        } else {
            throw new UnsupportedOperationException("Unknown matcher syntax: " + syntax);
        }
        return path -> path.getFileSystem() == this && compiled.matcher(path.toString()).matches();
    }

    /// Converts glob metacharacters using `/` rather than the host's native separator.
    private static String glob(String value) {
        StringBuilder regex = new StringBuilder();
        boolean group = false;
        for (int i = 0; i < value.length(); i++) {
            char ch = value.charAt(i);
            if (ch == '*') {
                if (i + 1 < value.length() && value.charAt(i + 1) == '*') {
                    regex.append(".*");
                    i++;
                } else {
                    regex.append("[^/]*");
                }
            } else if (ch == '?') {
                regex.append("[^/]");
            } else if (ch == '{') {
                if (group) {
                    throw new java.util.regex.PatternSyntaxException("Nested group", value, i);
                }
                group = true;
                regex.append("(?:");
            } else if (ch == '}') {
                if (!group) {
                    throw new java.util.regex.PatternSyntaxException("Unmatched group", value, i);
                }
                group = false;
                regex.append(')');
            } else if (ch == ',' && group) {
                regex.append('|');
            } else if (ch == '\\') {
                if (++i == value.length()) {
                    throw new java.util.regex.PatternSyntaxException("Trailing escape", value, i);
                }
                regex.append(Pattern.quote(value.substring(i, i + 1)));
            } else if (ch == '[') {
                int end = value.indexOf(']', i + 1);
                if (end < 0) {
                    throw new java.util.regex.PatternSyntaxException("Unclosed character class", value, i);
                }
                String set = value.substring(i + 1, end);
                if (set.startsWith("!")) {
                    set = "^" + set.substring(1);
                } else if (set.startsWith("^")) {
                    set = "\\" + set;
                }
                regex.append("[[").append(set.replace("&", "\\&")).append("]&&[^/]]");
                i = end;
            } else {
                regex.append(Pattern.quote(String.valueOf(ch)));
            }
        }
        if (group) {
            throw new java.util.regex.PatternSyntaxException("Unclosed group", value, value.length());
        }
        return regex.toString();
    }

    /// Rejects principal lookup because resource metadata does not identify native owners.
    @Override
    public UserPrincipalLookupService getUserPrincipalLookupService() {
        throw new UnsupportedOperationException("No native principals");
    }

    /// Rejects watching an immutable snapshot.
    @Override
    public WatchService newWatchService() {
        throw new UnsupportedOperationException("Immutable file system");
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.IOException;
import java.math.BigInteger;
import java.net.URI;
import java.nio.ByteBuffer;
import java.nio.channels.*;
import java.nio.file.*;
import java.nio.file.attribute.*;
import java.nio.file.spi.FileSystemProvider;
import java.time.Instant;
import java.util.*;
import java.util.concurrent.TimeUnit;

/// Exposes the active launch snapshot through `janex:` URIs and the standard read-only NIO APIs.
///
/// The provider does not open arbitrary packages. Each mounted view borrows the active loader's
/// snapshot. Closing a view invalidates its paths and channels for I/O, but does not close the loader.
/// A closed view can be replaced with [FileSystems#newFileSystem(URI, Map)]. Resource links have
/// already been resolved by the Host. Basic times default to the epoch when absent; the `janex`
/// attribute view additionally returns exact nullable nanosecond timestamps and permission bits.
public final class JanexFileSystemProvider extends FileSystemProvider {
    /// Creates a provider without requiring an active Janex launch.
    public JanexFileSystemProvider() {
    }

    /// Returns the resource URI scheme.
    @Override
    public String getScheme() {
        return "janex";
    }

    /// Validates an absolute resource URI without decoding its resource.
    private static void uri(URI uri) {
        if (!"janex".equalsIgnoreCase(uri.getScheme()) || uri.getRawAuthority() != null
                || uri.getRawQuery() != null || uri.getRawFragment() != null
                || uri.getPath() == null || !uri.getPath().startsWith("/")) {
            throw new IllegalArgumentException("Invalid Janex URI: " + uri);
        }
    }

    /// Creates a new view only when the previous launch view is closed; environment options are unsupported.
    @Override
    public FileSystem newFileSystem(URI uri, Map<String, ?> env) {
        uri(uri);
        if (!Objects.requireNonNull(env).isEmpty()) {
            throw new IllegalArgumentException("No Janex mount options");
        }
        return ResourceLoader.fileSystem(this, true);
    }

    /// Returns the open launch view, creating its initial mount lazily.
    @Override
    public FileSystem getFileSystem(URI uri) {
        uri(uri);
        return ResourceLoader.fileSystem(this, false);
    }

    /// Returns the path corresponding to a class-loader or module-reader resource URI.
    @Override
    public Path getPath(URI uri) {
        return getFileSystem(uri).getPath(uri.getPath());
    }

    /// Requires a Janex path without confusing paths from other providers.
    private static JanexPath path(Path path) {
        Objects.requireNonNull(path);
        if (!(path instanceof JanexPath)) {
            throw new ProviderMismatchException();
        }
        return (JanexPath) path;
    }

    /// Checks link options; the snapshot contains resolved aliases rather than live links.
    static void links(LinkOption... options) {
        for (LinkOption option : options) {
            Objects.requireNonNull(option);
        }
    }

    /// Opens an independently positioned read channel; write or creation options are rejected.
    @Override
    public SeekableByteChannel newByteChannel(Path path, Set<? extends OpenOption> options, FileAttribute<?>... attrs) throws IOException {
        JanexPath target = path(path);
        target.fileSystem.ensureOpen();
        for (OpenOption option : options) {
            Objects.requireNonNull(option);
            if (option == StandardOpenOption.READ || option == LinkOption.NOFOLLOW_LINKS) {
                continue;
            }
            if (option instanceof StandardOpenOption) {
                throw new ReadOnlyFileSystemException();
            }
            throw new UnsupportedOperationException("Unsupported open option: " + option);
        }
        if (attrs.length != 0) {
            throw new UnsupportedOperationException("Initial attributes on read-only channel");
        }
        ResourceIndex.Resource resource = target.fileSystem.node(target);
        if (resource == null || resource.id == -1) {
            throw new FileSystemException(path.toString(), null, "Is a directory");
        }
        return new Channel(target.fileSystem, resource.read());
    }

    /// Opens a single-iterator filtered snapshot of a directory's immediate children.
    @Override
    public DirectoryStream<Path> newDirectoryStream(Path dir, DirectoryStream.Filter<? super Path> filter) throws IOException {
        JanexPath directory = path(dir);
        return new Listing(directory, directory.fileSystem.children(directory), Objects.requireNonNull(filter));
    }

    /// Rejects directory creation in the immutable snapshot.
    @Override
    public void createDirectory(Path dir, FileAttribute<?>... attrs) {
        path(dir).fileSystem.ensureOpen();
        throw new ReadOnlyFileSystemException();
    }

    /// Rejects deletion in the immutable snapshot.
    @Override
    public void delete(Path path) {
        path(path).fileSystem.ensureOpen();
        throw new ReadOnlyFileSystemException();
    }

    /// Rejects a copy whose destination belongs to this read-only provider.
    @Override
    public void copy(Path source, Path target, CopyOption... options) {
        path(source).fileSystem.ensureOpen();
        path(target).fileSystem.ensureOpen();
        throw new ReadOnlyFileSystemException();
    }

    /// Rejects moving entries in the immutable snapshot.
    @Override
    public void move(Path source, Path target, CopyOption... options) {
        copy(source, target, options);
    }

    /// Returns true for equal paths; otherwise compares existing normalized paths within the same view.
    @Override
    public boolean isSameFile(Path first, Path second) throws IOException {
        JanexPath left = path(first);
        if (left.equals(second)) {
            return true;
        }
        if (!(second instanceof JanexPath) || second.getFileSystem() != left.fileSystem) {
            return false;
        }
        return left.toRealPath().equals(second.toRealPath());
    }

    /// Returns false for an existing entry; the format has no hidden-file flag.
    @Override
    public boolean isHidden(Path path) throws IOException {
        JanexPath target = path(path);
        target.fileSystem.node(target);
        return false;
    }

    /// Returns the logical read-only store containing an existing entry.
    @Override
    public FileStore getFileStore(Path path) throws IOException {
        JanexPath target = path(path);
        target.fileSystem.node(target);
        return new Store(target.fileSystem);
    }

    /// Checks existence and allows only read access; stored permissions do not grant write access.
    @Override
    public void checkAccess(Path path, AccessMode... modes) throws IOException {
        JanexPath target = path(path);
        target.fileSystem.node(target);
        for (AccessMode mode : modes) {
            if (Objects.requireNonNull(mode) != AccessMode.READ) {
                throw new AccessDeniedException(path.toString());
            }
        }
    }

    /// Returns a basic attribute view, or null when the requested view type is unsupported.
    @Override
    public <V extends FileAttributeView> V getFileAttributeView(final Path path, Class<V> type, LinkOption... options) {
        path(path);
        Objects.requireNonNull(type);
        links(options);
        if (type != BasicFileAttributeView.class) {
            return null;
        }
        return type.cast(new BasicFileAttributeView() {
            /// Returns the standard basic view name.
            @Override
            public String name() {
                return "basic";
            }

            /// Reads attributes without decoding file contents.
            @Override
            public BasicFileAttributes readAttributes() throws IOException {
                return JanexFileSystemProvider.this.readAttributes(path, BasicFileAttributes.class);
            }

            /// Rejects timestamp mutation.
            @Override
            public void setTimes(FileTime modified, FileTime accessed, FileTime created) {
                path(path).fileSystem.ensureOpen();
                throw new ReadOnlyFileSystemException();
            }
        });
    }

    /// Returns immutable basic attributes of an existing entry without reading its payload.
    @Override
    public <A extends BasicFileAttributes> A readAttributes(Path path, Class<A> type, LinkOption... options) throws IOException {
        JanexPath target = path(path);
        Objects.requireNonNull(type);
        links(options);
        if (type != BasicFileAttributes.class) {
            throw new UnsupportedOperationException("Unsupported attribute type: " + type);
        }
        return type.cast(new Attributes(target, target.fileSystem.node(target)));
    }

    /// Reads selected `basic` or `janex` attributes; unknown names are rejected.
    @Override
    public Map<String, Object> readAttributes(Path path, String attributes, LinkOption... options) throws IOException {
        int colon = attributes.indexOf(':');
        String view = colon < 0 ? "basic" : attributes.substring(0, colon);
        String names = attributes.substring(colon + 1);
        if (!view.equals("basic") && !view.equals("janex")) {
            throw new UnsupportedOperationException("Unsupported attribute view: " + view);
        }
        Attributes value = (Attributes) readAttributes(path, BasicFileAttributes.class, options);
        Map<String, Object> all = new LinkedHashMap<String, Object>();
        all.put("size", value.size());
        all.put("creationTime", value.creationTime());
        all.put("lastModifiedTime", value.lastModifiedTime());
        all.put("lastAccessTime", value.lastAccessTime());
        all.put("isRegularFile", value.isRegularFile());
        all.put("isDirectory", value.isDirectory());
        all.put("isSymbolicLink", false);
        all.put("isOther", false);
        all.put("fileKey", value.fileKey());
        if (view.equals("janex")) {
            all.put("creationTimeNanos", value.nanos(0));
            all.put("lastModifiedTimeNanos", value.nanos(1));
            all.put("lastAccessTimeNanos", value.nanos(2));
            all.put("permissions", value.resource == null || value.resource.permissions < 0 ? null : value.resource.permissions);
        }
        if (names.equals("*")) {
            return all;
        }
        Map<String, Object> selected = new LinkedHashMap<String, Object>();
        for (String name : names.split(",", -1)) {
            if (!all.containsKey(name)) {
                throw new IllegalArgumentException("Unknown attribute: " + name);
            }
            selected.put(name, all.get(name));
        }
        return selected;
    }

    /// Rejects attribute mutation.
    @Override
    public void setAttribute(Path path, String attribute, Object value, LinkOption... options) {
        path(path).fileSystem.ensureOpen();
        links(options);
        throw new ReadOnlyFileSystemException();
    }

    /// A read-only channel over private cached bytes; closing it never closes the snapshot.
    private static final class Channel implements SeekableByteChannel {
        /// View whose closure invalidates channel operations.
        private final JanexFileSystem fileSystem;
        /// Borrowed immutable decoded bytes, released when the channel closes.
        private byte[] bytes;
        /// Current offset, which may lie beyond EOF.
        private long position;

        /// Creates a channel at offset zero.
        Channel(JanexFileSystem fileSystem, byte[] bytes) {
            this.fileSystem = fileSystem;
            this.bytes = bytes;
        }

        /// Requires an open channel and view.
        private void check() throws ClosedChannelException {
            if (bytes == null || !fileSystem.isOpen()) {
                throw new ClosedChannelException();
            }
        }

        /// Reads into the destination, advancing both positions; an empty destination returns zero even at EOF.
        @Override
        public synchronized int read(ByteBuffer destination) throws IOException {
            check();
            Objects.requireNonNull(destination);
            if (!destination.hasRemaining()) {
                return 0;
            }
            if (position >= bytes.length) {
                return -1;
            }
            int count = (int) Math.min(destination.remaining(), bytes.length - position);
            destination.put(bytes, (int) position, count);
            position += count;
            return count;
        }

        /// Rejects writes without consuming the source.
        @Override
        public synchronized int write(ByteBuffer source) throws IOException {
            check();
            throw new NonWritableChannelException();
        }

        /// Returns the current offset.
        @Override
        public synchronized long position() throws IOException {
            check();
            return position;
        }

        /// Sets a nonnegative offset, including offsets beyond EOF.
        @Override
        public synchronized SeekableByteChannel position(long value) throws IOException {
            check();
            if (value < 0) {
                throw new IllegalArgumentException("Negative position");
            }
            position = value;
            return this;
        }

        /// Returns the immutable decoded length.
        @Override
        public synchronized long size() throws IOException {
            check();
            return bytes.length;
        }

        /// Rejects truncation.
        @Override
        public synchronized SeekableByteChannel truncate(long size) throws IOException {
            check();
            if (size < 0) {
                throw new IllegalArgumentException("Negative size");
            }
            throw new NonWritableChannelException();
        }

        /// Tests both channel and view lifecycle.
        @Override
        public synchronized boolean isOpen() {
            return bytes != null && fileSystem.isOpen();
        }

        /// Releases this channel's reference to decoded bytes; repeated calls are harmless.
        @Override
        public synchronized void close() {
            bytes = null;
        }
    }

    /// A directory snapshot with one lazily filtered iterator.
    private static final class Listing implements DirectoryStream<Path>, Iterator<Path> {
        /// Directory used to resolve returned names.
        private final JanexPath directory;
        /// Direct child names in stable index order.
        private final Iterator<String> names;
        /// Caller-supplied filter evaluated as iteration advances.
        private final Filter<? super Path> filter;
        /// Whether the single iterator has been requested.
        private boolean iterated;
        /// Whether this listing has closed.
        private boolean closed;
        /// Accepted next path, if already computed.
        private Path next;

        /// Creates a listing without evaluating its filter.
        Listing(JanexPath directory, List<String> names, Filter<? super Path> filter) {
            this.directory = directory;
            this.names = names.iterator();
            this.filter = filter;
        }

        /// Returns the sole iterator; closed or previously iterated listings reject this call.
        @Override
        public synchronized Iterator<Path> iterator() {
            if (closed || iterated) {
                throw new IllegalStateException("Directory stream is closed or already iterated");
            }
            iterated = true;
            return this;
        }

        /// Finds the next accepted entry; filter I/O failures are wrapped in DirectoryIteratorException.
        @Override
        public synchronized boolean hasNext() {
            if (next != null) {
                return true;
            }
            if (closed) {
                return false;
            }
            directory.fileSystem.ensureOpen();
            while (next == null && names.hasNext()) {
                Path candidate = directory.resolve(names.next());
                try {
                    if (filter.accept(candidate)) {
                        next = candidate;
                    }
                } catch (IOException failure) {
                    throw new DirectoryIteratorException(failure);
                }
            }
            return next != null;
        }

        /// Returns the next accepted entry, including an entry prefetched before closure.
        @Override
        public synchronized Path next() {
            if (!hasNext()) {
                throw new NoSuchElementException();
            }
            Path result = next;
            next = null;
            return result;
        }

        /// Rejects entry removal.
        @Override
        public void remove() {
            throw new UnsupportedOperationException("Read-only directory");
        }

        /// Stops further filtering; an already prefetched entry remains available to the iterator.
        @Override
        public synchronized void close() {
            closed = true;
        }
    }

    /// Immutable basic metadata; nanosecond timestamps retain their full format precision separately.
    private static final class Attributes implements BasicFileAttributes {
        /// Canonical path used as a stable key within this view.
        private final JanexPath path;
        /// Resource descriptor, or null for a synthetic directory.
        final ResourceIndex.Resource resource;

        /// Captures an existing resource's metadata.
        Attributes(JanexPath path, ResourceIndex.Resource resource) {
            this.path = path.toAbsolutePath().normalize();
            this.resource = resource;
        }

        /// Returns a nullable exact timestamp in nanoseconds.
        BigInteger nanos(int index) {
            return resource == null ? null : resource.times[index];
        }

        /// Converts a timestamp to FileTime, saturating values beyond its seconds range.
        private FileTime time(int index) {
            BigInteger value = nanos(index);
            if (value == null) {
                return FileTime.fromMillis(0);
            }
            BigInteger[] parts = value.divideAndRemainder(BigInteger.valueOf(1_000_000_000));
            if (parts[0].bitLength() <= 63) {
                long seconds = parts[0].longValue();
                try {
                    return FileTime.from(Instant.ofEpochSecond(seconds, parts[1].longValue()));
                } catch (java.time.DateTimeException outsideInstant) {
                    return FileTime.from(seconds, TimeUnit.SECONDS);
                }
            }
            return FileTime.from(value.signum() < 0 ? Long.MIN_VALUE : Long.MAX_VALUE, TimeUnit.SECONDS);
        }

        /// Returns the stored modification time or the epoch.
        @Override
        public FileTime lastModifiedTime() {
            return time(1);
        }

        /// Returns the stored access time or the epoch.
        @Override
        public FileTime lastAccessTime() {
            return time(2);
        }

        /// Returns the stored creation time or the epoch.
        @Override
        public FileTime creationTime() {
            return time(0);
        }

        /// Tests whether this entry has file content.
        @Override
        public boolean isRegularFile() {
            return resource != null && resource.id != -1;
        }

        /// Tests whether this entry is an explicit or synthetic directory.
        @Override
        public boolean isDirectory() {
            return !isRegularFile();
        }

        /// Returns false because links were resolved before launch.
        @Override
        public boolean isSymbolicLink() {
            return false;
        }

        /// Returns false because only files and directories are exposed.
        @Override
        public boolean isOther() {
            return false;
        }

        /// Returns the decoded file length, or zero for directories.
        @Override
        public long size() {
            return resource == null ? 0 : resource.length;
        }

        /// Returns the normalized path as a view-scoped identity.
        @Override
        public Object fileKey() {
            return path;
        }
    }

    /// The immutable logical store containing all indexed roots.
    static final class Store extends FileStore {
        /// View whose lifetime controls access to size information.
        private final JanexFileSystem fileSystem;

        /// Creates a view-scoped store descriptor.
        Store(JanexFileSystem fileSystem) {
            this.fileSystem = fileSystem;
        }

        /// Returns the logical snapshot name.
        @Override
        public String name() {
            return "janex";
        }

        /// Returns the archive store type.
        @Override
        public String type() {
            return "janex";
        }

        /// Returns true.
        @Override
        public boolean isReadOnly() {
            return true;
        }

        /// Returns the sum of decoded resource lengths without decoding payloads.
        @Override
        public long getTotalSpace() {
            fileSystem.ensureOpen();
            long size = 0;
            for (ResourceIndex.Resource resource : fileSystem.entries.values()) {
                if (resource != null) {
                    size += resource.length;
                }
            }
            return size;
        }

        /// Returns zero because the snapshot has no writable space.
        @Override
        public long getUsableSpace() {
            fileSystem.ensureOpen();
            return 0;
        }

        /// Returns zero because the snapshot has no allocation space.
        @Override
        public long getUnallocatedSpace() {
            fileSystem.ensureOpen();
            return 0;
        }

        /// Tests support for the standard basic attribute view.
        @Override
        public boolean supportsFileAttributeView(Class<? extends FileAttributeView> type) {
            return Objects.requireNonNull(type) == BasicFileAttributeView.class;
        }

        /// Tests support for basic or Janex metadata attributes.
        @Override
        public boolean supportsFileAttributeView(String name) {
            return fileSystem.supportedFileAttributeViews().contains(Objects.requireNonNull(name));
        }

        /// Returns null because no store-specific attribute view is defined.
        @Override
        public <V extends FileStoreAttributeView> V getFileStoreAttributeView(Class<V> type) {
            Objects.requireNonNull(type);
            return null;
        }

        /// Returns a standard space attribute, rejecting unknown names.
        @Override
        public Object getAttribute(String name) {
            if (name.equals("totalSpace")) {
                return getTotalSpace();
            }
            if (name.equals("usableSpace")) {
                return getUsableSpace();
            }
            if (name.equals("unallocatedSpace")) {
                return getUnallocatedSpace();
            }
            throw new UnsupportedOperationException(name);
        }
    }
}

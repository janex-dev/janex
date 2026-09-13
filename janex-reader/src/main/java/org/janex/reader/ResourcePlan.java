// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader;

import java.math.BigInteger;
import java.nio.file.Path;
import java.util.*;

/// Immutable selected resources referencing an unchanged snapshot, independent of a launch transport.
///
/// Descriptors remain usable after the reader closes. The snapshot must remain unchanged and present
/// until consumers finish reading its stored ranges. Arrays returned by accessors are independent copies.
public final class ResourcePlan {
    /// Snapshot path retained for stored byte ranges.
    private final Path snapshot;
    /// Read policy inherited from preparation.
    private final ReadLimits limits;
    /// Sources in dependency order.
    private final List<Source> sources;
    /// Selected CLASSFILE string pools.
    private final String[][] pools;
    /// Required module names and exact versions; empty versions are unconstrained.
    private final Map<String, String> requirements;
    /// Resource roots in lookup order.
    private final List<Root> roots;

    /// Retains validated preparation data without exposing mutable input arrays or collections.
    ResourcePlan(Path snapshot, ReadLimits limits, List<Source> sources, String[][] pools, Map<String, String> requirements, List<Root> roots) {
        this.snapshot = snapshot;
        this.limits = limits;
        this.sources = Collections.unmodifiableList(new ArrayList<>(sources));
        this.pools = copy(pools);
        this.requirements = Collections.unmodifiableMap(new LinkedHashMap<>(requirements));
        this.roots = Collections.unmodifiableList(new ArrayList<>(roots));
    }

    /// Returns the snapshot path retained for stored byte ranges.
    public Path snapshot() {
        return snapshot;
    }

    /// Returns the read policy inherited from preparation.
    public ReadLimits limits() {
        return limits;
    }

    /// Returns sources in dependency order.
    public List<Source> sources() {
        return sources;
    }

    /// Returns selected CLASSFILE string pools.
    public String[][] pools() {
        return copy(pools);
    }

    /// Returns required module names and exact versions; empty versions are unconstrained.
    public Map<String, String> requirements() {
        return requirements;
    }

    /// Returns resource roots in lookup order.
    public List<Root> roots() {
        return roots;
    }

    /// An inline, stored, or extent-assembled byte source.
    public static final class Source {
        /// Inline content, or null for stored and extent sources.
        private final byte[] inline;
        /// Physical offset for a stored source; unused for inline content and -1 for extents.
        private final long offset;
        /// Encoded snapshot length; zero for inline and extent sources.
        private final int storedLength;
        /// Successive Zstandard decoded byte lengths.
        private final int[] filters;
        /// Source index, decoded offset, and length triples; empty for other source kinds.
        private final int[][] extents;

        /// Retains validated preparation data without exposing mutable input arrays or collections.
        Source(byte[] inline, long offset, int storedLength, int[] filters, int[][] extents) {
            this.inline = inline == null ? null : inline.clone();
            this.offset = offset;
            this.storedLength = storedLength;
            this.filters = filters == null ? null : filters.clone();
            this.extents = copy(extents);
        }

        /// Returns inline content, or null for stored and extent sources.
        public byte[] inline() {
            return inline == null ? null : inline.clone();
        }

        /// Returns the stored source's physical offset; unused for inline content and -1 for extents.
        public long offset() {
            return offset;
        }

        /// Returns encoded snapshot length; zero for inline and extent sources.
        public int storedLength() {
            return storedLength;
        }

        /// Returns successive Zstandard decoded byte lengths.
        public int[] filters() {
            return filters == null ? null : filters.clone();
        }

        /// Returns source index, decoded offset, and length triples; empty for other source kinds.
        public int[][] extents() {
            return copy(extents);
        }
    }

    /// A classpath or module-path root with immutable file descriptors.
    public static final class Root {
        /// Original JAR filename.
        private final String name;
        /// Whether this root belongs to the module path.
        private final boolean module;
        /// Expanded resource paths in traversal order; directory names end with a slash.
        private final Map<String, File> files;

        /// Retains validated preparation data without exposing mutable input arrays or collections.
        Root(String name, boolean module, Map<String, File> files) {
            this.name = name;
            this.module = module;
            this.files = Collections.unmodifiableMap(new LinkedHashMap<>(files));
        }

        /// Returns original JAR filename.
        public String name() {
            return name;
        }

        /// Returns whether this root belongs to the module path.
        public boolean module() {
            return module;
        }

        /// Returns expanded resource paths in traversal order; directory names end with a slash.
        public Map<String, File> files() {
            return files;
        }
    }

    /// A logical resource and its decoded metadata.
    public static final class File {
        /// Source index, or -1 for a directory.
        private final int source;
        /// CLASSFILE decoded length and string-pool index pairs.
        private final int[][] transforms;
        /// Creation, modification, and access nanoseconds; absent values are null.
        private final BigInteger[] times;
        /// POSIX permission bits, or null when unspecified.
        private final Integer permissions;

        /// Retains validated preparation data without exposing mutable input arrays or collections.
        File(int source, int[][] transforms, BigInteger[] times, Integer permissions) {
            this.source = source;
            this.transforms = copy(transforms);
            this.times = times == null ? null : times.clone();
            this.permissions = permissions;
        }

        /// Returns source index, or -1 for a directory.
        public int source() {
            return source;
        }

        /// Returns cLASSFILE decoded length and string-pool index pairs.
        public int[][] transforms() {
            return copy(transforms);
        }

        /// Returns creation, modification, and access nanoseconds; absent values are null.
        public BigInteger[] times() {
            return times == null ? null : times.clone();
        }

        /// Returns pOSIX permission bits, or null when unspecified.
        public Integer permissions() {
            return permissions;
        }
    }

    /// Copies each row of a rectangular or ragged integer table.
    private static int[][] copy(int[][] values) {
        int[][] copy = values.clone();
        for (int i = 0; i < copy.length; i++) {
            copy[i] = copy[i].clone();
        }
        return copy;
    }

    /// Copies string-pool arrays while sharing immutable strings.
    private static String[][] copy(String[][] values) {
        String[][] copy = values.clone();
        for (int i = 0; i < copy.length; i++) {
            copy[i] = copy[i].clone();
        }
        return copy;
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.nio.file.Path;
import java.time.Instant;
import java.util.*;
import org.glavo.janex.reader.internal.JarSource;

/// Immutable selected resources referencing an unchanged snapshot, independent of a launch transport.
///
/// Descriptors remain usable after the reader closes. The snapshot must remain unchanged and present
/// until consumers finish reading its stored ranges. External JAR snapshots have the same lifetime
/// requirement. Arrays returned by accessors are independent copies.
public final class ResourcePlan {
    /// Snapshot path retained for stored byte ranges.
    private final Path snapshot;
    /// Read policy inherited from preparation.
    private final ReadLimits limits;
    /// Sources in dependency order.
    private final List<Source> sources;
    /// Selected CLASSFILE data pools.
    private final DataPool[] pools;
    /// Required module names and exact versions; empty versions are unconstrained.
    private final Map<String, String> requirements;
    /// Resource roots in lookup order.
    private final List<Root> roots;

    /// Retains validated preparation data without exposing mutable input arrays or collections.
    ResourcePlan(Path snapshot, ReadLimits limits, List<Source> sources, DataPool[] pools, Map<String, String> requirements, List<Root> roots) {
        this.snapshot = snapshot;
        this.limits = limits;
        this.sources = Collections.unmodifiableList(new ArrayList<>(sources));
        this.pools = pools.clone();
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

    /// Returns an independent array sharing the selected immutable CLASSFILE data pools.
    public DataPool[] pools() {
        return pools.clone();
    }

    /// Returns required module names and exact versions; empty versions are unconstrained.
    public Map<String, String> requirements() {
        return requirements;
    }

    /// Returns resource roots in lookup order.
    public List<Root> roots() {
        return roots;
    }

    /// An inline, stored, external JAR, or extent-assembled byte source.
    public static final class Source {
        /// Inline content, or null for other source kinds.
        private final byte[] inline;
        /// Deferred external JAR payload, or null for other sources.
        private final JarSource jar;
        /// Physical offset for a stored source; unused for inline content and -1 for extents.
        private final long offset;
        /// Encoded snapshot length; zero for inline and extent sources.
        private final int storedLength;
        /// Successive Zstandard decoded byte lengths.
        private final int[] filters;
        /// Source index, decoded offset, and length triples; empty for other source kinds.
        private final int[][] extents;

        /// Retains validated source data and an optional immutable external JAR descriptor.
        Source(byte[] inline, long offset, int storedLength, int[] filters, int[][] extents, JarSource jar) {
            this.inline = inline == null ? null : inline.clone();
            this.jar = jar;
            this.offset = offset;
            this.storedLength = storedLength;
            this.filters = filters == null ? null : filters.clone();
            this.extents = copy(extents);
        }

        /// Returns inline content, or null for other source kinds.
        public byte[] inline() {
            return inline == null ? null : inline.clone();
        }

        /// Returns a deferred external JAR payload, or null for other sources.
        public JarSource jar() {
            return jar;
        }

        /// Returns the container source's physical offset, or -1 for other source kinds.
        public long offset() {
            return offset;
        }

        /// Returns encoded container snapshot length, or zero for other source kinds.
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

    /// An immutable CLASSFILE decoding step using a selected data pool.
    public static final class ClassFileTransform {
        /// Number of bytes produced by decoding this step.
        private final int decodedLength;
        /// Index of the data pool used by this step.
        private final int dataPoolIndex;

        /// Creates a decoding step from resolved resource metadata.
        ///
        /// @param decodedLength nonnegative decoded byte length
        /// @param dataPoolIndex nonnegative index in the associated data-pool table
        public ClassFileTransform(int decodedLength, int dataPoolIndex) {
            this.decodedLength = decodedLength;
            this.dataPoolIndex = dataPoolIndex;
        }

        /// Returns the decoded byte length.
        public int decodedLength() {
            return decodedLength;
        }

        /// Returns the index in the associated data-pool table.
        public int dataPoolIndex() {
            return dataPoolIndex;
        }
    }

    /// A logical resource and its decoded metadata.
    public static final class File {
        /// Source index, or -1 for a directory.
        private final int source;
        /// Immutable CLASSFILE steps in decoding order.
        private final List<ClassFileTransform> transforms;
        /// Creation, modification, and access instants; absent values are null.
        private final Instant[] times;
        /// POSIX permission bits, or null when unspecified.
        private final Integer permissions;

        /// Retains validated preparation data without exposing mutable input arrays or collections.
        File(int source, ClassFileTransform[] transforms, Instant[] times, Integer permissions) {
            this.source = source;
            this.transforms = transforms.length == 0 ? Collections.emptyList()
                    : Collections.unmodifiableList(Arrays.asList(transforms.clone()));
            this.times = times == null ? null : times.clone();
            this.permissions = permissions;
        }

        /// Returns source index, or -1 for a directory.
        public int source() {
            return source;
        }

        /// Returns an immutable list of CLASSFILE steps in decoding order.
        public List<ClassFileTransform> transforms() {
            return transforms;
        }

        /// Returns an independent copy of creation, modification, and access instants, or null if no array was supplied.
        /// Null array elements denote absent timestamps.
        public Instant[] times() {
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

}

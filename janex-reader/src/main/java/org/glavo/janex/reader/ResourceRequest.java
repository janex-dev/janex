// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.nio.file.Path;
import java.util.Objects;

/// A selected container root or a caller-verified external JAR.
/// The caller must retain referenced files unchanged until resource consumers finish reading them.
public final class ResourceRequest {
    /// Whether the root belongs to the module path.
    public final boolean module;
    /// Container pool ID for a local root; unused for external JARs.
    public final long pool;
    /// Blob index for a local root; unused for external JARs.
    public final long index;
    /// External snapshot path, or null for a local root.
    public final Path path;
    /// Original external JAR filename, or null for a local root.
    public final String jarName;

    /// Selects a local root using unsigned 64-bit pool and blob identifiers.
    public ResourceRequest(boolean module, long pool, long index) {
        this.module = module;
        this.pool = pool;
        this.index = index;
        this.path = null;
        this.jarName = null;
    }

    /// Selects a verified external snapshot with its original JAR filename.
    /// Neither argument may be null. The file is read during resource preparation.
    public ResourceRequest(boolean module, Path path, String jarName) {
        this.module = module;
        this.pool = 0;
        this.index = 0;
        this.path = Objects.requireNonNull(path);
        this.jarName = Objects.requireNonNull(jarName);
    }
}

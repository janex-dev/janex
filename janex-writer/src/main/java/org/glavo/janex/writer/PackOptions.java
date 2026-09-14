// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;

import org.glavo.janex.reader.Checksum;
import org.glavo.janex.reader.ReadLimits;

/// Configures one application package. Fields and lists may be changed before writing.
/// Neither these options nor input files may be changed while a write is in progress.
public final class PackOptions {
    /// Primary directory or JAR, placed first on the classpath or selected module path.
    public final Path source;
    /// Destination; writing requires that it does not exist, including as a symbolic link.
    public final Path output;
    /// Additional embedded classpath inputs in lookup order.
    public final List<Path> classPath = new ArrayList<>();
    /// Additional embedded module-path inputs in lookup order.
    public final List<Path> modulePath = new ArrayList<>();
    /// External declarations appended after embedded classpath inputs, without downloading.
    public final List<ExternalDependency> externalClassPath = new ArrayList<>();
    /// External declarations appended after embedded module-path inputs, without downloading.
    public final List<ExternalDependency> externalModulePath = new ArrayList<>();
    /// Binary main-class name, or null to infer it from the primary manifest or module descriptor.
    public String mainClass;
    /// Main module name, or null for classpath launching.
    public String mainModule;
    /// Nonempty file-local application identifier.
    public String applicationId = "main";
    /// Complete JVM arguments in order, without shell splitting.
    public final List<String> jvmOptions = new ArrayList<>();
    /// Preset program arguments; empty strings and Unicode are preserved.
    public final List<String> arguments = new ArrayList<>();
    /// Optional Java version requirement in `vers:jep322` syntax.
    public String javaVersion;
    /// Whether to use Zstandard when stored bytes plus encoding overhead shrink; defaults to true.
    /// Applies to blobs and table pages. False stores both without compression.
    public boolean compression = true;
    /// Whether to append the bundled portable launcher for `java -jar` execution.
    public boolean withLauncher;
    /// Optional unsigned PE or ELF native launcher to prepend.
    public Path nativeLauncher;
    /// Native invocation strategy, either `bootstrap` or `direct`.
    public String nativeLaunchMode = "bootstrap";
    /// Per-value, collection, and resource-path nesting limits.
    public ReadLimits limits = ReadLimits.DEFAULT;
    /// Maximum aggregate original resource bytes across all inputs; must be nonnegative.
    public long maxTotalBytes = 512L * 1024 * 1024;

    /// Creates options with no additional inputs, wrappers, or external declarations.
    /// @param source nonnull primary directory or JAR
    /// @param output nonnull destination
    public PackOptions(Path source, Path output) {
        this.source = Objects.requireNonNull(source);
        this.output = Objects.requireNonNull(output);
    }

    /// Declares an external Java path entry whose syntax is checked during writing.
    /// @param uri nonnull external URI, or canonical PURL
    /// @param checksum optional checksum of the complete original artifact
    public record ExternalDependency(String uri, Checksum checksum) {
        /// Creates a declaration; the URI must be nonnull.
        public ExternalDependency {
            Objects.requireNonNull(uri);
        }
    }
}

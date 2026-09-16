// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.gradle;

import java.io.File;
import java.io.IOException;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.LinkOption;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.time.Clock;
import java.time.Instant;
import java.time.ZoneOffset;

import org.gradle.api.Action;
import org.gradle.api.tasks.Nested;

import org.glavo.janex.writer.JanexWriter;
import org.glavo.janex.writer.PackOptions;
import org.gradle.api.DefaultTask;
import org.gradle.api.GradleException;
import org.gradle.api.file.ConfigurableFileCollection;
import org.gradle.api.file.FileCollection;
import org.gradle.api.file.FileSystemLocation;
import org.gradle.api.file.RegularFileProperty;
import org.gradle.api.provider.ListProperty;
import org.gradle.api.provider.MapProperty;
import org.gradle.api.provider.Property;
import org.gradle.api.tasks.Input;
import org.gradle.api.tasks.InputFile;
import org.gradle.api.tasks.InputFiles;
import org.gradle.api.tasks.Internal;
import org.gradle.api.tasks.Optional;
import org.gradle.api.tasks.OutputFile;
import org.gradle.api.tasks.PathSensitive;
import org.gradle.api.tasks.PathSensitivity;
import org.gradle.api.tasks.TaskAction;
import org.gradle.api.tasks.CacheableTask;

/// Packages merged primary directories or an explicit JAR and ordered dependency paths.
/// A temporary sibling replaces the previous output only after successful packaging.
/// Gradle tracks input contents, path order, and launch settings for incremental and cached builds.
@CacheableTask
public abstract class JanexPack extends DefaultTask {
    /// Managed settings containing key locations but no secret key material.
    private final JanexSigning signing = getProject().getObjects().newInstance(JanexSigning.class);
    /// Optional minimization settings tracked as nested task inputs.
    private final JanexMinimization minimization = getProject().getObjects().newInstance(JanexMinimization.class);
    /// Resource filters tracked as nested task inputs.
    private final JanexResources resources = getProject().getObjects().newInstance(JanexResources.class);
    /// Effective directory inputs whose producer tasks are omitted when a primary JAR is selected.
    private final ConfigurableFileCollection inputDirectoryFiles = getProject().getObjects().fileCollection();

    /// Creates a packaging task. [JanexPlugin] supplies conventions for `janexPack`.
    public JanexPack() {
        getSourceName().convention("resources.jar");
        getManifestAttributes().convention(Map.of("Manifest-Version", "1.0"));
        inputDirectoryFiles.from(getSource().map(file -> List.<File>of()).orElse(
                getInputDirectories().getElements().map(elements -> elements.stream()
                        .map(FileSystemLocation::getAsFile).toList())));
        getApplicationId().convention("main");
        getJvmOptions().convention(List.of());
        getArguments().convention(List.of());
        getWithLauncher().convention(true);
        getCompression().convention(true);
        getCompressionLevel().convention(3);
        getTransformClassfiles().convention(true);
        getNativeLaunchMode().convention("bootstrap");
        getOutputs().upToDateWhen(task -> !((JanexPack) task).hasDirectoryInputs() && !((JanexPack) task).signing.isEnabled());
        getOutputs().doNotCacheIf("Signing keys and execution-time policy are not cached",
                task -> ((JanexPack) task).signing.isEnabled());
        getOutputs().doNotCacheIf("Directory permissions and native executable modes require filesystem metadata",
                task -> ((JanexPack) task).hasDirectoryInputs() || ((JanexPack) task).getNativeLauncher().isPresent());
    }

    /// Returns optional publisher-signing settings, resolved only during execution.
    /// @return the mutable managed signing settings
    @Nested
    public JanexSigning getSigning() { return signing; }

    /// Configures publisher-signing settings for this task.
    /// @param action configuration action applied immediately
    public void signing(Action<? super JanexSigning> action) { action.execute(signing); }

    /// Returns dependency minimization settings; disabled by default.
    /// @return mutable managed minimization settings
    @Nested
    public JanexMinimization getMinimization() { return minimization; }

    /// Enables whole-class dependency minimization with the current keep rules.
    public void minimize() { minimization.getEnabled().set(true); }

    /// Enables minimization and then configures keep rules or overrides enablement.
    /// @param action configuration applied immediately
    public void minimize(Action<? super JanexMinimization> action) {
        minimize();
        action.execute(minimization);
    }

    /// Returns resource exclusions, applied independently of minimization.
    /// @return mutable managed resource-filter settings
    @Nested
    public JanexResources getResources() { return resources; }

    /// Configures resource exclusions.
    /// @param action configuration applied immediately
    public void resources(Action<? super JanexResources> action) { action.execute(resources); }

    /// Returns an optional primary JAR overriding the merged directory inputs.
    ///
    /// @return the optional primary JAR file property
    @InputFile
    @Optional
    @PathSensitive(PathSensitivity.NAME_ONLY)
    public abstract RegularFileProperty getSource();

    /// Returns directories configured for the merged primary root; ignored for an explicit JAR.
    /// Missing directories are skipped. Duplicate files or links fail. Producer tasks are inferred.
    /// File and directory permissions are normalized to 0644 and 0755 respectively.
    /// @return the ordered configurable directory collection
    @Internal
    public abstract ConfigurableFileCollection getInputDirectories();

    /// Returns effective directory inputs, empty when [#getSource()] is present.
    /// @return the tracked directory collection
    @InputFiles
    @PathSensitive(PathSensitivity.RELATIVE)
    public FileCollection getInputDirectoryFiles() { return inputDirectoryFiles; }

    /// Returns effective directory order so changes invalidate the output.
    /// @return absolute directory locations in merge order
    @Input
    public List<String> getInputDirectoryOrder() {
        return inputDirectoryFiles.getFiles().stream().map(File::getAbsolutePath).toList();
    }

    /// Returns symbolic-link identities and targets, which file-content snapshots alone do not preserve.
    /// Paths are relative to each input directory and qualified by its ordinal; traversal does not follow links.
    /// @return sorted link descriptions in directory order
    /// @throws IOException if traversing an existing input or reading a link fails
    @Input
    public List<String> getInputDirectoryLinks() throws IOException {
        List<String> links = new ArrayList<>();
        int index = 0;
        for (File directory : inputDirectoryFiles) {
            Path root = directory.toPath();
            if (Files.exists(root, LinkOption.NOFOLLOW_LINKS)) {
                final String prefix = index + ":";
                try (var paths = Files.walk(root)) {
                    for (Path path : paths.filter(Files::isSymbolicLink).sorted().toList()) {
                        links.add(prefix + root.relativize(path) + "\0" + Files.readSymbolicLink(path));
                    }
                }
            }
            index++;
        }
        return links;
    }

    /// Returns the merged root's metadata filename; explicit JAR inputs retain their own name.
    /// @return a filename ending in `.jar`
    @Input
    public abstract Property<String> getSourceName();

    /// Returns main manifest attribute overrides, independent of the Java `jar` task.
    /// Defaults to `Manifest-Version: 1.0`; values cannot contain CR, LF, or NUL.
    /// @return the tracked main-attribute map
    @Input
    public abstract MapProperty<String, String> getManifestAttributes();

    /// Returns additional classpath JARs or directories in lookup order.
    ///
    /// @return the ordered classpath collection
    @InputFiles
    @PathSensitive(PathSensitivity.RELATIVE)
    public abstract ConfigurableFileCollection getClassPath();

    /// Returns additional module-path JARs or directories in lookup order.
    ///
    /// @return the ordered module-path collection
    @InputFiles
    @PathSensitive(PathSensitivity.RELATIVE)
    public abstract ConfigurableFileCollection getModulePath();

    /// Returns classpath locations in lookup order so reordering invalidates the output.
    ///
    /// @return a snapshot of absolute classpath locations in lookup order
    @Input
    public List<String> getClassPathOrder() {
        return getClassPath().getFiles().stream().map(File::getAbsolutePath).toList();
    }

    /// Returns module-path locations in lookup order, including automatic-module filenames.
    ///
    /// @return a snapshot of absolute module-path locations in lookup order
    @Input
    public List<String> getModulePathOrder() {
        return getModulePath().getFiles().stream().map(File::getAbsolutePath).toList();
    }

    /// Returns the optional binary main-class name; absence enables entry-point inference.
    ///
    /// @return the optional main-class property
    @Input
    @Optional
    public abstract Property<String> getMainClass();

    /// Returns the optional main module; when set, the primary resource root enters the module path.
    ///
    /// @return the optional main-module property
    @Input
    @Optional
    public abstract Property<String> getMainModule();

    /// Returns the application ID, defaulting to `main`.
    ///
    /// @return the application ID property
    @Input
    public abstract Property<String> getApplicationId();

    /// Returns ordered JVM arguments without shell splitting, defaulting to an empty list.
    ///
    /// @return the ordered JVM-argument property
    @Input
    public abstract ListProperty<String> getJvmOptions();

    /// Returns preset application arguments, defaulting to an empty list; empty strings are retained.
    ///
    /// @return the ordered preset-argument property
    @Input
    public abstract ListProperty<String> getArguments();

    /// Returns the optional Java runtime requirement in `vers:jep322` syntax.
    /// This setting does not configure the Java toolchain or compilation target.
    ///
    /// @return the optional Java version requirement property
    @Input
    @Optional
    public abstract Property<String> getJavaVersion();

    /// Sets the Java runtime requirement to `vers:jep322/>=minimumVersion`.
    /// This setting does not configure the Java toolchain or compilation target.
    ///
    /// @param minimumVersion minimum Java feature version, at least 8
    /// @throws IllegalArgumentException if `minimumVersion` is below 8
    public void javaVersion(int minimumVersion) {
        if (minimumVersion < 8) {
            throw new IllegalArgumentException("javaVersion must be at least 8");
        }
        getJavaVersion().set("vers:jep322/>=" + minimumVersion);
    }

    /// Returns whether to compress blobs and table pages when their encoded representation shrinks.
    ///
    /// @return the compression property, defaulting to `true`
    @Input
    public abstract Property<Boolean> getCompression();

    /// Returns the Zstandard level for blobs and table pages, defaulting to 3.
    /// Zero selects the native default; negative levels favor speed. The writer rejects values
    /// outside the native library's supported range when compression is enabled.
    /// Ignored when compression is disabled.
    /// @return the compression-level property
    @Input
    public abstract Property<Integer> getCompressionLevel();

    /// Returns whether to try shared CLASSFILE strings, defaulting to true.
    /// The writer retains the smaller complete pool representation.
    /// @return the CLASSFILE transform property
    @Input
    public abstract Property<Boolean> getTransformClassfiles();

    /// Returns whether to append a `java -jar` launcher, defaulting to `true`.
    ///
    /// @return the JAR-launcher inclusion property
    @Input
    public abstract Property<Boolean> getWithLauncher();

    /// Returns the optional PE or ELF native launcher prefix.
    ///
    /// @return the optional native launcher file property
    @InputFile
    @Optional
    @PathSensitive(PathSensitivity.NONE)
    public abstract RegularFileProperty getNativeLauncher();

    /// Returns the native launch mode, `bootstrap` or `direct`, defaulting to `bootstrap`.
    /// It is used only when a native launcher is present.
    ///
    /// @return the native launch-mode property
    @Input
    public abstract Property<String> getNativeLaunchMode();

    /// Returns the destination, replaced only after the writer successfully creates the package.
    ///
    /// @return the destination file property
    @OutputFile
    public abstract RegularFileProperty getOutputFile();

    /// Detects directory inputs whose permission changes are not tracked by Gradle file snapshots.
    private boolean hasDirectoryInputs() {
        return getClassPath().getFiles().stream().anyMatch(File::isDirectory)
                || getModulePath().getFiles().stream().anyMatch(File::isDirectory);
    }

    /// Writes the package and installs its output, preserving a previous package on writer failure.
    /// Temporary files are removed after execution. The destination must not name an input.
    ///
    /// @throws IOException if creating, moving, or removing output files fails
    /// @throws GradleException if the destination aliases or is inside an input
    @TaskAction
    public void pack() throws IOException {
        Path output = getOutputFile().get().getAsFile().toPath().toAbsolutePath().normalize();
        List<File> inputs = new ArrayList<>();
        if (getSource().isPresent()) inputs.add(getSource().get().getAsFile());
        inputs.addAll(inputDirectoryFiles.getFiles());
        inputs.addAll(getClassPath().getFiles());
        inputs.addAll(getModulePath().getFiles());
        if (getNativeLauncher().isPresent()) {
            inputs.add(getNativeLauncher().get().getAsFile());
        }
        for (var property : List.of(signing.getCmsCertificate(), signing.getCmsKey(), signing.getOpenPgpKey())) {
            if (property.isPresent()) inputs.add(property.get().getAsFile());
        }
        for (File input : inputs) {
            Path path = input.toPath().toAbsolutePath().normalize();
            if (output.equals(path) || (Files.exists(output) && Files.exists(path) && Files.isSameFile(output, path))
                    || (Files.isDirectory(path) && output.startsWith(path))) {
                throw new GradleException("Janex output must not replace or be inside an input: " + output);
            }
        }
        Files.createDirectories(output.getParent());
        Path temporaryDirectory = Files.createTempDirectory(output.getParent(), ".janex-");
        Path temporaryOutput = temporaryDirectory.resolve(output.getFileName());
        try {
            PackOptions options;
            if (getSource().isPresent()) {
                options = new PackOptions(getSource().get().getAsFile().toPath(), temporaryOutput);
            } else {
                options = new PackOptions(inputDirectoryFiles.getFiles().stream().map(File::toPath)
                        .filter(path -> Files.exists(path, LinkOption.NOFOLLOW_LINKS)).toList(), temporaryOutput);
                options.sourceName = getSourceName().get();
                options.normalizeSourcePermissions = true;
            }
            options.manifestAttributes.putAll(getManifestAttributes().get());
            options.applicationId = getApplicationId().get();
            options.mainClass = getMainClass().getOrNull();
            options.mainModule = getMainModule().getOrNull();
            options.javaVersion = getJavaVersion().getOrNull();
            for (File file : getClassPath()) options.classPath.add(file.toPath());
            for (File file : getModulePath()) options.modulePath.add(file.toPath());
            options.jvmOptions.addAll(getJvmOptions().get());
            options.arguments.addAll(getArguments().get());
            options.withLauncher = getWithLauncher().get();
            options.compression = getCompression().get();
            options.compressionLevel = getCompressionLevel().get();
            options.transformClassfiles = getTransformClassfiles().get();
            minimization.configure(options);
            resources.configure(options);
            Instant time = signing.getTime().isPresent() ? Instant.parse(signing.getTime().get()) : Instant.now();
            options.signingClock = Clock.fixed(time, ZoneOffset.UTC);
            options.signer = signing.load(time);
            if (getNativeLauncher().isPresent()) {
                options.nativeLauncher = getNativeLauncher().get().getAsFile().toPath();
                options.nativeLaunchMode = getNativeLaunchMode().get();
            }
            JanexWriter.write(options);
            try {
                Files.move(temporaryOutput, output, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
            } catch (AtomicMoveNotSupportedException ignored) {
                Files.move(temporaryOutput, output, StandardCopyOption.REPLACE_EXISTING);
            }
        } finally {
            Files.deleteIfExists(temporaryOutput);
            Files.deleteIfExists(temporaryDirectory);
        }
    }
}

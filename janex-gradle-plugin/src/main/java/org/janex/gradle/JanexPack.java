// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.gradle;

import java.io.File;
import java.io.IOException;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;

import javax.inject.Inject;

import org.gradle.api.DefaultTask;
import org.gradle.api.GradleException;
import org.gradle.api.file.ConfigurableFileCollection;
import org.gradle.api.file.RegularFileProperty;
import org.gradle.api.provider.ListProperty;
import org.gradle.api.provider.Property;
import org.gradle.api.tasks.Input;
import org.gradle.api.tasks.InputFile;
import org.gradle.api.tasks.InputFiles;
import org.gradle.api.tasks.Optional;
import org.gradle.api.tasks.OutputFile;
import org.gradle.api.tasks.PathSensitive;
import org.gradle.api.tasks.PathSensitivity;
import org.gradle.api.tasks.TaskAction;
import org.gradle.process.ExecOperations;
import org.gradle.work.DisableCachingByDefault;

/// Packages a primary JAR and ordered dependency paths by invoking the native CLI without a shell.
/// The CLI writes a temporary sibling of the output, which replaces the output only after success.
/// Execution failures propagate as Gradle failures. Gradle tracks inputs for up-to-date checks;
/// shared build caching is disabled because packaging uses a platform-specific external tool.
@DisableCachingByDefault(because = "Packaging uses a platform-specific native executable")
public abstract class JanexPack extends DefaultTask {
    /// Creates a packaging task. [JanexPlugin] supplies conventions for `janexPack`.
    public JanexPack() {
        getApplicationId().convention("main");
        getJvmOptions().convention(List.of());
        getArguments().convention(List.of());
        getWithLauncher().convention(true);
        getNativeLaunchMode().convention("bootstrap");
    }

    /// Returns the required native CLI executable; its bytes are tracked as an input.
    ///
    /// @return the native CLI file property
    @InputFile
    @PathSensitive(PathSensitivity.NONE)
    public abstract RegularFileProperty getExecutable();

    /// Returns the required primary JAR, preserving its filename during packaging.
    ///
    /// @return the primary JAR file property
    @InputFile
    @PathSensitive(PathSensitivity.NAME_ONLY)
    public abstract RegularFileProperty getSource();

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

    /// Returns the optional binary main-class name; absence enables CLI entry-point inference.
    ///
    /// @return the optional main-class property
    @Input
    @Optional
    public abstract Property<String> getMainClass();

    /// Returns the optional main module; when set, the primary JAR enters the module path.
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

    /// Returns the optional Java version requirement in `vers:jep322` syntax.
    ///
    /// @return the optional Java version requirement property
    @Input
    @Optional
    public abstract Property<String> getJavaVersion();

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
    /// It is passed to the CLI only when a native launcher is present.
    ///
    /// @return the native launch-mode property
    @Input
    public abstract Property<String> getNativeLaunchMode();

    /// Returns the destination, replaced only after the CLI successfully creates the package.
    ///
    /// @return the destination file property
    @OutputFile
    public abstract RegularFileProperty getOutputFile();

    /// Returns Gradle's process execution service.
    ///
    /// @return the injected process execution service
    @Inject
    protected abstract ExecOperations getExecOperations();

    /// Runs the CLI and installs its output, preserving a previous package if the CLI fails.
    /// Temporary files are removed after execution. The destination must not name an input.
    ///
    /// @throws IOException if creating, moving, or removing output files fails
    /// @throws GradleException if the CLI fails or the destination aliases an input
    @TaskAction
    public void pack() throws IOException {
        Path output = getOutputFile().get().getAsFile().toPath().toAbsolutePath().normalize();
        List<File> inputs = new ArrayList<>();
        inputs.add(getExecutable().get().getAsFile());
        inputs.add(getSource().get().getAsFile());
        inputs.addAll(getClassPath().getFiles());
        inputs.addAll(getModulePath().getFiles());
        if (getNativeLauncher().isPresent()) {
            inputs.add(getNativeLauncher().get().getAsFile());
        }
        for (File input : inputs) {
            Path path = input.toPath().toAbsolutePath().normalize();
            if (output.equals(path) || (Files.exists(output) && Files.isSameFile(output, path))
                    || (Files.isDirectory(path) && output.startsWith(path))) {
                throw new GradleException("Janex output must not replace or be inside an input: " + output);
            }
        }
        Files.createDirectories(output.getParent());
        Path temporaryDirectory = Files.createTempDirectory(output.getParent(), ".janex-");
        Path temporaryOutput = temporaryDirectory.resolve(output.getFileName());
        try {
            List<String> command = new ArrayList<>();
            command.add(getExecutable().get().getAsFile().getAbsolutePath());
            command.add("pack");
            command.add(getSource().get().getAsFile().getAbsolutePath());
            add(command, "--output", temporaryOutput.toString());
            add(command, "--application", getApplicationId().get());
            addOptional(command, "--main-class", getMainClass());
            addOptional(command, "--main-module", getMainModule());
            addOptional(command, "--java-version", getJavaVersion());
            for (File file : getClassPath()) {
                add(command, "--class-path", file.getAbsolutePath());
            }
            for (File file : getModulePath()) {
                add(command, "--module-path", file.getAbsolutePath());
            }
            for (String option : getJvmOptions().get()) {
                command.add("--jvm-option=" + option);
            }
            for (String argument : getArguments().get()) {
                command.add("--argument=" + argument);
            }
            if (getWithLauncher().get()) {
                command.add("--with-launcher");
            }
            if (getNativeLauncher().isPresent()) {
                add(command, "--native-launcher", getNativeLauncher().get().getAsFile().getAbsolutePath());
                add(command, "--native-launch-mode", getNativeLaunchMode().get());
            }
            getExecOperations().exec(spec -> spec.commandLine(command)).assertNormalExitValue();
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

    /// Appends one option and its value as separate process arguments.
    private static void add(List<String> command, String option, String value) {
        command.add(option);
        command.add(value);
    }

    /// Appends an option only when its property has a value.
    private static void addOptional(List<String> command, String option, Property<String> value) {
        if (value.isPresent()) {
            add(command, option, value.get());
        }
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.gradle;

import org.gradle.api.file.ConfigurableFileCollection;
import org.gradle.api.file.RegularFileProperty;
import org.gradle.api.provider.ListProperty;
import org.gradle.api.provider.Property;

/// Configures the application's Janex package using lazy Gradle properties.
///
/// Runtime dependencies are embedded in resolution order. With a main module they default to
/// the module path; otherwise they default to the classpath. Each path can be replaced with
/// [ConfigurableFileCollection#setFrom(Object...)].
public abstract class JanexExtension {
    /// Creates the extension; conventions are supplied by [JanexPlugin].
    public JanexExtension() {
    }

    /// Returns the required native CLI executable, defaulting to the `janexExecutable` Gradle
    /// property or the `JANEX_EXECUTABLE` environment variable. No tool is downloaded.
    ///
    /// @return the native CLI file property
    public abstract RegularFileProperty getExecutable();

    /// Returns the primary JAR, defaulting to the Java plugin's `jar` output.
    ///
    /// @return the primary JAR file property
    public abstract RegularFileProperty getSource();

    /// Returns additional embedded classpath entries in lookup order.
    ///
    /// @return the ordered classpath collection
    public abstract ConfigurableFileCollection getClassPath();

    /// Returns additional embedded module-path entries in lookup order.
    ///
    /// @return the ordered module-path collection
    public abstract ConfigurableFileCollection getModulePath();

    /// Returns the optional binary main-class name, defaulting to `application.mainClass`.
    /// When absent, the CLI attempts to infer the entry point from the primary JAR.
    ///
    /// @return the optional main-class property
    public abstract Property<String> getMainClass();

    /// Returns the optional main module, defaulting to `application.mainModule`.
    /// When present, the primary JAR is placed on the module path.
    ///
    /// @return the optional main-module property
    public abstract Property<String> getMainModule();

    /// Returns the application ID within the package, defaulting to `main`.
    ///
    /// @return the application ID property
    public abstract Property<String> getApplicationId();

    /// Returns complete JVM arguments without shell splitting, defaulting to
    /// `application.applicationDefaultJvmArgs` when the application plugin is applied.
    ///
    /// @return the ordered JVM-argument property
    public abstract ListProperty<String> getJvmOptions();

    /// Returns preset application arguments in order, defaulting to an empty list.
    /// Empty strings and Unicode characters are passed to the CLI unchanged.
    ///
    /// @return the ordered preset-argument property
    public abstract ListProperty<String> getArguments();

    /// Returns the optional Java version requirement in `vers:jep322` syntax.
    ///
    /// @return the optional Java version requirement property
    public abstract Property<String> getJavaVersion();

    /// Returns whether to append the `java -jar` launcher, defaulting to `true`.
    ///
    /// @return the JAR-launcher inclusion property
    public abstract Property<Boolean> getWithLauncher();

    /// Returns the optional native launcher prefix. The CLI validates its executable format.
    ///
    /// @return the optional native launcher file property
    public abstract RegularFileProperty getNativeLauncher();

    /// Returns the native launch mode, `bootstrap` or `direct`, defaulting to `bootstrap`.
    /// This value is used only when a native launcher is supplied.
    ///
    /// @return the native launch-mode property
    public abstract Property<String> getNativeLaunchMode();

    /// Returns the destination, defaulting to `build/distributions/<project-name>.janex`.
    /// A successful task replaces the previous output; a failed pack preserves it.
    /// Set an `.exe` filename when distributing a Windows native launcher.
    ///
    /// @return the destination file property
    public abstract RegularFileProperty getOutputFile();
}

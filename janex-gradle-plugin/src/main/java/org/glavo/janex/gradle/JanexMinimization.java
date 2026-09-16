// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.gradle;

import java.util.List;

import org.gradle.api.provider.ListProperty;
import org.gradle.api.provider.Property;
import org.gradle.api.tasks.Input;
import org.glavo.janex.writer.PackOptions;

/// Configures optional whole-class dependency minimization, including explicit reflection and JNI roots.
/// Primary-input classes, module-path classes, package annotations, and service providers are retained automatically.
/// Ordinary resources remain unless explicitly excluded. Bytecode and member names are not rewritten.
public abstract class JanexMinimization {
    /// Creates disabled minimization with no additional keep rules.
    public JanexMinimization() {
        getEnabled().convention(false);
        getKeepClasses().convention(List.of());
        getKeepJars().convention(List.of());
    }

    /// Returns whether unreachable dependency classes are removed; defaults to false.
    /// @return the enablement property
    @Input
    public abstract Property<Boolean> getEnabled();

    /// Returns binary class-name patterns whose matches and referenced classes must remain.
    /// Use `$` for nested classes, `*` within a package component, `?` for one character,
    /// and `**` across components. Matching is case-sensitive; unmatched patterns are permitted.
    /// These rules do not override resource exclusions and have no effect when minimization is disabled.
    /// @return the additional reachability-root patterns
    @Input
    public abstract ListProperty<String> getKeepClasses();

    /// Returns original JAR filename globs whose classes must remain; directories use resources.jar.
    /// These rules do not override resource exclusions.
    /// @return the whole-JAR keep patterns
    @Input
    public abstract ListProperty<String> getKeepJars();

    /// Appends class-name patterns without changing enablement.
    /// @param patterns binary class-name globs
    public void keep(String... patterns) { getKeepClasses().addAll(patterns); }

    /// Appends original JAR filename patterns without changing enablement.
    /// @param patterns JAR filename globs
    public void keepJar(String... patterns) { getKeepJars().addAll(patterns); }

    /// Connects task defaults to the extension while retaining explicit task settings.
    void convention(JanexMinimization extension) {
        getEnabled().convention(extension.getEnabled());
        getKeepClasses().convention(extension.getKeepClasses());
        getKeepJars().convention(extension.getKeepJars());
    }

    /// Resolves settings into one writer invocation during task execution.
    void configure(PackOptions options) {
        options.minimize = getEnabled().get();
        options.keepClasses.addAll(getKeepClasses().get());
        options.keepJars.addAll(getKeepJars().get());
    }
}

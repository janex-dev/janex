// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.gradle;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

import org.gradle.api.provider.ListProperty;
import org.gradle.api.provider.MapProperty;
import org.gradle.api.tasks.Input;
import org.glavo.janex.writer.PackOptions;

/// Configures explicit exclusions from imported resource roots, before dependency minimization.
/// Patterns match logical paths in every Multi-Release layer. They use `/` separators, `*` within
/// a component, `?` for one character, and `**` across components. `**/` also matches zero directories;
/// a trailing `/` excludes the entire subtree. Matching is case-sensitive on every platform.
public abstract class JanexResources {
    /// Creates filtering settings that retain all resources.
    public JanexResources() {
        getExcludes().convention(List.of());
        getJarExcludes().convention(Map.of());
    }

    /// Returns resource-path patterns excluded from every input.
    /// @return global exclusion patterns
    @Input
    public abstract ListProperty<String> getExcludes();

    /// Returns additional exclusions keyed by original JAR filename glob; directories use resources.jar.
    /// Every matching filename rule applies. Each value follows the resource-path pattern syntax.
    /// @return filename-scoped resource exclusions
    @Input
    public abstract MapProperty<String, List<String>> getJarExcludes();

    /// Appends resource-path patterns excluded from every input.
    /// @param patterns resource-path globs
    public void exclude(String... patterns) { getExcludes().addAll(patterns); }

    /// Appends exclusions for one JAR filename pattern, resolving its existing rule list when called.
    /// @param jar original JAR filename glob
    /// @param patterns resource-path globs
    public void excludeFrom(String jar, String... patterns) {
        List<String> values = new ArrayList<>(getJarExcludes().get().getOrDefault(jar, List.of()));
        values.addAll(List.of(patterns));
        getJarExcludes().put(jar, List.copyOf(values));
    }

    /// Connects task defaults to extension properties without resolving resources.
    void convention(JanexResources extension) {
        getExcludes().convention(extension.getExcludes());
        getJarExcludes().convention(extension.getJarExcludes());
    }

    /// Resolves selection rules into one writer invocation during task execution.
    void configure(PackOptions options) {
        options.excludes.addAll(getExcludes().get());
        options.jarExcludes.putAll(getJarExcludes().get());
    }
}

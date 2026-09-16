// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

import org.glavo.janex.reader.internal.Input;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.commons.ClassRemapper;
import org.objectweb.asm.commons.Remapper;

import static org.glavo.janex.reader.internal.Input.require;

/// Filters imported resources and computes conservative, whole-class dependency reachability.
final class ResourceSelection {
    /// Prevents construction.
    private ResourceSelection() { }

    /// Applies filters before minimization, retaining each resource root and its layer order.
    /// Conflicts between exclusions and reachable embedded classes fail before output creation.
    static void apply(List<Resources> roots, PackOptions options, String mainClass) throws IOException {
        List<Pattern> global = patterns(options.excludes, '/', true);
        Map<Pattern, List<Pattern>> scoped = new HashMap<>();
        for (var rule : options.jarExcludes.entrySet()) {
            scoped.put(pattern(rule.getKey(), '/', false), patterns(rule.getValue(), '/', true));
        }
        List<Pattern> keep = patterns(options.keepClasses, '.', false);
        List<Pattern> keepJars = patterns(options.keepJars, '/', false);
        Set<String> originalClasses = new HashSet<>();
        for (Resources root : roots) {
            List<Pattern> excludes = new ArrayList<>(global);
            scoped.forEach((jar, paths) -> { if (jar.matcher(root.name).matches()) excludes.addAll(paths); });
            for (var layer : root.layers.values()) {
                for (var entry : layer.entrySet()) {
                    if (options.minimize && !entry.getValue().directory() && classFile(entry.getKey())) {
                        originalClasses.add(binaryName(entry.getKey()));
                    }
                }
                layer.entrySet().removeIf(entry -> excluded(excludes, entry.getKey(), entry.getValue().directory()));
            }
        }
        if (!options.minimize) return;
        require(options.externalClassPath.isEmpty() && options.externalModulePath.isEmpty(),
                "Minimization requires embedded dependencies");
        Map<String, Set<String>> graph = new HashMap<>();
        Set<String> seeds = new LinkedHashSet<>();
        if (mainClass != null) seeds.add(mainClass);
        for (String name : originalClasses) if (matches(keep, name)) seeds.add(name);
        for (int index = 0; index < roots.size(); index++) {
            Resources root = roots.get(index);
            boolean retain = index == 0 || index > options.classPath.size() || matches(keepJars, root.name);
            for (var layer : root.layers.values()) {
                for (var entry : layer.entrySet()) {
                    String path = entry.getKey();
                    Resources.Node node = entry.getValue();
                    if (node.directory()) continue;
                    if (path.endsWith(".class")) {
                        require(node.bytes() != null, "Cannot minimize a symbolic class resource: " + path);
                        Set<String> references = references(node.bytes(), root.name + ":" + path);
                        if (path.equals("module-info.class")) {
                            // Descriptors, including uses/provides declarations, remain valid without rewriting.
                            seeds.addAll(references);
                        } else {
                            String name = binaryName(path);
                            graph.computeIfAbsent(name, ignored -> new HashSet<>()).addAll(references);
                            if (retain || path.endsWith("/package-info.class") || path.equals("package-info.class")) seeds.add(name);
                        }
                    } else if (path.startsWith("META-INF/services/")) {
                        require(node.bytes() != null, "Cannot minimize a symbolic service descriptor: " + path);
                        seeds.add(path.substring("META-INF/services/".length()));
                        for (String line : Input.utf8(node.bytes()).split("\\R")) {
                            int comment = line.indexOf('#');
                            String provider = (comment < 0 ? line : line.substring(0, comment)).trim();
                            if (!provider.isEmpty()) seeds.add(provider);
                        }
                    }
                }
            }
        }
        Set<String> reachable = new HashSet<>();
        ArrayDeque<String> pending = new ArrayDeque<>(seeds);
        while (!pending.isEmpty()) {
            String name = pending.removeFirst();
            if (!reachable.add(name)) continue;
            Set<String> references = graph.get(name);
            require(references != null || !originalClasses.contains(name),
                    "Resource exclusions remove a required class: " + name);
            if (references != null) pending.addAll(references);
        }
        for (Resources root : roots) {
            for (var layer : root.layers.values()) {
                layer.entrySet().removeIf(entry -> !entry.getValue().directory() && classFile(entry.getKey())
                        && !reachable.contains(binaryName(entry.getKey())));
            }
        }
    }

    /// Collects type references from descriptors, signatures, annotations, instructions, frames, and module metadata.
    private static Set<String> references(byte[] bytes, String location) throws IOException {
        Set<String> result = new HashSet<>();
        try {
            new ClassReader(bytes).accept(new ClassRemapper(new ClassWriter(0), new ReferenceCollector(result)), 0);
        } catch (RuntimeException failure) {
            throw new IOException("Cannot analyze class resource: " + location, failure);
        }
        return result;
    }

    /// Records type names while leaving bytecode identities unchanged.
    private static final class ReferenceCollector extends Remapper {
        /// References collected from one class variant.
        private final Set<String> references;

        /// Creates a collector without resolving or loading application classes.
        ReferenceCollector(Set<String> references) {
            super(Opcodes.ASM9);
            this.references = references;
        }

        /// Records an internal JVM type name and returns it unchanged.
        @Override
        public String map(String internalName) {
            references.add(internalName.replace('/', '.'));
            return internalName;
        }
    }

    /// Identifies ordinary class resources; module descriptors are retained separately.
    private static boolean classFile(String path) {
        return path.endsWith(".class") && !path.equals("module-info.class");
    }

    /// Converts a logical class resource path to its binary name.
    private static String binaryName(String path) {
        return path.substring(0, path.length() - 6).replace('/', '.');
    }

    /// Matches a resource or any of its parent directories, so an excluded subtree stays excluded.
    private static boolean excluded(List<Pattern> rules, String path, boolean directory) {
        if (path.isEmpty()) return false;
        if (matches(rules, path) || directory && matches(rules, path + "/")) return true;
        for (int slash = path.indexOf('/'); slash >= 0; slash = path.indexOf('/', slash + 1)) {
            if (matches(rules, path.substring(0, slash)) || matches(rules, path.substring(0, slash + 1))) return true;
        }
        return false;
    }

    /// Tests whether any compiled rule matches the entire input.
    private static boolean matches(List<Pattern> patterns, String value) {
        return patterns.stream().anyMatch(pattern -> pattern.matcher(value).matches());
    }

    /// Compiles independent glob rules without resolving filesystem paths.
    private static List<Pattern> patterns(List<String> values, char separator, boolean subtree) throws IOException {
        List<Pattern> result = new ArrayList<>();
        for (String value : values) result.add(pattern(value, separator, subtree));
        return result;
    }

    /// Compiles literal text plus *, ?, and **; a double star followed by a separator permits zero components.
    private static Pattern pattern(String value, char separator, boolean subtree) throws IOException {
        require(value != null && !value.isEmpty() && value.indexOf('\\') < 0 && value.indexOf(0) < 0
                && !value.startsWith("/") && (separator != '.' || value.indexOf('/') < 0), "Invalid selection pattern: " + value);
        if (subtree && value.endsWith("/")) value += "**";
        StringBuilder regex = new StringBuilder();
        String component = separator == '/' ? "[^/]" : "[^.]";
        for (int index = 0; index < value.length(); index++) {
            char ch = value.charAt(index);
            if (ch == '*') {
                if (index + 1 < value.length() && value.charAt(index + 1) == '*') {
                    index++;
                    if (index + 1 < value.length() && value.charAt(index + 1) == separator) {
                        regex.append("(?:.*").append(Pattern.quote(String.valueOf(separator))).append(")?");
                        index++;
                    } else regex.append(".*");
                } else regex.append(component).append('*');
            } else if (ch == '?') regex.append(component);
            else regex.append(Pattern.quote(String.valueOf(ch)));
        }
        return Pattern.compile(regex.toString());
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;
import javax.tools.ToolProvider;

import org.glavo.janex.reader.JanexReader;

/// Exercises real bytecode, reflection roots, service providers, resource filters, and Multi-Release reachability.
public final class MinimizationTest {
    /// Prevents construction.
    private MinimizationTest() { }

    /// Builds independent fixtures and checks their selected resources and standalone execution.
    /// @param args unused arguments
    /// @throws Exception if compilation, packing, execution, or a behavioral assertion fails
    public static void main(String[] args) throws Exception {
        Path root = Files.createTempDirectory("janex-minimization-");
        try {
            fixture(root);
            PackOptions original = options(root, "original");
            original.minimize = false;
            original.excludes.add("unused-resource.txt");
            JanexWriter.write(original);
            check(files(original, "dependency-1.jar").contains("dep/Unused.class"), "Default unexpectedly minimizes");
            check(!files(original, "dependency-1.jar").contains("unused-resource.txt"), "Filtering requires minimization");

            PackOptions minimized = options(root, "minimized");
            minimized.keepClasses.add("dep.Reflective");
            minimized.excludes.add("META-INF/maven/**");
            minimized.jarExcludes.put("dependency-?.jar", List.of("native/", "**/scoped.txt"));
            minimized.withLauncher = true;
            JanexWriter.write(minimized);
            Set<String> dependency = files(minimized, "dependency-1.jar");
            for (String name : List.of("Used", "Helper", "Reflective", "ReflectionHelper", "Service", "Provider",
                    "Marker", "AnnotationOnly", "GenericOnly", "LambdaOnly", "Versioned", "BaseOnly", "FutureOnly",
                    "package-info", "PackageMarker", "PackageOnly")) {
                check(dependency.contains("dep/" + name + ".class"), "Missing reachable class: " + name);
            }
            for (String name : List.of("Unused", "UnusedProvider", "CycleA", "CycleB")) {
                check(!dependency.contains("dep/" + name + ".class"), "Unused class retained: " + name);
            }
            check(!dependency.contains("native/windows/test.dll") && !dependency.contains("scoped.txt"), "Scoped exclusion ignored");
            check(!dependency.contains("META-INF/maven/dependency/pom.xml"), "Global exclusion ignored");
            check(dependency.contains("unused-resource.txt"), "Ordinary resource removed implicitly");
            check(dependency.contains("data.class/value.txt"), "Resource directory was mistaken for a class");
            check(files(minimized, "application.jar").contains("scoped.txt"), "Scoped exclusion affected another root");
            check(files(minimized, "application.jar").contains("app/Unused.class"), "Primary class removed");
            run(minimized.output);

            PackOptions duplicate = options(root, "duplicate");
            duplicate.keepClasses.addAll(minimized.keepClasses);
            duplicate.excludes.addAll(minimized.excludes);
            duplicate.jarExcludes.putAll(minimized.jarExcludes);
            duplicate.withLauncher = true;
            JanexWriter.write(duplicate);
            check(Arrays.equals(Files.readAllBytes(minimized.output), Files.readAllBytes(duplicate.output)), "Minimization is not reproducible");

            PackOptions withoutKeep = options(root, "without-keep");
            JanexWriter.write(withoutKeep);
            check(!files(withoutKeep, "dependency-1.jar").contains("dep/Reflective.class"), "String literal treated as a static class reference");

            PackOptions glob = options(root, "glob");
            glob.keepClasses.add("dep.**");
            JanexWriter.write(glob);
            check(files(glob, "dependency-1.jar").contains("dep/CycleA.class"), "Class glob ignored");
            PackOptions jar = options(root, "keep-jar");
            jar.keepJars.add("dependency-*.jar");
            JanexWriter.write(jar);
            check(files(jar, "dependency-1.jar").contains("dep/Unused.class"), "Whole-JAR keep ignored");

            PackOptions module = options(root, "module");
            module.classPath.clear();
            module.modulePath.add(root.resolve("dependency-1.jar"));
            JanexWriter.write(module);
            check(files(module, "dependency-1.jar").contains("dep/Unused.class"), "Module-path API removed");

            for (String exclusion : List.of("dep/Helper.class", "dep/Reflective.class", "dep/Provider.class")) {
                PackOptions conflict = options(root, "conflict-" + exclusion.substring(4));
                conflict.keepClasses.add("dep.Reflective");
                conflict.excludes.add(exclusion);
                fails(conflict, "required class");
            }
            PackOptions external = options(root, "external");
            external.externalClassPath.add(new PackOptions.ExternalDependency("https://example.org/external.jar", null));
            fails(external, "embedded dependencies");
            PackOptions invalid = options(root, "invalid");
            invalid.keepClasses.add("dep/Reflective");
            fails(invalid, "Invalid selection pattern");
            System.out.println("Java minimization checks passed.");
        } finally {
            try (var paths = Files.walk(root)) {
                for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) Files.deleteIfExists(path);
            }
        }
    }

    /// Creates fresh options with one embedded dependency and minimization enabled.
    private static PackOptions options(Path root, String name) {
        PackOptions options = new PackOptions(root.resolve("application.jar"), root.resolve(name + ".janex"));
        options.classPath.add(root.resolve("dependency-1.jar"));
        options.minimize = true;
        return options;
    }

    /// Returns the selected logical filenames of one preserved input root.
    private static Set<String> files(PackOptions options, String jar) throws Exception {
        try (JanexReader reader = new JanexReader(options.output)) {
            return Set.copyOf(reader.launch("main").resources.roots().stream()
                    .filter(root -> root.name().equals(jar)).findFirst().orElseThrow().files().keySet());
        }
    }

    /// Requires a selection failure to leave the output absent.
    private static void fails(PackOptions options, String message) throws Exception {
        try {
            JanexWriter.write(options);
            throw new AssertionError("Invalid selection accepted");
        } catch (IOException expected) {
            check(expected.getMessage().contains(message), expected.toString());
            check(!Files.exists(options.output), "Failed selection produced output");
        }
    }

    /// Runs reflection, service discovery, and a method reference from the minimized executable package.
    private static void run(Path packageFile) throws Exception {
        Path java = Path.of(System.getProperty("java.home"), "bin",
                System.getProperty("os.name").startsWith("Windows") ? "java.exe" : "java");
        Process process = new ProcessBuilder(java.toString(), "-jar", packageFile.toString()).redirectErrorStream(true).start();
        String output = new String(process.getInputStream().readAllBytes(), StandardCharsets.UTF_8);
        check(process.waitFor() == 0 && output.contains("used|reflection|service|lambda|base"), output);
    }

    /// Compiles a primary application, deliberately unreachable classes, and a future Multi-Release variant.
    private static void fixture(Path root) throws Exception {
        Map<String, String> sources = new LinkedHashMap<>();
        sources.put("app.Main", """
                package app;
                @dep.Marker(dep.AnnotationOnly.class)
                public class Main {
                    public java.util.List<dep.GenericOnly> values;
                    public static void main(String[] args) throws Exception {
                        Object reflection = Class.forName(System.getProperty("reflective.type", "dep.Reflective"))
                            .getMethod("value").invoke(null);
                        String service = java.util.ServiceLoader.load(dep.Service.class).iterator().next().value();
                        java.util.function.Supplier<String> lambda = dep.LambdaOnly::value;
                        System.out.println(dep.Used.value() + "|" + reflection + "|" + service + "|" + lambda.get() + "|" + dep.Versioned.value());
                    }
                }
                """);
        sources.put("app.Unused", "package app; public class Unused {}");
        sources.put("dep.Marker", "package dep; public @interface Marker { Class<?> value(); }");
        sources.put("dep.package-info", "@dep.PackageMarker(dep.PackageOnly.class) package dep;");
        sources.put("dep.PackageMarker", "package dep; public @interface PackageMarker { Class<?> value(); }");
        sources.put("dep.Service", "package dep; public interface Service { String value(); }");
        sources.put("dep.Provider", "package dep; public class Provider implements Service { public String value() { return \"service\"; } }");
        sources.put("dep.UnusedProvider", "package dep; public class UnusedProvider implements Service { public String value() { return \"unused\"; } }");
        for (String name : List.of("AnnotationOnly", "GenericOnly", "PackageOnly", "Unused")) sources.put("dep." + name, "package dep; public class " + name + " {}");
        for (var pair : Map.of("Used", "Helper", "Reflective", "ReflectionHelper", "Versioned", "BaseOnly").entrySet()) {
            sources.put("dep." + pair.getKey(), "package dep; public class " + pair.getKey()
                    + " { public static String value() { return " + pair.getValue() + ".value(); } }");
        }
        for (var pair : Map.of("Helper", "used", "ReflectionHelper", "reflection", "LambdaOnly", "lambda", "BaseOnly", "base", "FutureOnly", "future").entrySet()) {
            sources.put("dep." + pair.getKey(), "package dep; public class " + pair.getKey()
                    + " { public static String value() { return \"" + pair.getValue() + "\"; } }");
        }
        sources.put("dep.CycleA", "package dep; public class CycleA { CycleB value; }");
        sources.put("dep.CycleB", "package dep; public class CycleB { CycleA value; }");
        Path classes = root.resolve("classes");
        compile(root.resolve("src"), classes, sources, List.of());
        Path future = root.resolve("future");
        compile(root.resolve("future-src"), future, Map.of("dep.Versioned",
                "package dep; public class Versioned { public static String value() { return FutureOnly.value(); } }"),
                List.of("-classpath", classes.toString()));
        for (boolean application : List.of(true, false)) {
            try (ZipOutputStream zip = new ZipOutputStream(Files.newOutputStream(root.resolve(application ? "application.jar" : "dependency-1.jar")))) {
                entry(zip, "META-INF/MANIFEST.MF", ("Manifest-Version: 1.0\r\n"
                        + (application ? "Main-Class: app.Main\r\n" : "Multi-Release: true\r\n") + "\r\n").getBytes(StandardCharsets.UTF_8));
                try (var paths = Files.walk(classes.resolve(application ? "app" : "dep"))) {
                    for (Path path : paths.filter(Files::isRegularFile).sorted().toList()) {
                        entry(zip, classes.relativize(path).toString().replace('\\', '/'), Files.readAllBytes(path));
                    }
                }
                entry(zip, "scoped.txt", new byte[]{42});
                if (!application) {
                    entry(zip, "META-INF/versions/99/dep/Versioned.class", Files.readAllBytes(future.resolve("dep/Versioned.class")));
                    entry(zip, "META-INF/services/dep.Service", "# retained provider\ndep.Provider # comment\n".getBytes(StandardCharsets.UTF_8));
                    entry(zip, "META-INF/maven/dependency/pom.xml", new byte[]{42});
                    entry(zip, "native/windows/test.dll", new byte[]{42});
                    entry(zip, "unused-resource.txt", new byte[]{42});
                    entry(zip, "data.class/", new byte[0]);
                    entry(zip, "data.class/value.txt", new byte[]{42});
                }
            }
        }
    }

    /// Compiles source fixtures using a stable Java 17 bytecode target.
    private static void compile(Path source, Path output, Map<String, String> sources, List<String> extra) throws Exception {
        Files.createDirectories(output);
        List<String> arguments = new ArrayList<>(List.of("--release", "17", "-d", output.toString()));
        arguments.addAll(extra);
        for (var entry : sources.entrySet()) {
            Path file = source.resolve(entry.getKey().replace('.', '/') + ".java");
            Files.createDirectories(file.getParent());
            Files.writeString(file, entry.getValue());
            arguments.add(file.toString());
        }
        check(ToolProvider.getSystemJavaCompiler().run(null, null, null, arguments.toArray(String[]::new)) == 0, "Fixture compilation failed");
    }

    /// Writes one deterministic ZIP resource.
    private static void entry(ZipOutputStream zip, String name, byte[] content) throws IOException {
        ZipEntry entry = new ZipEntry(name);
        entry.setTime(0);
        zip.putNextEntry(entry);
        zip.write(content);
        zip.closeEntry();
    }

    /// Fails a behavioral assertion independently of JVM assertion flags.
    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
}

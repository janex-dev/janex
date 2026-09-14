// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.gradle;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

import org.gradle.testkit.runner.BuildResult;
import org.gradle.testkit.runner.GradleRunner;
import org.gradle.testkit.runner.TaskOutcome;

/// Exercises packaging, execution, task invalidation, and failure recovery with the real CLI.
public final class JanexPluginTest {
    /// Prevents instantiation of the test entry point.
    private JanexPluginTest() {
    }

    /// Runs isolated TestKit builds beneath the configured test directory.
    ///
    /// @param args unused command-line arguments
    /// @throws Exception if a fixture, build, or assertion fails
    public static void main(String[] args) throws Exception {
        Path executable = Path.of(System.getProperty("janex.test.executable")).toAbsolutePath();
        require(Files.isRegularFile(executable), "Build the Janex CLI first: " + executable);
        Path directory = Path.of(System.getProperty("janex.test.directory"));
        Files.createDirectories(directory);
        classpathPackaging(Files.createTempDirectory(directory, "classpath-"), executable);
        modularPackaging(Files.createTempDirectory(directory, "modules-"), executable);
        publishedPlugin(Files.createTempDirectory(directory, "published-"), executable);
        javaOnlyPackaging(Files.createTempDirectory(directory, "java-only-"));
        System.out.println("Janex Gradle plugin functional checks passed.");
    }

    /// Verifies dependencies, resources, argument boundaries, incremental builds, and failed repacks.
    private static void classpathPackaging(Path project, Path executable) throws Exception {
        fixture(project, false);
        BuildResult first = build(project, executable, false, "assemble");
        require(first.task(":janexPack").getOutcome() == TaskOutcome.SUCCESS, first.getOutput());
        Path output = project.resolve("build/distributions/fixture.janex");
        require(run(List.of(executable.toString(), "run", "--allow-unsigned", "--java", javaExecutable(), output.toString()))
                .contains("hello|resource|configured|4"), "Rust could not launch the Java-written package");
        String launch = runJar(output);
        require(launch.contains("hello|resource|configured|4"), launch);
        require(launch.contains("0:\n"), launch);
        require(launch.contains("2:1f680"), launch);
        require(launch.contains("6:2d,2d,68,65,6c,70"), launch);
        Path nativeWritten = project.resolve("rust-written.janex");
        run(List.of(executable.toString(), "pack", project.resolve("build/libs/fixture.jar").toString(),
                "--output", nativeWritten.toString(), "--main-class", "demo.Main", "--with-launcher",
                "--class-path", project.resolve("dependency/build/libs/dependency.jar").toString(),
                "--jvm-option=-Ddemo.flag=configured"));
        require(runJar(nativeWritten).contains("hello|resource|configured|0"), "Java could not launch the Rust-written package");
        String java8Home = System.getenv("JANEX_TEST_JAVA8_HOME");
        String java8 = java8Home == null ? null : Path.of(java8Home, "bin",
                System.getProperty("os.name").startsWith("Windows") ? "java.exe" : "java").toString();
        if (java8 != null) {
            require(run(List.of(java8, "-jar", output.toString()))
                    .contains("hello|resource|configured|4"), "Java 8 could not launch the Java-written package");
        }
        List<String> runtimes = new ArrayList<>(List.of(javaExecutable()));
        if (java8 != null) runtimes.add(java8);
        for (String runtime : runtimes) {
            for (String mode : List.of("bootstrap", "direct")) {
                String result = run(List.of(executable.toString(), "run", "--allow-unsigned", "--java", runtime,
                        "--launch-mode", mode, output.toString()));
                require(result.contains("hello|resource|configured|4"), result);
            }
        }

        BuildResult second = build(project, executable, false, "assemble");
        require(second.getOutput().contains("Reusing configuration cache"), second.getOutput());
        require(second.task(":janexPack").getOutcome() == TaskOutcome.UP_TO_DATE, second.getOutput());

        write(project, "dependency/src/main/resources/dependency.txt", "changed");
        BuildResult changed = build(project, executable, false, "assemble");
        require(changed.task(":janexPack").getOutcome() == TaskOutcome.SUCCESS, changed.getOutput());
        require(runJar(output).contains("hello|changed|configured|4"), "Dependency change was not packaged");

        String original = Files.readString(project.resolve("build.gradle.kts"));
        write(project, "build.gradle.kts", original + "\njanex { classPath.setFrom(configurations.runtimeClasspath.get().files.reversed()) }\n");
        BuildResult reordered = build(project, executable, false, "assemble");
        require(reordered.task(":janexPack").getOutcome() == TaskOutcome.SUCCESS, reordered.getOutput());
        require(runJar(output).contains("hello|other|configured|4"), "Classpath order was not preserved");

        byte[] previous = Files.readAllBytes(output);
        write(project, "build.gradle.kts", original + "\njanex { javaVersion.set(\"invalid\") }\n");
        build(project, executable, true, "janexPack");
        require(Arrays.equals(previous, Files.readAllBytes(output)), "Failed packaging replaced the output");
        try (var files = Files.list(output.getParent())) {
            require(files.noneMatch(path -> path.getFileName().toString().startsWith(".janex-")),
                    "Temporary packaging output was retained");
        }

        write(project, "build.gradle.kts", original + "\njanex { withLauncher.set(false) }\n");
        build(project, executable, false, "janexPack");
        String direct = run(List.of(executable.toString(), "run", "--allow-unsigned", "--launch-mode", "direct",
                "--java", javaExecutable(), output.toString()));
        require(direct.contains("hello|changed|configured|4"), direct);

        Path nativeLauncher = executable.resolveSibling(
                executable.getFileName().toString().endsWith(".exe") ? "janex-launcher.exe" : "janex-launcher");
        String os = System.getProperty("os.name");
        if (os.startsWith("Windows") || os.equals("Linux") || os.equals("FreeBSD")) {
            require(Files.isRegularFile(nativeLauncher), "Build the native launcher first: " + nativeLauncher);
            write(project, "build.gradle.kts", original + """

                    janex {
                        nativeLauncher.set(file("%s"))
                        nativeLaunchMode.set("direct")
                        outputFile.set(layout.buildDirectory.file("distributions/native%s"))
                    }
                    """.formatted(kotlinPath(nativeLauncher), os.startsWith("Windows") ? ".exe" : ""));
            build(project, executable, false, "janexPack");
            Path nativeOutput = project.resolve("build/distributions/native" + (os.startsWith("Windows") ? ".exe" : ""));
            require(runJar(nativeOutput).contains("hello|changed|configured|4"), "Native prefix broke the JAR tail");
            require(run(List.of(nativeOutput.toString())).contains("hello|changed|configured|4"),
                    "Native launcher failed");
        }
    }

    /// Verifies main-module conventions and embedded module dependencies.
    private static void modularPackaging(Path project, Path executable) throws Exception {
        fixture(project, true);
        build(project, executable, false, "janexPack");
        String output = runJar(project.resolve("build/distributions/fixture.janex"));
        require(output.contains("hello|resource|configured|4"), output);
    }

    /// Resolves the plugin marker and implementation through an ordinary Maven repository.
    private static void publishedPlugin(Path project, Path executable) throws Exception {
        fixture(project, false);
        Path settings = project.resolve("settings.gradle.kts");
        String repository = System.getProperty("janex.test.repository");
        Files.writeString(settings, """
                pluginManagement {
                    repositories {
                        maven { url = uri("%s") }
                        mavenCentral()
                    }
                }
                """.formatted(repository) + Files.readString(settings));
        Path script = project.resolve("build.gradle.kts");
        Files.writeString(script, Files.readString(script)
                .replace("id(\"org.glavo.janex\")", "id(\"org.glavo.janex\") version \"0.1.0\""));
        GradleRunner.create().withProjectDir(project.toFile())
                .withArguments("janexPack", "--configuration-cache", "--stacktrace")
                .build();
        require(runJar(project.resolve("build/distributions/fixture.janex"))
                .contains("hello|resource|configured|4"), "Published plugin produced an invalid package");
    }

    /// Verifies that packaging needs no CLI and can restore a package from Gradle's build cache.
    private static void javaOnlyPackaging(Path project) throws Exception {
        fixture(project, false);
        List<String> arguments = List.of("janexPack", "--build-cache", "--configuration-cache", "--stacktrace",
                "-PjanexExecutable=/does/not/exist");
        GradleRunner.create().withProjectDir(project.toFile()).withPluginClasspath().withArguments(arguments).build();
        Path output = project.resolve("build/distributions/fixture.janex");
        byte[] original = Files.readAllBytes(output);
        Files.delete(output);
        BuildResult restored = GradleRunner.create().withProjectDir(project.toFile()).withPluginClasspath()
                .withArguments(arguments).build();
        require(restored.task(":janexPack").getOutcome() == TaskOutcome.FROM_CACHE, restored.getOutput());
        require(Arrays.equals(original, Files.readAllBytes(output)), "Build cache changed the package");
        require(runJar(output).contains("hello|resource|configured|4"), "Java-only packaging failed");
        String script = Files.readString(project.resolve("build.gradle.kts"));
        write(project, "build.gradle.kts", script + "\njanex { compression.set(false) }\n");
        BuildResult uncompressed = GradleRunner.create().withProjectDir(project.toFile()).withPluginClasspath()
                .withArguments(arguments).build();
        require(uncompressed.task(":janexPack").getOutcome() == TaskOutcome.SUCCESS, uncompressed.getOutput());
        require(Files.size(output) > original.length, "Disabling compression did not change the package");
        require(runJar(output).contains("hello|resource|configured|4"), "Uncompressed packaging failed");
        write(project, "build.gradle.kts", script);
        BuildResult recompressed = GradleRunner.create().withProjectDir(project.toFile()).withPluginClasspath()
                .withArguments(arguments).build();
        require(recompressed.task(":janexPack").getOutcome() == TaskOutcome.FROM_CACHE, recompressed.getOutput());
        require(Arrays.equals(original, Files.readAllBytes(output)), "Compression cache key changed the package");
    }

    /// Writes a Kotlin DSL application with a project dependency and duplicate dependency resources.
    private static void fixture(Path project, boolean modular) throws IOException {
        write(project, "settings.gradle.kts", """
                rootProject.name = "fixture"
                include("dependency", "other")
                """);
        write(project, "build.gradle.kts", """
                plugins {
                    id("org.glavo.janex")
                    application
                }
                allprojects {
                    tasks.withType<JavaCompile>().configureEach { options.release.set(%d) }
                }
                application {
                    mainClass.set("demo.Main")
                    applicationDefaultJvmArgs = listOf("-Ddemo.flag=configured")
                    %s
                }
                dependencies {
                    implementation(project(":dependency"))
                    runtimeOnly(project(":other"))
                }
                janex {
                    arguments.addAll("", "two words", "\\uD83D\\uDE80", "--help")
                }
                """.formatted(modular ? 9 : 8, modular ? "mainModule.set(\"demo.app\")" : ""));
        write(project, "dependency/build.gradle.kts", "plugins { `java-library` }\n");
        write(project, "other/build.gradle.kts", "plugins { `java-library` }\n");
        write(project, "dependency/src/main/java/dependency/Greeting.java", """
                package dependency;
                /// Supplies a message and a dependency resource for the application fixture.
                public class Greeting {
                    /// Creates the fixture helper.
                    public Greeting() { }
                    /// Returns the fixture message.
                    public static String message() { return "hello"; }
                    /// Reads the dependency resource through the owning class loader or module.
                    public static String resource() throws Exception {
                        try (java.io.InputStream input = Greeting.class.getResourceAsStream("/dependency.txt")) {
                            java.io.ByteArrayOutputStream bytes = new java.io.ByteArrayOutputStream();
                            for (int ch; (ch = input.read()) >= 0;) bytes.write(ch);
                            return new String(bytes.toByteArray(), java.nio.charset.StandardCharsets.UTF_8);
                        }
                    }
                }
                """);
        write(project, "dependency/src/main/resources/dependency.txt", "resource");
        write(project, "other/src/main/resources/dependency.txt", "other");
        write(project, "src/main/resources/packed.txt", "compressible-resource\n".repeat(20000));
        write(project, "src/main/java/demo/Main.java", """
                package demo;
                import dependency.Greeting;
                /// Reports dependencies, JVM options, and argument boundaries for the fixture.
                public class Main {
                    /// Creates the application fixture.
                    public Main() { }
                    /// Prints launch state and Unicode code points for each argument.
                    public static void main(String[] args) throws Exception {
                        String pattern = "compressible-resource\\n";
                        int length = 0;
                        try (java.io.InputStream input = Main.class.getResourceAsStream("/packed.txt")) {
                            for (int ch; (ch = input.read()) >= 0; length++) {
                                if (ch != pattern.charAt(length % pattern.length())) {
                                    throw new AssertionError("Compressed resource changed");
                                }
                            }
                        }
                        if (length != pattern.length() * 20000) throw new AssertionError("Truncated resource");
                        System.out.println(Greeting.message() + "|" + Greeting.resource() + "|"
                                + System.getProperty("demo.flag") + "|" + args.length);
                        for (String arg : args) {
                            System.out.println(arg.length() + ":" + arg.codePoints()
                                    .mapToObj(Integer::toHexString).collect(java.util.stream.Collectors.joining(",")));
                        }
                    }
                }
                """);
        if (modular) {
            write(project, "src/main/java/module-info.java", "module demo.app { requires demo.dependency; }\n");
            write(project, "dependency/src/main/java/module-info.java", "module demo.dependency { exports dependency; }\n");
        }
    }

    /// Runs a fixture build with strict configuration-cache validation; the CLI is used only by interoperability checks.
    private static BuildResult build(Path project, Path executable, boolean failure, String task) {
        GradleRunner runner = GradleRunner.create().withProjectDir(project.toFile()).withPluginClasspath()
                .withArguments(task, "--configuration-cache", "--configuration-cache-problems=fail", "--stacktrace");
        return failure ? runner.buildAndFail() : runner.build();
    }

    /// Launches a package through its appended JAR launcher.
    private static String runJar(Path output) throws Exception {
        return run(List.of(javaExecutable(), "-jar", output.toString()));
    }

    /// Returns the Java executable of the test process, avoiding runtime selection ambiguity.
    private static String javaExecutable() {
        return Path.of(System.getProperty("java.home"), "bin",
                System.getProperty("os.name").startsWith("Windows") ? "java.exe" : "java").toString();
    }

    /// Executes a command and returns normalized UTF-8 output, failing on a nonzero exit status.
    private static String run(List<String> command) throws Exception {
        ProcessBuilder builder = new ProcessBuilder(new ArrayList<>(command)).redirectErrorStream(true);
        builder.environment().put("JANEX_JAVA", javaExecutable());
        builder.environment().remove("JANEX_LAUNCH_MODE");
        Process process = builder.start();
        String output = new String(process.getInputStream().readAllBytes(), StandardCharsets.UTF_8)
                .replace("\r\n", "\n");
        require(process.waitFor() == 0, output);
        return output;
    }

    /// Escapes a filesystem path for a Kotlin string literal.
    private static String kotlinPath(Path path) {
        return path.toString().replace("\\", "/").replace("\"", "\\\"").replace("$", "\\$");
    }

    /// Writes a UTF-8 fixture file, creating its parent directories as needed.
    private static void write(Path root, String name, String contents) throws IOException {
        Path file = root.resolve(name);
        Files.createDirectories(file.getParent());
        Files.writeString(file, contents);
    }

    /// Reports a failed behavioral assertion independently of JVM assertion flags.
    private static void require(boolean condition, String message) {
        if (!condition) {
            throw new AssertionError(message);
        }
    }
}

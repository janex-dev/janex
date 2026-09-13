// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.gradle;

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
        missingExecutable(Files.createTempDirectory(directory, "missing-"));
        System.out.println("Janex Gradle plugin functional checks passed.");
    }

    /// Verifies dependencies, resources, argument boundaries, incremental builds, and failed repacks.
    private static void classpathPackaging(Path project, Path executable) throws Exception {
        fixture(project, false);
        BuildResult first = build(project, executable, false, "assemble");
        require(first.task(":janexPack").getOutcome() == TaskOutcome.SUCCESS, first.getOutput());
        Path output = project.resolve("build/distributions/fixture.janex");
        String launch = runJar(output);
        require(launch.contains("hello|resource|configured|4"), launch);
        require(launch.contains("0:\n"), launch);
        require(launch.contains("2:1f680"), launch);
        require(launch.contains("6:2d,2d,68,65,6c,70"), launch);

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
        Files.writeString(settings, "pluginManagement { repositories { maven { url = uri(\""
                + repository + "\") } } }\n" + Files.readString(settings));
        Path script = project.resolve("build.gradle.kts");
        Files.writeString(script, Files.readString(script)
                .replace("id(\"org.janex\")", "id(\"org.janex\") version \"0.1.0\""));
        GradleRunner.create().withProjectDir(project.toFile())
                .withArguments("janexPack", "--configuration-cache", "--stacktrace", "-PjanexExecutable=" + executable)
                .build();
        require(runJar(project.resolve("build/distributions/fixture.janex"))
                .contains("hello|resource|configured|4"), "Published plugin produced an invalid package");
    }

    /// Verifies that configuration-only tasks do not require a CLI and packaging reports its absence.
    private static void missingExecutable(Path project) throws IOException {
        write(project, "settings.gradle.kts", "rootProject.name = \"missing\"\n");
        write(project, "build.gradle.kts", "plugins { id(\"org.janex\") }\n");
        GradleRunner.create().withProjectDir(project.toFile()).withPluginClasspath()
                .withArguments("help", "--configuration-cache", "--stacktrace").build();
        BuildResult failure = GradleRunner.create().withProjectDir(project.toFile()).withPluginClasspath()
                .withArguments("janexPack", "--configuration-cache", "--stacktrace").buildAndFail();
        require(failure.getOutput().contains("executable"), failure.getOutput());
    }

    /// Writes a Kotlin DSL application with a project dependency and duplicate dependency resources.
    private static void fixture(Path project, boolean modular) throws IOException {
        write(project, "settings.gradle.kts", """
                rootProject.name = "fixture"
                include("dependency", "other")
                """);
        write(project, "build.gradle.kts", """
                plugins {
                    id("org.janex")
                    application
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
                """.formatted(modular ? "mainModule.set(\"demo.app\")" : ""));
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
                        try (var input = Greeting.class.getResourceAsStream("/dependency.txt")) {
                            return new String(input.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8);
                        }
                    }
                }
                """);
        write(project, "dependency/src/main/resources/dependency.txt", "resource");
        write(project, "other/src/main/resources/dependency.txt", "other");
        write(project, "src/main/java/demo/Main.java", """
                package demo;
                import dependency.Greeting;
                /// Reports dependencies, JVM options, and argument boundaries for the fixture.
                public class Main {
                    /// Creates the application fixture.
                    public Main() { }
                    /// Prints launch state and Unicode code points for each argument.
                    public static void main(String[] args) throws Exception {
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

    /// Runs a fixture build with strict configuration-cache validation and the supplied CLI.
    private static BuildResult build(Path project, Path executable, boolean failure, String task) {
        GradleRunner runner = GradleRunner.create().withProjectDir(project.toFile()).withPluginClasspath()
                .withArguments(task, "--configuration-cache", "--configuration-cache-problems=fail", "--stacktrace",
                        "-PjanexExecutable=" + executable);
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

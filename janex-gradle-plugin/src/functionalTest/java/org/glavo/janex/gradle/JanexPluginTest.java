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
import java.time.Clock;
import java.time.Instant;
import java.time.ZoneOffset;

import org.glavo.janex.writer.CmsSigner;
import org.glavo.janex.writer.OpenPgpSigner;
import org.glavo.janex.writer.JanexWriter;
import org.glavo.janex.writer.PackOptions;
import org.glavo.janex.writer.PackageSigner;
import org.glavo.janex.writer.SignatureTest;
import org.glavo.janex.writer.SigningAlgorithm;
import org.glavo.janex.reader.JanexReader;

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
        signingPackaging(Files.createTempDirectory(directory, "signing-"), executable);
        System.out.println("Janex Gradle plugin functional checks passed.");
    }

    /// Verifies dependencies, resources, argument boundaries, incremental builds, and failed repacks.
    private static void classpathPackaging(Path project, Path executable) throws Exception {
        fixture(project, false);
        BuildResult first = build(project, executable, false, "assemble");
        require(first.task(":janexPack").getOutcome() == TaskOutcome.SUCCESS, first.getOutput());
        Path output = project.resolve("build/distributions/fixture.janex");
        try (JanexReader reader = new JanexReader(output)) {
            require(reader.launch("main").resources.roots().get(0).files().get("demo/Shared0.class").transforms().length == 1,
                    "Interoperability fixture did not select CLASSFILE transforms");
        }
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

    /// Verifies signed task execution, every algorithm through Rust, native pins, and tamper rejection.
    private static void signingPackaging(Path project, Path executable) throws Exception {
        fixture(project, false);
        Path fixtures = Path.of(System.getProperty("janex.test.fixtures"));
        Path cert = fixtures.resolve("cms/rsa256.cert.pem");
        Files.copy(fixtures.resolve("cms/rsa256.encrypted.pem"), project.resolve("signing.key"));
        String original = Files.readString(project.resolve("build.gradle.kts"));
        Instant time = Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS);
        write(project, "build.gradle.kts", original + """

                janex {
                    withLauncher.set(false)
                    signing {
                        cmsCertificate.set(file("%s"))
                        cmsKey.set(file("signing.key"))
                        passwordEnvironment.set("JANEX_TEST_KEY_PASSWORD")
                        time.set("%s")
                    }
                }
                """.formatted(kotlinPath(cert), time));
        build(project, executable, false, "janexPack");
        Path output = project.resolve("build/distributions/fixture.janex");
        String launched = run(List.of(executable.toString(), "run", "--java", javaExecutable(),
                "--trust-cms-certificate", cert.toString(), output.toString()));
        require(launched.contains("hello|resource|configured|4"), launched);
        BuildResult repeated = build(project, executable, false, "janexPack");
        require(repeated.getOutput().contains("Reusing configuration cache"), repeated.getOutput());
        require(repeated.task(":janexPack").getOutcome() == TaskOutcome.SUCCESS, "Signed task was incorrectly up to date");
        byte[] previous = Files.readAllBytes(output);
        Files.writeString(project.resolve("signing.key"), "invalid private key");
        build(project, executable, true, "janexPack");
        require(Arrays.equals(previous, Files.readAllBytes(output)), "Failed signing replaced the previous package");
        write(project, "build.gradle.kts", original + """

                janex {
                    withLauncher.set(false)
                    signing {
                        openPgpKey.set(file("%s"))
                        passwordEnvironment.set("JANEX_TEST_KEY_PASSWORD")
                    }
                }
                """.formatted(kotlinPath(fixtures.resolve("openpgp/encrypted.secret.asc"))));
        build(project, executable, false, "janexPack");
        require(run(List.of(executable.toString(), "run", "--java", javaExecutable(), "--trust-openpgp-key",
                fixtures.resolve("openpgp/encrypted.public.pgp").toString(), output.toString()))
                .contains("hello|resource|configured|4"), "Encrypted OpenPGP plugin signing failed");
        for (SigningAlgorithm algorithm : SigningAlgorithm.values()) {
            String name = SignatureTest.fixture(algorithm);
            for (boolean pgp : new boolean[]{false, true}) {
                if (!pgp && name.equals("ed25519")) continue;
                PackageSigner signer = pgp ? OpenPgpSigner.load(fixtures.resolve("openpgp/" + name + ".secret.pgp"),
                        null, null, algorithm, time) : CmsSigner.load(fixtures.resolve("cms/" + name + ".cert.pem"),
                        fixtures.resolve("cms/" + name + ".key.pem"), null, algorithm);
                Path trusted = fixtures.resolve(pgp ? "openpgp/" + name + ".public.pgp" : "cms/" + name + ".cert.pem");
                String trustOption = pgp ? "--trust-openpgp-key" : "--trust-cms-certificate";
                PackOptions options = new PackOptions(project.resolve("build/libs/fixture.jar"),
                        project.resolve((pgp ? "pgp-" : "cms-") + algorithm + ".janex"));
                options.mainClass = "demo.Main";
                options.classPath.add(project.resolve("dependency/build/libs/dependency.jar"));
                options.jvmOptions.add("-Ddemo.flag=configured");
                options.signer = signer;
                options.signingClock = Clock.fixed(time, ZoneOffset.UTC);
                JanexWriter.write(options);
                for (String mode : List.of("bootstrap", "direct")) {
                    require(run(List.of(executable.toString(), "run", "--java", javaExecutable(), "--launch-mode", mode,
                            trustOption, trusted.toString(), options.output.toString()))
                            .contains("hello|resource|configured|0"), "Rust rejected Java signature " + algorithm);
                }
                if (algorithm == SigningAlgorithm.RSA_SHA256) {
                    String os = System.getProperty("os.name");
                    if (os.startsWith("Windows") || os.equals("Linux") || os.equals("FreeBSD")) {
                        PackOptions nativeOptions = new PackOptions(options.source,
                                project.resolve((pgp ? "pgp-native" : "cms-native") + (os.startsWith("Windows") ? ".exe" : "")));
                        nativeOptions.mainClass = options.mainClass;
                        nativeOptions.classPath.addAll(options.classPath);
                        nativeOptions.jvmOptions.addAll(options.jvmOptions);
                        nativeOptions.signer = signer;
                        nativeOptions.nativeLauncher = executable.resolveSibling(os.startsWith("Windows") ? "janex-launcher.exe" : "janex-launcher");
                        JanexWriter.write(nativeOptions);
                        require(run(List.of(nativeOptions.output.toString())).contains("hello|resource|configured|0"),
                                "Native launcher rejected embedded signer pin");
                    }
                }
                byte[] damaged = Files.readAllBytes(options.output);
                damaged[16] ^= 1;
                Path corrupted = project.resolve("corrupted-" + pgp + "-" + algorithm + ".janex");
                Files.write(corrupted, damaged);
                run(List.of(executable.toString(), "run", "--java", javaExecutable(), trustOption,
                        trusted.toString(), corrupted.toString()), false);
                run(List.of(executable.toString(), "run", "--allow-unsigned", "--java", javaExecutable(),
                        options.output.toString()), false);
            }
        }
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
        for (int index = 0; index < 24; index++) {
            write(project, "src/main/java/demo/Shared" + index + ".java", """
                    package demo;
                    /// Exercises shared constant-pool strings across multiple class files.
                    public class Shared%d {
                        /// Creates the fixture.
                        public Shared%d() { }
                        /// Returns the shared resource-contract description.
                        public static String value() {
                            return "Shared CLASSFILE constants must preserve exact Modified UTF-8 bytes, descriptors, names, and original class-file structure across Java and Rust readers.";
                        }
                    }
                    """.formatted(index, index));
        }
        write(project, "src/main/java/demo/Main.java", """
                package demo;
                import dependency.Greeting;
                /// Reports dependencies, JVM options, and argument boundaries for the fixture.
                public class Main {
                    /// Creates the application fixture.
                    public Main() { }
                    /// Prints launch state and Unicode code points for each argument.
                    public static void main(String[] args) throws Exception {
                        for (int i = 0; i < 24; i++) {
                            String shared = (String) Class.forName("demo.Shared" + i).getMethod("value").invoke(null);
                            if (!shared.startsWith("Shared CLASSFILE constants")) throw new AssertionError("Class constant changed");
                        }
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
        return run(command, true);
    }

    /// Executes a command and checks whether its exit status matches the expected outcome.
    private static String run(List<String> command, boolean success) throws Exception {
        ProcessBuilder builder = new ProcessBuilder(new ArrayList<>(command)).redirectErrorStream(true);
        builder.environment().put("JANEX_JAVA", javaExecutable());
        builder.environment().remove("JANEX_LAUNCH_MODE");
        Process process = builder.start();
        String output = new String(process.getInputStream().readAllBytes(), StandardCharsets.UTF_8)
                .replace("\r\n", "\n");
        require((process.waitFor() == 0) == success, "Unexpected command outcome: " + command + "\n" + output);
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

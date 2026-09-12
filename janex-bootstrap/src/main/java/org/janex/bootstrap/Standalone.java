// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.*;
import java.lang.management.ManagementFactory;
import java.nio.file.*;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.*;
import java.util.jar.*;
import org.janex.format.JanexReader;
import org.janex.bootstrap.internal.zstd.Zstandard;

/// Launches a Janex file carrying this executable JAR as its tail.
///
/// The current Java installation is reused. A child JVM receives the selected JVM options and
/// system resource loader before application code runs. Temporary launch files remain owned by
/// the parent until the child exits. This entry point checks integrity, not publisher trust.
public final class Standalone {
    /// Prevents instantiation.
    private Standalone() {
    }

    /// Reads this executable's Janex prefix and waits for the application process.
    ///
    /// @param arguments program arguments appended after the package's preset arguments
    /// @throws Exception if the package cannot be read or the child JVM cannot be started
    public static void main(String[] arguments) throws Exception {
        Path executable = Paths.get(Standalone.class.getProtectionDomain().getCodeSource().getLocation().toURI());
        int exit;
        try (Session session = new Session()) {
            Path snapshot = session.directory.resolve("snapshot.janex");
            try (InputStream input = Files.newInputStream(executable);
                 OutputStream output = Files.newOutputStream(snapshot)) {
                transfer(input, output, 512L * 1024 * 1024);
            }
            JanexReader.Launch launch;
            try (JanexReader reader = new JanexReader(snapshot, (input, length) -> {
                byte[] decoded = new byte[length];
                if (Zstandard.decompress(input, 0, input.length, decoded, 0, length) != length) {
                    throw new IOException("Zstd decoded length mismatch");
                }
                return decoded;
            }, new Dependencies())) {
                launch = reader.launch(System.getProperty("janex.application"));
            }
            launch.arguments.addAll(Arrays.asList(arguments));
            int feature = JanexReader.feature();
            if (feature < 9 && !launch.mainModule.isEmpty()) {
                throw new IOException("Module launching requires Java 9 or later");
            }
            List<String> options = new ArrayList<String>(ManagementFactory.getRuntimeMXBean().getInputArguments());
            options.addAll(launch.options);
            Path bridge = session.directory.resolve("bootstrap.jar");
            writeBridge(snapshot, bridge, launch, options, feature);
            List<String> command = new ArrayList<String>();
            command.add(Paths.get(System.getProperty("java.home"), "bin", isWindows() ? "java.exe" : "java").toString());
            command.addAll(nativeOptions(options, feature));
            if (feature >= 9) {
                command.add("--add-modules=ALL-SYSTEM");
            }
            command.add("-Djava.system.class.loader=org.janex.bootstrap.ResourceLoader");
            command.add("-cp");
            command.add(bridge.toString());
            command.add("org.janex.bootstrap.Bootstrap");
            ProcessBuilder builder = new ProcessBuilder(command).inheritIO();
            // The first JVM has already expanded these variables into its effective options.
            builder.environment().remove("JDK_JAVA_OPTIONS");
            builder.environment().remove("JAVA_TOOL_OPTIONS");
            builder.environment().remove("_JAVA_OPTIONS");
            session.process = builder.start();
            exit = session.process.waitFor();
        }
        System.exit(exit);
    }

    /// Returns whether this process runs on Windows.
    private static boolean isWindows() {
        return System.getProperty("os.name").startsWith("Windows");
    }

    /// Copies a bounded stream without closing either endpoint.
    private static void transfer(InputStream input, OutputStream output, long limit) throws IOException {
        byte[] buffer = new byte[32768];
        long count = 0;
        for (int size; (size = input.read(buffer)) != -1;) {
            count += size;
            if (count > limit) {
                throw new IOException("Standalone launch byte limit exceeded");
            }
            output.write(buffer, 0, size);
        }
    }

    /// Retains startup options while deferring indexed-module access changes.
    private static List<String> nativeOptions(List<String> options, int feature) throws IOException {
        List<String> result = new ArrayList<String>();
        if (feature >= 9) {
            String vm = System.getProperty("java.vm.name", "");
            if (vm.contains("OpenJDK") || vm.contains("HotSpot")) {
                result.add("-Xlog:cds=error");
            }
            result.add("--add-exports=java.base/jdk.internal.module=ALL-UNNAMED");
        }
        Set<String> deferred = new HashSet<String>(Arrays.asList(
                "--add-modules", "--add-reads", "--add-exports", "--add-opens", "--enable-native-access"));
        Set<String> launchOptions = new HashSet<String>(Arrays.asList(
                "-jar", "-m", "--module", "-cp", "-classpath", "--class-path", "-p", "--module-path",
                "--patch-module", "--upgrade-module-path", "--limit-modules", "--source", "--describe-module",
                "-d", "--list-modules", "--validate-modules", "--dry-run", "--help", "-help", "-h", "-?",
                "--version", "-version"));
        for (int i = 0; i < options.size(); i++) {
            String option = options.get(i);
            String key = option.split("=", 2)[0];
            if (launchOptions.contains(key) || option.startsWith("-Djava.system.class.loader=")
                    || option.startsWith("@") || option.indexOf(0) >= 0 || !option.startsWith("-")) {
                throw new IOException("Unsupported standalone JVM option: " + option + "; use Janex Host direct mode");
            }
            if (feature >= 9 && deferred.contains(key)) {
                String value;
                if (option.indexOf('=') >= 0) {
                    value = option.substring(option.indexOf('=') + 1);
                } else {
                    if (++i == options.size()) {
                        throw new IOException("Missing JVM option operand: " + key);
                    }
                    value = options.get(i);
                }
                if (key.equals("--enable-native-access")) {
                    result.add("--add-opens=java.base/java.lang=ALL-UNNAMED");
                    if (Arrays.asList(value.split(",")).contains("ALL-UNNAMED")) {
                        result.add("--enable-native-access=ALL-UNNAMED");
                    }
                }
            } else {
                result.add(option);
            }
        }
        return result;
    }

    /// Copies this JAR and embeds data consumed by the existing resource loader and entry point.
    private static void writeBridge(Path snapshot, Path output, JanexReader.Launch launch,
                                    List<String> options, int feature) throws IOException {
        try (JarFile source = new JarFile(snapshot.toFile());
             JarOutputStream jar = new JarOutputStream(Files.newOutputStream(output))) {
            Enumeration<JarEntry> entries = source.entries();
            while (entries.hasMoreElements()) {
                JarEntry entry = entries.nextElement();
                if (entry.isDirectory()) {
                    continue;
                }
                String name = entry.getName();
                if (name.equals("org/janex/bootstrap/resources.bin") || name.equals("org/janex/bootstrap/launch.bin")
                        || name.equals("org/janex/bootstrap/options.bin")) {
                    throw new IOException("Executable tail must not contain precomputed launch data");
                }
                jar.putNextEntry(new JarEntry(name));
                try (InputStream input = source.getInputStream(entry)) {
                    transfer(input, jar, 32L * 1024 * 1024);
                }
                jar.closeEntry();
            }
            jar.putNextEntry(new JarEntry("org/janex/bootstrap/resources.bin"));
            jar.write(launch.resources);
            jar.closeEntry();
            DataOutputStream data = new DataOutputStream(jar);
            jar.putNextEntry(new JarEntry("org/janex/bootstrap/launch.bin"));
            JanexReader.string(data, launch.mainModule);
            JanexReader.string(data, launch.mainClass);
            data.writeBoolean(feature >= 25 || (feature >= 21 && options.contains("--enable-preview")));
            data.writeInt(launch.arguments.size());
            for (String argument : launch.arguments) {
                JanexReader.string(data, argument);
            }
            jar.closeEntry();
            jar.putNextEntry(new JarEntry("org/janex/bootstrap/options.bin"));
            JanexReader.string(data, launch.mainModule);
            JanexReader.string(data, launch.mainClass);
            data.writeInt(options.size());
            for (String option : options) {
                JanexReader.string(data, option);
            }
            jar.closeEntry();
        }
    }

    /// Owns launch files and ensures a child does not outlive parent shutdown.
    private static final class Session implements AutoCloseable {
        /// Private directory containing only this launch's files.
        final Path directory;
        /// Shutdown callback retained until normal closure.
        final Thread hook;
        /// Child JVM, or null until process creation succeeds.
        volatile Process process;

        /// Creates a private temporary directory and registers process cleanup.
        Session() throws IOException {
            directory = Files.createTempDirectory("janex-java-");
            hook = new Thread(() -> {
                try {
                    cleanup();
                } catch (Exception ignored) {
                    // Process shutdown cannot report cleanup failures to the caller.
                }
            }, "janex-java-cleanup");
            Runtime.getRuntime().addShutdownHook(hook);
        }

        /// Terminates any remaining child before removing its private launch files.
        private synchronized void cleanup() throws IOException, InterruptedException {
            if (process != null && process.isAlive()) {
                process.destroy();
                if (!process.waitFor(5, java.util.concurrent.TimeUnit.SECONDS)) {
                    process.destroyForcibly().waitFor();
                }
            }
            if (Files.exists(directory)) {
                Files.walkFileTree(directory, new SimpleFileVisitor<Path>() {
                    /// Deletes one file without following symbolic links.
                    @Override
                    public FileVisitResult visitFile(Path file, BasicFileAttributes attributes) throws IOException {
                        Files.delete(file);
                        return FileVisitResult.CONTINUE;
                    }

                    /// Deletes a directory after its children.
                    @Override
                    public FileVisitResult postVisitDirectory(Path path, IOException failure) throws IOException {
                        if (failure != null) {
                            throw failure;
                        }
                        Files.delete(path);
                        return FileVisitResult.CONTINUE;
                    }
                });
            }
        }

        /// Removes the shutdown hook and cleans up after normal completion or failure.
        @Override
        public void close() throws IOException, InterruptedException {
            Runtime.getRuntime().removeShutdownHook(hook);
            cleanup();
        }
    }
}

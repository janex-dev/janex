// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.*;
import java.lang.management.ManagementFactory;
import java.nio.channels.Channels;
import java.nio.channels.FileChannel;
import java.nio.file.*;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.*;
import java.util.jar.*;

import org.janex.bootstrap.dependency.Dependencies;
import org.janex.bootstrap.loader.ResourceIndex;
import org.janex.bootstrap.loader.ResourceIndexes;
import org.janex.reader.Checksum;
import org.janex.reader.JanexReader;

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
        System.exit(launch(executable, arguments));
    }

    /// Launches a Janex file with an executable JAR tail and waits for its child JVM.
    ///
    /// The current Java installation, working directory, and standard streams are inherited.
    /// The input is copied to a private snapshot before validation. Temporary files are removed
    /// when the child exits or preparation fails. Recorded integrity is verified without
    /// establishing publisher trust. This method does not terminate the calling JVM.
    ///
    /// @param executable Janex file with an ordinary or ZIP64 executable JAR tail
    /// @param arguments arguments appended after the selected package's preset arguments
    /// @return the child process's exit status
    /// @throws Exception if preparation, process creation, waiting, or cleanup fails
    public static int launch(Path executable, String[] arguments) throws Exception {
        int exit;
        try (Session session = new Session()) {
            Path snapshot = session.directory.resolve("snapshot.janex");
            try (InputStream input = Files.newInputStream(executable);
                 OutputStream output = Files.newOutputStream(snapshot)) {
                transfer(input, output, 512L * 1024 * 1024);
            }
            JanexReader.Launch launch;
            long tailOffset;
            try (JanexReader reader = new JanexReader(snapshot, new Dependencies())) {
                launch = reader.launch(System.getProperty("janex.application"));
                tailOffset = reader.externalTailOffset();
            }
            byte[] resourceIndex = ResourceIndexes.encode(launch.resources);
            byte[] agentIndex = launch.agentResources == null ? null : ResourceIndexes.encode(launch.agentResources);
            launch.resources.limits().bytes((long) resourceIndex.length + (agentIndex == null ? 0 : agentIndex.length));
            launch.arguments.addAll(Arrays.asList(arguments));
            int feature = JanexReader.feature();
            if (feature < 9 && !launch.mainModule.isEmpty()) {
                throw new IOException("Module launching requires Java 9 or later");
            }
            List<String> options = new ArrayList<String>(ManagementFactory.getRuntimeMXBean().getInputArguments());
            options.addAll(launch.options);
            Path launcher = session.directory.resolve("launcher.jar");
            try (FileChannel input = FileChannel.open(snapshot);
                 OutputStream output = Files.newOutputStream(launcher)) {
                if (tailOffset == input.size()) {
                    throw new IOException("Standalone launching requires an executable JAR tail");
                }
                input.position(tailOffset);
                transfer(Channels.newInputStream(input), output, 512L * 1024 * 1024);
            }
            Path bridge = session.directory.resolve("bootstrap.jar");
            writeBridge(launcher, bridge, launch, resourceIndex, options, feature);
            List<String> nativeOptions = nativeOptions(options, feature);
            String java = Paths.get(System.getProperty("java.home"), "bin", isWindows() ? "java.exe" : "java").toString();
            if (feature >= 9) {
                validateModules(session, java, bridge, nativeOptions);
            }
            List<String> agents = prepareAgents(session.directory, launch, agentIndex);
            List<String> command = new ArrayList<String>();
            command.add(java);
            command.addAll(nativeOptions);
            command.addAll(agents);
            if (feature >= 9) {
                command.add("--add-modules=ALL-SYSTEM");
            }
            command.add("-Djava.system.class.loader=org.janex.bootstrap.loader.ResourceLoader");
            command.add("-cp");
            command.add(bridge.toString());
            command.add("org.janex.bootstrap.Bootstrap");
            session.process = process(command).inheritIO().start();
            exit = session.process.waitFor();
        }
        return exit;
    }

    /// Creates a child builder without re-expanding options already consumed by the initial JVM.
    private static ProcessBuilder process(List<String> command) {
        ProcessBuilder builder = new ProcessBuilder(command);
        builder.environment().remove("JDK_JAVA_OPTIONS");
        builder.environment().remove("JAVA_TOOL_OPTIONS");
        builder.environment().remove("_JAVA_OPTIONS");
        return builder;
    }

    /// Checks indexed module resolution in a child without application entry points or agents.
    private static void validateModules(Session session, String java, Path bridge, List<String> options)
            throws IOException, InterruptedException {
        List<String> command = new ArrayList<String>();
        command.add(java);
        command.add("--add-exports=java.base/jdk.internal.module=ALL-UNNAMED");
        command.add("--add-modules=ALL-SYSTEM");
        for (String option : options) {
            if (option.equals("--enable-preview") || option.startsWith("--enable-native-access=")
                    || option.equals("--add-opens=java.base/java.lang=ALL-UNNAMED")) {
                command.add(option);
            }
        }
        command.add("-cp");
        command.add(bridge.toString());
        command.add("org.janex.bootstrap.loader.ModuleSupport");
        Path diagnostics = session.directory.resolve("module-check.txt");
        session.process = process(command).redirectErrorStream(true).redirectOutput(diagnostics.toFile()).start();
        session.process.getOutputStream().close();
        if (session.process.waitFor() != 0) {
            ByteArrayOutputStream output = new ByteArrayOutputStream();
            try (InputStream input = Files.newInputStream(diagnostics)) {
                transfer(input, output, 1024 * 1024);
            }
            throw new IOException("Java module validation failed: " + output.toString("UTF-8"));
        }
        Files.delete(diagnostics);
    }

    /// Returns whether this process runs on Windows.
    private static boolean isWindows() {
        return System.getProperty("os.name").startsWith("Windows");
    }

    /// Materializes selected agent roots before any descriptor-supplied agent runs.
    private static List<String> prepareAgents(Path directory, JanexReader.Launch launch, byte[] agentIndex) throws IOException {
        List<String> result = new ArrayList<String>();
        if (launch.agentResources == null) {
            return result;
        }
        try (ResourceIndex index = new ResourceIndex(new ByteArrayInputStream(agentIndex))) {
            if (index.roots().size() != launch.agentOptions.size() || index.roots().size() != launch.agentChecksums.size()) {
                throw new IOException("Java agent roots and options disagree");
            }
            for (int i = 0; i < index.roots().size(); i++) {
                Path path = directory.resolve("agent-" + i + ".jar");
                if (path.toString().indexOf('=') >= 0) {
                    throw new IOException("Java agent path contains an unrepresentable equals sign");
                }
                try (JarOutputStream output = new JarOutputStream(Files.newOutputStream(path))) {
                    for (Map.Entry<String, ResourceIndex.Resource> entry : index.roots().get(i).files().entrySet()) {
                        if (entry.getKey().isEmpty()) {
                            continue;
                        }
                        ResourceIndex.Resource resource = entry.getValue();
                        JarEntry member = new JarEntry(entry.getKey() + (resource.isDirectory() ? "/" : ""));
                        member.setTime(0);
                        output.putNextEntry(member);
                        if (!resource.isDirectory()) {
                            byte[] bytes = resource.readBytes();
                            byte[] checksum = launch.agentChecksums.get(i).get(entry.getKey());
                            if (checksum != null) {
                                Checksum.decode(checksum).verify(bytes);
                            }
                            if (entry.getKey().equalsIgnoreCase("META-INF/MANIFEST.MF")) {
                                bytes = JanexReader.runtimeManifest(bytes);
                            }
                            output.write(bytes);
                        }
                        output.closeEntry();
                    }
                }
                String option = launch.agentOptions.get(i);
                result.add("-javaagent:" + path + (option.isEmpty() ? "" : "=" + option));
            }
        }
        return result;
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

    /// Copies the extracted launcher JAR and embeds the selected resource index and launch data.
    private static void writeBridge(Path launcher, Path output, JanexReader.Launch launch,
                                    byte[] resourceIndex, List<String> options, int feature) throws IOException {
        try (JarFile source = new JarFile(launcher.toFile());
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
            jar.write(resourceIndex);
            jar.closeEntry();
            DataOutputStream data = new DataOutputStream(jar);
            jar.putNextEntry(new JarEntry("org/janex/bootstrap/launch.bin"));
            ResourceIndexes.string(data, launch.mainModule);
            ResourceIndexes.string(data, launch.mainClass);
            data.writeBoolean(feature >= 25 || (feature >= 21 && options.contains("--enable-preview")));
            data.writeInt(launch.arguments.size());
            for (String argument : launch.arguments) {
                ResourceIndexes.string(data, argument);
            }
            jar.closeEntry();
            jar.putNextEntry(new JarEntry("org/janex/bootstrap/options.bin"));
            ResourceIndexes.string(data, launch.mainModule);
            ResourceIndexes.string(data, launch.mainClass);
            data.writeInt(options.size());
            for (String option : options) {
                ResourceIndexes.string(data, option);
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

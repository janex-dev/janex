// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap;

import java.io.*;
import java.lang.management.ManagementFactory;
import java.nio.channels.Channels;
import java.nio.channels.FileChannel;
import java.nio.file.*;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.*;
import java.util.jar.*;

import org.glavo.janex.bootstrap.dependency.Dependencies;
import org.glavo.janex.bootstrap.loader.ResourceIndex;
import org.glavo.janex.bootstrap.loader.ResourceIndexes;
import org.glavo.janex.bootstrap.loader.ResourceHandoff;
import org.glavo.janex.reader.ResourcePlan;
import org.glavo.janex.reader.Checksum;
import org.glavo.janex.reader.JanexReader;

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
    /// The package and dependency cache files are read directly and must remain unchanged until
    /// the child exits. Temporary agent files are removed on exit or failure. Integrity is verified without
    /// establishing publisher trust. This method does not terminate the calling JVM.
    ///
    /// @param executable Janex file with an ordinary or ZIP64 executable JAR tail
    /// @param arguments arguments appended after the selected package's preset arguments
    /// @return the child process's exit status
    /// @throws Exception if preparation, process creation, waiting, or cleanup fails
    public static int launch(Path executable, String[] arguments) throws Exception {
        int exit;
        try (Session session = new Session()) {
            Path source = executable.toRealPath();
            if (Files.size(source) > 512L * 1024 * 1024) {
                throw new IOException("Standalone launch byte limit exceeded");
            }
            JanexReader.Launch launch;
            long tailOffset;
            try (JanexReader reader = new JanexReader(source, new Dependencies())) {
                launch = reader.prepareHandoff(System.getProperty("janex.application"));
                tailOffset = reader.externalTailOffset();
            }
            String os = System.getProperty("os.name");
            os = os.startsWith("Windows") ? "windows" : os.equals("Mac OS X") ? "macos"
                    : os.equals("Linux") ? "linux" : os.equals("FreeBSD") ? "freebsd" : os;
            String arch = System.getProperty("os.arch");
            arch = arch.equals("amd64") || arch.equals("x86_64") ? "x86-64"
                    : arch.matches("i[3-6]86") ? "x86" : arch.equals("arm64") ? "aarch64" : arch;
            byte[] resources = ResourceHandoff.encode(source,
                    new String[]{os, arch, "run", System.getProperty("java.version"), System.getProperty("java.vendor")},
                    launch.requests, launch.moduleRequirements, launch.resources.limits(), launch.resourceAllowance);
            launch.arguments.addAll(Arrays.asList(arguments));
            int feature = JanexReader.feature();
            if (feature < 9 && !launch.mainModule.isEmpty()) {
                throw new IOException("Module launching requires Java 9 or later");
            }
            List<String> options = new ArrayList<String>(ManagementFactory.getRuntimeMXBean().getInputArguments());
            options.addAll(launch.options);
            Path launcher = cacheLauncher(source, tailOffset);
            String description = launchArgument(launch, resources, options, feature);
            List<String> nativeOptions = nativeOptions(options, feature);
            String java = Paths.get(System.getProperty("java.home"), "bin", isWindows() ? "java.exe" : "java").toString();
            if (feature >= 9) {
                boolean modules = !launch.resources.requirements().isEmpty();
                for (org.glavo.janex.reader.ResourcePlan.Root root : launch.resources.roots()) {
                    modules |= root.module();
                }
                String[] systemModules;
                try {
                    systemModules = (String[]) Class.forName("org.glavo.janex.bootstrap.loader.ModuleSupport")
                            .getMethod("launchModules", ResourcePlan.class, String.class, List.class)
                            .invoke(null, modules ? launch.resources : null, launch.mainModule, options);
                } catch (java.lang.reflect.InvocationTargetException failure) {
                    throw new IOException("Cannot prepare Java modules", failure.getCause());
                }
                if (systemModules.length != 0) {
                    nativeOptions.add("--add-modules=" + String.join(",", systemModules));
                }
            }
            List<String> agents = launch.agentResources == null ? Collections.emptyList()
                    : prepareAgents(session.directory(), launch);
            List<String> command = new ArrayList<String>();
            command.add(java);
            command.addAll(nativeOptions);
            command.addAll(agents);
            command.add("-Djava.system.class.loader=org.glavo.janex.bootstrap.loader.ResourceLoader");
            command.add(description);
            command.add("-cp");
            command.add(launcher.toString());
            command.add("org.glavo.janex.bootstrap.Bootstrap");
            session.process = process(command).inheritIO().start();
            exit = session.process.waitFor();
        }
        return exit;
    }

    /// Creates a child builder without re-expanding options already consumed by the initial JVM.
    private static ProcessBuilder process(List<String> command) {
        ProcessBuilder builder = new ProcessBuilder(command);
        for (int i = command.size() - 1; i >= 0; i--) {
            String argument = command.get(i);
            if (!argument.startsWith("-Djanex.launch=")) continue;
            if (argument.length() > 8015) {
                String value = argument.substring(15);
                int count = 0;
                for (int offset = 0; offset < value.length(); offset += 8000) {
                    builder.environment().put("JANEX_LAUNCH_" + count++, value.substring(offset, Math.min(offset + 8000, value.length())));
                }
                command.set(i, "-Djanex.launch=env:" + count);
            }
            break;
        }
        builder.environment().remove("JDK_JAVA_OPTIONS");
        builder.environment().remove("JAVA_TOOL_OPTIONS");
        builder.environment().remove("_JAVA_OPTIONS");
        return builder;
    }

    /// Returns whether this process runs on Windows.
    private static boolean isWindows() {
        return System.getProperty("os.name").startsWith("Windows");
    }

    /// Materializes selected agent roots before any descriptor-supplied agent runs.
    private static List<String> prepareAgents(Path directory, JanexReader.Launch launch) throws IOException {
        List<String> result = new ArrayList<String>();
        if (launch.agentResources == null) {
            return result;
        }
        try (ResourceIndex index = new ResourceIndex(launch.agentResources)) {
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
                        JarEntry member = new JarEntry(entry.getKey() + (resource.isDirectory() && !entry.getKey().endsWith("/") ? "/" : ""));
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

    /// Encodes entry data and options without rebuilding the fixed launcher JAR.
    private static String launchArgument(JanexReader.Launch launch, byte[] resources,
                                         List<String> options, int feature) throws IOException {
        ByteArrayOutputStream entry = new ByteArrayOutputStream();
        DataOutputStream data = new DataOutputStream(entry);
        ResourceIndexes.string(data, launch.mainModule);
        ResourceIndexes.string(data, launch.mainClass);
        data.writeBoolean(feature >= 25 || (feature >= 21 && options.contains("--enable-preview")));
        data.writeInt(launch.arguments.size());
        for (String argument : launch.arguments) ResourceIndexes.string(data, argument);
        ByteArrayOutputStream optionBytes = new ByteArrayOutputStream();
        data = new DataOutputStream(optionBytes);
        ResourceIndexes.string(data, launch.mainModule);
        ResourceIndexes.string(data, launch.mainClass);
        data.writeInt(options.size());
        for (String option : options) ResourceIndexes.string(data, option);
        return LaunchData.encode(entry.toByteArray(), optionBytes.toByteArray(), resources);
    }

    /// Publishes the unchanged executable tail under its content digest, repairing cache damage.
    private static Path cacheLauncher(Path snapshot, long offset) throws IOException {
        byte[] bytes;
        try (FileChannel input = FileChannel.open(snapshot);
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            if (offset == input.size()) throw new IOException("Standalone launching requires an executable JAR tail");
            input.position(offset);
            transfer(Channels.newInputStream(input), output, 512L * 1024 * 1024);
            bytes = output.toByteArray();
        }
        String digest;
        try {
            byte[] hash = java.security.MessageDigest.getInstance("SHA-256").digest(bytes);
            StringBuilder text = new StringBuilder();
            for (byte value : hash) text.append(String.format(java.util.Locale.ROOT, "%02x", value & 255));
            digest = text.toString();
        } catch (java.security.NoSuchAlgorithmException unavailable) {
            throw new IOException("SHA-256 is unavailable", unavailable);
        }
        String home = System.getenv("JANEX_HOME");
        boolean overridden = home != null;
        if (!overridden) home = System.getenv(isWindows() ? "USERPROFILE" : "HOME");
        if (home == null) throw new IOException("Cannot locate user home; set JANEX_HOME");
        Path base = Paths.get(home);
        if (home.isEmpty() || !base.isAbsolute()) throw new IOException("JANEX_HOME must be a nonempty absolute path");
        if (!overridden) base = base.resolve(".janex");
        Path directory = base.resolve("cache/bootstrap").resolve(digest);
        Files.createDirectories(directory);
        Path jar = directory.resolve("bootstrap.jar");
        if (!Files.exists(jar) || !Arrays.equals(Files.readAllBytes(jar), bytes)) {
            Path temporary = Files.createTempFile(directory, "bootstrap-", ".tmp");
            try {
                Files.write(temporary, bytes);
                try {
                    Files.move(temporary, jar, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
                } catch (AtomicMoveNotSupportedException unsupported) {
                    Files.move(temporary, jar, StandardCopyOption.REPLACE_EXISTING);
                } catch (FileSystemException concurrent) {
                    if (!Files.exists(jar) || !Arrays.equals(Files.readAllBytes(jar), bytes)) throw concurrent;
                }
            } finally {
                Files.deleteIfExists(temporary);
            }
        }
        return jar;
    }

    /// Owns launch files and ensures a child does not outlive parent shutdown.
    private static final class Session implements AutoCloseable {
        /// Private agent directory, or null when no agent files have been materialized.
        private Path directory;
        /// Shutdown callback retained until normal closure.
        final Thread hook;
        /// Child JVM, or null until process creation succeeds.
        volatile Process process;

        /// Registers process cleanup without creating launch files.
        Session() throws IOException {
            hook = new Thread(() -> {
                try {
                    cleanup();
                } catch (Exception ignored) {
                    // Process shutdown cannot report cleanup failures to the caller.
                }
            }, "janex-java-cleanup");
            Runtime.getRuntime().addShutdownHook(hook);
        }

        /// Creates the private directory only when materializing agents.
        synchronized Path directory() throws IOException {
            if (directory == null) directory = Files.createTempDirectory("janex-java-");
            return directory;
        }

        /// Terminates any remaining child before removing its private launch files.
        private synchronized void cleanup() throws IOException, InterruptedException {
            if (process != null && process.isAlive()) {
                process.destroy();
                if (!process.waitFor(5, java.util.concurrent.TimeUnit.SECONDS)) {
                    process.destroyForcibly().waitFor();
                }
            }
            if (directory != null && Files.exists(directory)) {
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

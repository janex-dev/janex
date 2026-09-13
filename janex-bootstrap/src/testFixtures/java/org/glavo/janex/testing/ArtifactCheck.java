// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.testing;

import java.io.DataInputStream;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.TimeUnit;
import java.util.stream.Stream;

/// Checks distribution binaries and their supported Java launch modes.
public final class ArtifactCheck {
    /// Prevents instantiation.
    private ArtifactCheck() {
    }

    /// Verifies executable structure, dependencies, resources, and program arguments.
    ///
    /// @param args target triple and directory containing the release binaries
    /// @throws Exception if inspection or a launch check fails
    public static void main(String[] args) throws Exception {
        String target = args[0];
        Path directory = Paths.get(args[1]).toAbsolutePath();
        boolean windows = target.endsWith("-windows-msvc");
        boolean macos = target.endsWith("-apple-darwin");
        String suffix = windows ? ".exe" : "";
        Path cli = directory.resolve("janex" + suffix);
        Path launcher = macos ? null : directory.resolve("janex-launcher" + suffix);
        inspect(target, cli);
        if (launcher != null) {
            inspect(target, launcher);
        }
        List<String> runner = new ArrayList<>();
        if (target.equals("aarch64-unknown-linux-musl")) {
            runner.add("qemu-aarch64");
        }
        Path java = Paths.get(System.getProperty("java.home"), "bin", "java" + suffix);
        Path temporary = Files.createTempDirectory("janex-artifacts-");
        try {
            Path classes = Files.createDirectory(temporary.resolve("classes"));
            String name = Application.class.getName();
            String resource = name.replace('.', '/') + ".class";
            Path compiled = classes.resolve(resource);
            Files.createDirectories(compiled.getParent());
            try (InputStream input = ArtifactCheck.class.getResourceAsStream("/" + resource)) {
                Files.copy(input, compiled);
            }
            Files.write(classes.resolve("message.txt"), "resource:".getBytes(StandardCharsets.UTF_8));
            execute(runner, cli, "--version");
            for (String mode : Arrays.asList("bootstrap", "direct")) {
                String argument = windows && mode.equals("direct") ? "ASCII argument" : "Unicode 参数 🚀";
                Path application = temporary.resolve("application-" + mode + suffix);
                List<String> pack = new ArrayList<>(Arrays.asList("pack", classes.toString(),
                        "--output", application.toString(), "--main-class", name, "--with-launcher"));
                if (launcher != null) {
                    pack.addAll(Arrays.asList("--native-launcher", launcher.toString(), "--native-launch-mode", mode));
                }
                execute(runner, cli, pack.toArray());
                expect(argument, execute(runner, cli, "run", "--allow-unsigned", "--java", java,
                        "--launch-mode", mode, application, argument));
                if (launcher != null) {
                    expect(argument, execute(runner, application, argument));
                }
                String standaloneArgument = windows ? "ASCII argument" : argument;
                expect(standaloneArgument, run(java, "-jar", application, standaloneArgument));
            }
        } finally {
            try (Stream<Path> paths = Files.walk(temporary)) {
                for (Path path : (Iterable<Path>) paths.sorted(Comparator.reverseOrder())::iterator) {
                    Files.delete(path);
                }
            }
        }
        System.out.println("Verified binary format, dependencies, and launch modes: " + target);
    }

    /// Checks target architecture and platform-specific runtime dependencies.
    private static void inspect(String target, Path binary) throws Exception {
        if (target.endsWith("-windows-msvc")) {
            ByteBuffer data = ByteBuffer.wrap(Files.readAllBytes(binary)).order(ByteOrder.LITTLE_ENDIAN);
            int offset = data.getInt(0x3C);
            int machine = target.startsWith("i686-") ? 0x14C : target.startsWith("aarch64-") ? 0xAA64 : 0x8664;
            if (data.getShort(0) != 0x5A4D || data.getInt(offset) != 0x4550
                    || (data.getShort(offset + 4) & 0xFFFF) != machine) {
                throw new AssertionError("Unexpected PE architecture: " + binary);
            }
            String host = System.getProperty("os.arch").matches("aarch64|arm64") ? "arm64" : "x64";
            Path vswhere = Paths.get(System.getenv("ProgramFiles(x86)"),
                    "Microsoft Visual Studio", "Installer", "vswhere.exe");
            String tools = run(vswhere, "-latest", "-products", "*", "-find",
                    "VC\\Tools\\MSVC\\**\\bin\\Host" + host + "\\" + host + "\\dumpbin.exe").trim();
            if (tools.isEmpty()) {
                throw new AssertionError("Visual Studio dumpbin not found");
            }
            String imports = run(tools.split("\\R")[0], "/dependents", binary).toLowerCase(Locale.ROOT);
            if (imports.matches("(?s).*(vcruntime|msvcp|ucrtbase|api-ms-win-crt).*")) {
                throw new AssertionError("Dynamic MSVC runtime dependency: " + binary);
            }
        } else if (target.endsWith("-apple-darwin")) {
            String machine = target.startsWith("aarch64-") ? "arm64" : "x86_64";
            if (!run("lipo", "-archs", binary).trim().equals(machine)) {
                throw new AssertionError("Unexpected Mach-O architecture: " + binary);
            }
            String[] lines = run("otool", "-L", binary).split("\\R");
            for (int i = 1; i < lines.length; i++) {
                String dependency = lines[i].trim();
                if (!dependency.startsWith("/usr/lib/") && !dependency.startsWith("/System/Library/")) {
                    throw new AssertionError("Non-system library dependency: " + dependency);
                }
            }
        } else {
            String machine = target.startsWith("aarch64-") ? "AArch64" : "Advanced Micro Devices X86-64";
            if (!run("readelf", "-hW", binary).contains(machine)
                    || run("readelf", "-lW", binary).contains("INTERP")
                    || run("readelf", "-dW", binary).contains("(NEEDED)")) {
                throw new AssertionError("Unexpected ELF architecture or dynamic dependency: " + binary);
            }
            Map<String, String> symbols = new HashMap<>();
            for (String line : run("nm", "--defined-only", binary).split("\\R")) {
                String[] fields = line.trim().split("\\s+");
                if (fields.length == 3) {
                    symbols.put(fields[2], fields[0]);
                }
            }
            for (String name : Arrays.asList("malloc", "calloc", "realloc", "free")) {
                if (!symbols.containsKey(name) || !symbols.get(name).equals(symbols.get("mi_" + name))) {
                    throw new AssertionError("mimalloc does not override " + name + ": " + binary);
                }
            }
        }
    }

    /// Checks the resource contents and argument emitted by the test application.
    private static void expect(String argument, String actual) {
        if (!actual.equals("resource:" + argument)) {
            throw new AssertionError("Unexpected application output: " + actual);
        }
    }

    /// Runs an executable through the optional architecture emulator.
    private static String execute(List<String> runner, Path executable, Object... arguments) throws Exception {
        List<Object> command = new ArrayList<>(runner);
        command.add(executable);
        command.addAll(Arrays.asList(arguments));
        return run(command.toArray());
    }

    /// Captures UTF-8 output and requires successful completion within two minutes.
    private static String run(Object... arguments) throws Exception {
        List<String> command = new ArrayList<>();
        for (Object argument : arguments) {
            command.add(argument.toString());
        }
        Path output = Files.createTempFile("janex-artifact-command-", ".log");
        try {
            ProcessBuilder builder = new ProcessBuilder(command);
            String suffix = System.getProperty("os.name").startsWith("Windows") ? ".exe" : "";
            builder.environment().put("JANEX_JAVA", Paths.get(System.getProperty("java.home"), "bin", "java" + suffix).toString());
            Process process = builder.redirectOutput(output.toFile()).redirectError(ProcessBuilder.Redirect.INHERIT).start();
            try {
                if (!process.waitFor(120, TimeUnit.SECONDS)) {
                    throw new AssertionError("Command timed out: " + command);
                }
                if (process.exitValue() != 0) {
                    throw new AssertionError("Command failed: " + command);
                }
                return new String(Files.readAllBytes(output), StandardCharsets.UTF_8);
            } finally {
                if (process.isAlive()) {
                    process.destroyForcibly().waitFor();
                }
            }
        } finally {
            Files.delete(output);
        }
    }

    /// Prints a packaged resource and the first program argument in UTF-8.
    public static final class Application {
        /// Prevents instantiation.
        private Application() {
        }

        /// Reads the resource completely before emitting the output.
        ///
        /// @param args program arguments, with at least one element
        /// @throws Exception if the resource is missing or cannot be read
        public static void main(String[] args) throws Exception {
            try (DataInputStream input = new DataInputStream(Application.class.getResourceAsStream("/message.txt"))) {
                byte[] bytes = new byte[9];
                input.readFully(bytes);
                System.out.write((new String(bytes, StandardCharsets.UTF_8) + args[0]).getBytes(StandardCharsets.UTF_8));
            }
        }
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

/// Compares selected entry points and ordered JVM/program arguments with native evaluation.
public final class LaunchSelectionTest {
    /// Prevents instantiation.
    private LaunchSelectionTest() {
    }

    /// Checks overlay pruning, replacement, list clearing, and pending-work limits.
    public static void main(String[] arguments) throws Exception {
        System.setProperty("os.name", "Windows 11");
        System.setProperty("os.arch", "amd64");
        System.setProperty("java.version", "25");
        System.setProperty("java.vendor", "Vendor");
        Path snapshot = Paths.get(arguments[0]).resolveSibling("selection.janex");
        try (DataInputStream input = new DataInputStream(new BufferedInputStream(new FileInputStream(arguments[0])))) {
            int count = input.readInt();
            for (int index = 0; index < count; index++) {
                ReadLimits limits = new ReadLimits(input.readInt(), input.readInt(), input.readInt());
                Files.write(snapshot, bytes(input));
                boolean valid = input.readBoolean();
                String mainClass = "";
                String mainModule = "";
                List<String> options = null;
                List<String> preset = null;
                if (valid) {
                    mainClass = text(input);
                    mainModule = text(input);
                    options = strings(input);
                    preset = strings(input);
                }
                try (JanexReader reader = new JanexReader(snapshot, (data, length, dictionary) -> {
                    throw new AssertionError("Unexpected compressed data");
                }, null, null, limits)) {
                    JanexReader.Launch launch = reader.launch("main");
                    if (!valid || !mainClass.equals(launch.mainClass) || !mainModule.equals(launch.mainModule)
                            || !options.equals(launch.options) || !preset.equals(launch.arguments)) {
                        throw new AssertionError("Launch selection mismatch at " + index + ": " + launch.options + "/" + launch.arguments);
                    }
                } catch (IOException failure) {
                    if (valid) {
                        throw new AssertionError("Valid launch selection rejected at " + index, failure);
                    }
                }
            }
            if (input.read() != -1) {
                throw new AssertionError("Trailing launch-selection vectors");
            }
        }
        Files.delete(snapshot);
    }

    /// Reads one trusted harness byte sequence.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] value = new byte[input.readInt()];
        input.readFully(value);
        return value;
    }

    /// Reads trusted UTF-8 text.
    private static String text(DataInputStream input) throws IOException {
        return new String(bytes(input), StandardCharsets.UTF_8);
    }

    /// Reads an ordered trusted text list.
    private static List<String> strings(DataInputStream input) throws IOException {
        int count = input.readInt();
        List<String> values = new ArrayList<String>();
        for (int index = 0; index < count; index++) {
            values.add(text(input));
        }
        return values;
    }
}

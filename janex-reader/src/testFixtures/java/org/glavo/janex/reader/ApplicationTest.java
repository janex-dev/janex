// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.*;
import java.nio.file.*;
import java.util.Arrays;

import org.glavo.janex.reader.internal.Input;

/// Checks application schemas and locale lookup against native format results.
public final class ApplicationTest {
    /// Prevents instantiation.
    private ApplicationTest() {
    }

    /// Reads an owned byte sequence from a trusted oracle stream.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] value = new byte[input.readInt()];
        input.readFully(value);
        return value;
    }

    /// Compares complete descriptor validation, original bytes, and localized presentation.
    ///
    /// @param args oracle stream path
    /// @throws Exception if fixture I/O fails or a native/Java comparison differs
    public static void main(String[] args) throws Exception {
        String version = System.getProperty("java.version");
        System.setProperty("java.version", "must-not-be-consulted-by-descriptor-parsing");
        try (DataInputStream input = new DataInputStream(Files.newInputStream(Paths.get(args[0])))) {
            int count = input.readInt();
            for (int i = 0; i < count; i++) {
                byte[] encoded = bytes(input);
                byte[] info = bytes(input);
                boolean valid = input.readBoolean();
                Application application;
                try {
                    application = new Application(encoded, info);
                } catch (IOException failure) {
                    if (valid) {
                        throw new AssertionError("Valid application rejected at " + i, failure);
                    }
                    continue;
                }
                check(valid, "Malformed application accepted at " + i);
                check(application.id().equals(Input.utf8(bytes(input))), "Application ID");
                check(application.type().equals(Input.utf8(bytes(input))), "Application type");
                check(application.windowed() == input.readBoolean(), "Application launch mode");
                check(Arrays.equals(application.encoded(), encoded) && Arrays.equals(application.typeInfo(), info), "Original application bytes");
                Arrays.fill(application.encoded(), (byte) 0);
                Arrays.fill(application.typeInfo(), (byte) 0);
                check(Arrays.equals(application.encoded(), encoded) && Arrays.equals(application.typeInfo(), info), "Aliased application metadata");
                int locales = input.readInt();
                for (int j = 0; j < locales; j++) {
                    String locale = Input.utf8(bytes(input));
                    String title = Input.utf8(bytes(input));
                    String comment = input.readBoolean() ? Input.utf8(bytes(input)) : null;
                    check(application.title(locale).equals(title), "Title lookup at " + i + ": " + locale);
                    check(java.util.Objects.equals(application.comment(locale), comment), "Comment lookup at " + i + ": " + locale);
                }
            }
            check(input.read() == -1, "Trailing application fixtures");
            System.out.println("Verified " + count + " application vectors");
        } finally {
            System.setProperty("java.version", version);
        }
    }

    /// Requires a test invariant.
    private static void check(boolean condition, String message) {
        if (!condition) {
            throw new AssertionError(message);
        }
    }
}

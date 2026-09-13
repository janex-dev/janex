// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.*;
import java.nio.file.*;

import org.glavo.janex.reader.internal.Input;
import org.glavo.janex.reader.internal.ModuleRequirement;

/// Compares decoded Janex module requirements with the native format parser.
public final class ModuleRequirementTest {
    /// Prevents instantiation.
    private ModuleRequirementTest() {
    }

    /// Reads a counted UTF-8 string from a trusted oracle stream.
    private static String text(DataInputStream input) throws IOException {
        byte[] bytes = new byte[input.readInt()];
        input.readFully(bytes);
        return Input.utf8(bytes);
    }

    /// Checks acceptance, exact decoded names and versions, and path placement.
    ///
    /// @param args oracle stream path
    /// @throws Exception if fixture I/O fails or Java disagrees with Rust
    public static void main(String[] args) throws Exception {
        try (DataInputStream input = new DataInputStream(Files.newInputStream(Paths.get(args[0])))) {
            int count = input.readInt();
            for (int i = 0; i < count; i++) {
                String uri = text(input);
                boolean module = input.readBoolean();
                boolean valid = input.readBoolean();
                String name = valid ? text(input) : null;
                String version = valid ? text(input) : null;
                try {
                    ModuleRequirement requirement = ModuleRequirement.parse(uri, module);
                    if (!valid || requirement == null || !requirement.name().equals(name) || !requirement.version().equals(version)) {
                        throw new AssertionError("Module requirement mismatch at " + i + ": " + uri);
                    }
                } catch (IOException failure) {
                    if (valid) {
                        throw new AssertionError("Valid module requirement rejected at " + i + ": " + uri, failure);
                    }
                }
            }
            if (input.read() != -1) {
                throw new AssertionError("Trailing module requirement vectors");
            }
            System.out.println("Verified " + count + " module requirement vectors");
        }
    }
}

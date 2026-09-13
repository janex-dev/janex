// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.DataInputStream;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Arrays;
import java.util.List;

/// Compares ZIP import and Janex tail discovery with independently constructed Rust fixtures.
public final class ArchiveTest {
    /// Prevents instantiation.
    private ArchiveTest() {
    }

    /// Reads one byte string from a trusted fixture stream.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] bytes = new byte[input.readInt()];
        input.readFully(bytes);
        return bytes;
    }

    /// Runs import or framing checks without invoking application code.
    ///
    /// @param args fixture-stream path and reusable snapshot path
    /// @throws Exception if fixture I/O fails or Java disagrees with the native result
    public static void main(String[] args) throws Exception {
        Path snapshot = Paths.get(args[1]);
        try (DataInputStream input = new DataInputStream(Files.newInputStream(Paths.get(args[0])))) {
            int count = input.readInt();
            for (int i = 0; i < count; i++) {
                boolean archive = input.readBoolean();
                boolean valid = input.readBoolean();
                byte[] encoded = bytes(input);
                List<JarArchive.Entry> entries = null;
                try {
                    if (archive) {
                        entries = JarArchive.read(encoded);
                    } else {
                        Files.write(snapshot, encoded);
                        try (ContainerReader reader = new ContainerReader(snapshot)) {
                            // Boundary discovery checks framing without conflating it with integrity.
                        }
                    }
                    if (!valid) {
                        throw new AssertionError("Malformed archive vector accepted: " + i);
                    }
                } catch (IOException failure) {
                    if (valid) {
                        throw new AssertionError("Valid archive vector rejected: " + i, failure);
                    }
                }
                if (archive && valid) {
                    int expected = input.readInt();
                    if (entries.size() != expected) {
                        throw new AssertionError("Entry count mismatch: " + i);
                    }
                    for (JarArchive.Entry entry : entries) {
                        if (!entry.name.equals(Input.utf8(bytes(input))) || !Arrays.equals(entry.bytes, bytes(input))) {
                            throw new AssertionError("Entry content mismatch: " + i);
                        }
                        if (entry.mode != input.readInt()) {
                            throw new AssertionError("Entry mode mismatch: " + i);
                        }
                    }
                }
            }
            if (input.read() != -1) {
                throw new AssertionError("Trailing fixture bytes");
            }
            System.out.println("Verified " + count + " archive vectors");
        }
    }
}

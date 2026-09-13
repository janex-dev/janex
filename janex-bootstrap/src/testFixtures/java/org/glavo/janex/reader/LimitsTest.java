// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.*;
import java.nio.file.*;

import org.glavo.janex.reader.internal.Input;
import org.glavo.janex.reader.internal.codec.ZstandardFrames;
import org.glavo.janex.reader.internal.codec.zstd.Zstandard;

/// Compares bounded Java decoding and container operations with independent native results.
public final class LimitsTest {
    /// Prevents instantiation.
    private LimitsTest() {
    }

    /// Runs trusted vectors covering exact bounds, inherited limits, and distinct failure categories.
    public static void main(String[] arguments) throws Exception {
        Path directory = Paths.get(arguments[0]).getParent();
        Path snapshot = directory.resolve("limits.janex");
        try (DataInputStream input = new DataInputStream(new FileInputStream(arguments[0]))) {
            int count = input.readInt();
            for (int index = 0; index < count; index++) {
                int mode = input.readUnsignedByte();
                ReadLimits limits = new ReadLimits(input.readInt(), input.readInt(), input.readInt());
                byte[] encoded = bytes(input);
                byte[] info = bytes(input);
                int expected = input.readUnsignedByte();
                int actual = 1;
                Exception failure = null;
                try {
                    if (mode == 0) {
                        Input value = new Input(encoded, limits);
                        value.cbor(0);
                        value.end();
                    } else if (mode == 1) {
                        new Application(encoded, info, limits);
                    } else if (mode == 6) {
                        ZstandardFrames.validate(encoded, limits);
                        byte[] decoded = new byte[info[0]];
                        try {
                            if (Zstandard.decompress(encoded, 0, encoded.length, decoded, 0, decoded.length) != decoded.length) {
                                throw new IOException("Decoded length mismatch");
                            }
                        } catch (IllegalArgumentException invalid) {
                            throw new IOException("Invalid Zstandard input", invalid);
                        }
                    } else {
                        Files.write(snapshot, encoded);
                        if (mode >= 4) {
                            try (JanexReader reader = new JanexReader(snapshot, (data, length, dictionary) -> {
                                if (length > limits.maxBytes()) {
                                    throw new AssertionError("Decoder invoked before output bound check");
                                }
                                byte[] output = new byte[length];
                                int written = Zstandard.decompress(data, 0, data.length, output, 0, length, dictionary);
                                if (written != length) {
                                    throw new AssertionError("Decoded length mismatch");
                                }
                                return output;
                            }, null, null, limits)) {
                                if (reader.limits() != limits) {
                                    throw new AssertionError("Lost reader policy");
                                }
                                if (mode == 5) {
                                    byte[] resources = org.glavo.janex.bootstrap.loader.ResourceIndexes.encode(reader.launch("main").resources);
                                    try (DataInputStream indexInput = new DataInputStream(new ByteArrayInputStream(resources))) {
                                        indexInput.readLong();
                                        if (indexInput.readInt() != limits.maxBytes() || indexInput.readInt() != limits.maxElements()) {
                                            throw new AssertionError("Private index lost policy");
                                        }
                                    }
                                }
                            }
                        } else {
                            try (ContainerReader reader = new ContainerReader(snapshot, mode == 3 ? -1 : 0, limits)) {
                                if (reader.limits() != limits) {
                                    throw new AssertionError("Lost container policy");
                                }
                                reader.verifyChecksums();
                                // Streaming a large section is independent of the buffered byte limit.
                                if (info.length != 0) {
                                    for (ContainerReader.Section section : reader.sections()) {
                                        reader.readSection(section.id());
                                    }
                                }
                            }
                        }
                    }
                } catch (Input.Invalid invalid) {
                    actual = 0;
                    failure = invalid;
                } catch (IOException invalid) {
                    actual = invalid.getMessage().contains("limit") ? 2 : 0;
                    failure = invalid;
                }
                if (actual != expected && (mode != 6 || actual == 1 || expected == 1)) {
                    throw new AssertionError("Vector " + index + " mode " + mode + " limits "
                            + limits.maxBytes() + "/" + limits.maxElements() + "/" + limits.maxDepth()
                            + ": expected " + expected + ", got " + actual, failure);
                }
            }
            if (input.read() != -1) {
                throw new AssertionError("Trailing oracle bytes");
            }
        }
        Files.deleteIfExists(snapshot);
        for (int field = 0; field < 3; field++) {
            try {
                new ReadLimits(field == 0 ? -1 : 0, field == 1 ? -1 : 0, field == 2 ? -1 : 0);
                throw new AssertionError("Negative limit accepted");
            } catch (IllegalArgumentException expected) {
                // Each policy field rejects negative bounds independently.
            }
        }
    }

    /// Reads one trusted harness byte sequence, independently of the policy being tested.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] bytes = new byte[input.readInt()];
        input.readFully(bytes);
        return bytes;
    }
}

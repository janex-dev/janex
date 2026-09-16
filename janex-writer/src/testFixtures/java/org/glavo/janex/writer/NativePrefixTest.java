// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;

/// Checks native-prefix memory use and bounds in a JVM with a limited heap.
public final class NativePrefixTest {
    /// Prevents instantiation.
    private NativePrefixTest() { }

    /// Verifies a large launcher, its policy trailer, and rejected launcher sizes.
    /// @param args unused arguments
    /// @throws Exception if a fixture or assertion fails
    public static void main(String[] args) throws Exception {
        Path launcher = Files.createTempFile("janex-native-prefix-", ".bin");
        try {
            largeLauncher(launcher);
            PackOptions options = new PackOptions(launcher, launcher);
            options.nativeLauncher = launcher;
            try (RandomAccessFile file = new RandomAccessFile(launcher.toFile(), "rw")) {
                file.setLength(256L * 1024 * 1024);
            }
            rejected(options, "Native launcher header exceeds byte limit");

            byte[] truncated = new byte[192];
            ByteBuffer pe = ByteBuffer.wrap(truncated).order(ByteOrder.LITTLE_ENDIAN);
            pe.put(0, (byte) 'M').put(1, (byte) 'Z').putInt(60, 64);
            pe.putInt(64, 0x00004550).putShort(84, (short) 112).putShort(88, (short) 0x20b);
            Files.write(launcher, truncated);
            rejected(options, "Truncated PE optional header");
            System.out.println("Native prefix checks passed.");
        } finally {
            Files.deleteIfExists(launcher);
        }
    }

    /// Preserves a 64 MiB launcher and appends its policy without retaining full-size copies.
    private static void largeLauncher(Path launcher) throws Exception {
        int length = 64 * 1024 * 1024;
        try (RandomAccessFile file = new RandomAccessFile(launcher.toFile(), "rw")) {
            file.setLength(length);
            file.write(new byte[] {127, 'E', 'L', 'F', 2});
            file.seek(length - 1);
            file.write(42);
        }
        PackOptions options = new PackOptions(launcher, launcher);
        options.nativeLauncher = launcher;
        options.nativeLaunchMode = "direct";
        byte[] bytes = NativePrefix.create(options);
        byte[] policy = {(byte) 0xa4, 0, 0x64, 'm', 'a', 'i', 'n', 1, 1, 2, 0, 3, 0x40};
        require(bytes.length == length + policy.length + 12, "Unexpected prefix length");
        require(bytes[0] == 127 && bytes[4] == 2 && bytes[length - 1] == 42, "Launcher bytes changed");
        require(Arrays.equals(bytes, length, length + policy.length, policy, 0, policy.length), "Native policy changed");
        require(ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).getInt(length + policy.length) == policy.length,
                "Incorrect policy length");
        require(new String(bytes, bytes.length - 8, 8, StandardCharsets.US_ASCII).equals("JNXBOOT1"),
                "Incorrect policy trailer");
    }

    /// Requires invalid input to fail with the expected validation error.
    private static void rejected(PackOptions options, String message) throws Exception {
        try {
            NativePrefix.create(options);
            throw new AssertionError("Invalid launcher was accepted");
        } catch (IOException failure) {
            require(message.equals(failure.getMessage()), failure.toString());
        }
    }

    /// Reports a failed invariant independently of JVM assertion settings.
    private static void require(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
}

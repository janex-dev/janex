// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Map;

import static org.glavo.janex.reader.internal.Input.require;

/// Encodes the private native-launcher policy shared with the Rust launcher.
final class NativePrefix {
    /// Prevents instantiation.
    private NativePrefix() { }

    /// Validates an unsigned PE or ELF and appends its application and invocation policy.
    static byte[] create(PackOptions options) throws IOException {
        if (options.nativeLauncher == null) return new byte[0];
        require(options.nativeLaunchMode.equals("bootstrap") || options.nativeLaunchMode.equals("direct"), "Invalid native launch mode");
        byte[] bytes = Resources.read(options.nativeLauncher, 256 * 1024 * 1024);
        boolean elf = bytes.length >= 52 && bytes[0] == 127 && bytes[1] == 'E' && bytes[2] == 'L'
                && bytes[3] == 'F' && (bytes[4] == 1 || bytes[4] == 2);
        if (!elf) {
            require(bytes.length >= 64 && bytes[0] == 'M' && bytes[1] == 'Z', "Native launcher must be PE or ELF");
            ByteBuffer input = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN);
            int pe = input.getInt(60);
            require(pe >= 0 && pe <= bytes.length - 26 && input.getInt(pe) == 0x00004550, "Invalid PE header");
            int length = Short.toUnsignedInt(input.getShort(pe + 20));
            int start = pe + 24;
            require(length >= 2 && length <= bytes.length - start, "Truncated PE optional header");
            int magic = Short.toUnsignedInt(input.getShort(start));
            int directory = magic == 0x10b ? 96 : magic == 0x20b ? 112 : -1;
            require(directory >= 0 && length >= directory, "Unsupported PE optional header");
            if (Integer.toUnsignedLong(input.getInt(start + directory - 4)) > 4) {
                require(length >= directory + 40 && input.getLong(start + directory + 32) == 0,
                        "Native launcher must not contain an Authenticode certificate table");
            }
        }
        byte[] config = Encoding.cbor(Map.of(0, options.applicationId, 1, options.nativeLaunchMode.equals("direct") ? 1 : 0,
                2, 0, 3, new byte[0]));
        require(config.length <= 4 * 1024 * 1024 && (long) bytes.length + config.length + 12 <= 256 * 1024 * 1024,
                "Native launcher header exceeds byte limit");
        Encoding output = new Encoding();
        output.writeBytes(bytes);
        output.writeBytes(config);
        output.little(config.length, 4);
        output.writeBytes(Encoding.utf8("JNXBOOT1"));
        return output.toByteArray();
    }
}

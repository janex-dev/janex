// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap;

import java.io.*;
import java.util.Base64;

/// Receives one compact launch description before application or agent code runs.
public final class LaunchData {
    /// Cached launch sections shared by loader initialization and the application entry point.
    private static byte[][] sections;

    /// Prevents instantiation.
    private LaunchData() {
    }

    /// Returns a new stream for entry data (0), JVM options (1), or resource requests (2).
    /// The private property is consumed once; streams have independent positions.
    /// @param section section index, from 0 through 2
    /// @return caller-owned stream over immutable section bytes
    /// @throws IOException if the launch description is missing or malformed
    public static synchronized InputStream open(int section) throws IOException {
        if (sections == null) {
            String value = System.clearProperty("janex.launch");
            if (value == null) throw new IOException("Missing Janex launch description");
            if (value.startsWith("env:")) {
                int count;
                try { count = Integer.parseInt(value.substring(4)); }
                catch (NumberFormatException invalid) { throw new IOException("Invalid launch chunk count", invalid); }
                if (count <= 0 || count > 65536) throw new IOException("Invalid launch chunk count");
                StringBuilder joined = new StringBuilder();
                for (int i = 0; i < count; i++) {
                    String part = System.getenv("JANEX_LAUNCH_" + i);
                    if (part == null || part.length() > 8000) throw new IOException("Missing or oversized launch chunk");
                    joined.append(part);
                }
                value = joined.toString();
            }
            byte[] bytes;
            try {
                bytes = Base64.getDecoder().decode(value);
            } catch (IllegalArgumentException invalid) {
                throw new IOException("Invalid Janex launch encoding", invalid);
            }
            byte[][] result = new byte[3][];
            try (DataInputStream input = new DataInputStream(new ByteArrayInputStream(bytes))) {
                if (input.readInt() != 0x4a4e5831) throw new IOException("Invalid Janex launch description");
                for (int i = 0; i < result.length; i++) {
                    int length = input.readInt();
                    if (length < 0 || length > input.available()) throw new IOException("Invalid launch section length");
                    result[i] = new byte[length];
                    input.readFully(result[i]);
                }
                if (input.read() != -1) throw new IOException("Trailing launch description data");
            }
            sections = result;
        }
        return new ByteArrayInputStream(sections[section]);
    }

    /// Encodes the three private sections for a child JVM using ASCII process arguments.
    /// Input arrays are copied. The result remains subject to native process argument limits;
    /// launchers may move its Base64 payload into numbered environment chunks.
    /// @param entry encoded entry point and UTF-16 program arguments
    /// @param options encoded entry point and JVM options
    /// @param resources selected resource-root requests, or an empty array
    /// @return one complete private JVM property argument
    /// @throws IOException if writing the description fails
    public static String encode(byte[] entry, byte[] options, byte[] resources) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        DataOutputStream output = new DataOutputStream(bytes);
        output.writeInt(0x4a4e5831);
        for (byte[] section : new byte[][]{entry, options, resources}) {
            output.writeInt(section.length);
            output.write(section);
        }
        return "-Djanex.launch=" + Base64.getEncoder().encodeToString(bytes.toByteArray());
    }
}

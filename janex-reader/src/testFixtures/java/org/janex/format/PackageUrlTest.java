// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.format;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

/// Checks canonical Package URLs against native decoded components and application validation.
public final class PackageUrlTest {
    /// Prevents instantiation.
    private PackageUrlTest() {
    }

    /// Reads shared vectors and validates every component, immutable qualifier views, and placement.
    public static void main(String[] arguments) throws Exception {
        try (DataInputStream input = new DataInputStream(new BufferedInputStream(new FileInputStream(arguments[0])))) {
            int count = input.readInt();
            for (int i = 0; i < count; i++) {
                String uri = text(input);
                boolean valid = input.readBoolean();
                String[] expected = new String[5];
                Map<String, String> qualifiers = new TreeMap<String, String>();
                if (valid) {
                    for (int j = 0; j < expected.length; j++) {
                        expected[j] = text(input);
                    }
                    int size = input.readInt();
                    for (int j = 0; j < size; j++) {
                        qualifiers.put(text(input), text(input));
                    }
                }
                try {
                    PackageUrl value = PackageUrl.parse(uri);
                    String[] actual = {value.type(), nullable(value.namespace()), value.name(), nullable(value.version()), nullable(value.subpath())};
                    if (!valid || !Arrays.equals(expected, actual) || !qualifiers.equals(value.qualifiers()) || !uri.equals(value.toString())) {
                        throw new AssertionError("Package URL mismatch at " + i + ": " + uri);
                    }
                    try {
                        value.qualifiers().put("mutation", "value");
                        throw new AssertionError("Mutable Package URL qualifiers");
                    } catch (UnsupportedOperationException expectedFailure) {
                        // Qualifier views remain immutable after parsing.
                    }
                } catch (IOException failure) {
                    if (valid) {
                        throw new AssertionError("Valid Package URL rejected at " + i + ": " + uri, failure);
                    }
                }
                for (boolean module : new boolean[]{false, true}) {
                    boolean accepted = input.readBoolean();
                    try {
                        ModuleRequirement.parse(uri, module);
                        if (!accepted) {
                            throw new AssertionError("Invalid path reference accepted at " + i + ": " + uri);
                        }
                    } catch (IOException failure) {
                        if (accepted) {
                            throw new AssertionError("Valid path reference rejected at " + i + ": " + uri, failure);
                        }
                    }
                }
                byte[] application = bytes(input);
                byte[] typeInfo = bytes(input);
                boolean applicationValid = input.readBoolean();
                try {
                    new Application(application, typeInfo);
                    if (!applicationValid) {
                        throw new AssertionError("Invalid inactive reference accepted at " + i + ": " + uri);
                    }
                } catch (IOException failure) {
                    if (applicationValid) {
                        throw new AssertionError("Valid inactive reference rejected at " + i + ": " + uri, failure);
                    }
                }
            }
            for (int point = 0; point <= Character.MAX_CODE_POINT; point++) {
                if (point >= Character.MIN_SURROGATE && point <= Character.MAX_SURROGATE) {
                    continue;
                }
                String value = new String(Character.toChars(point));
                if (UnicodeCaseChecks.isLowercase(value) != input.readBoolean()) {
                    throw new AssertionError("Unicode lowercase mismatch at U+" + Integer.toHexString(point));
                }
            }
            if (input.read() != -1) {
                throw new AssertionError("Trailing Package URL vectors");
            }
        }
    }

    /// Represents an absent component by an empty string in the trusted comparison protocol.
    private static String nullable(String value) {
        return value == null ? "" : value;
    }

    /// Reads a trusted length-prefixed string.
    private static String text(DataInputStream input) throws IOException {
        return new String(bytes(input), StandardCharsets.UTF_8);
    }

    /// Reads a trusted length-prefixed byte sequence.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] value = new byte[input.readInt()];
        input.readFully(value);
        return value;
    }
}

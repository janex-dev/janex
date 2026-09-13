// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap;

import java.io.*;
import java.nio.file.*;
import java.util.*;

import org.janex.reader.*;
import org.janex.reader.Checksum;
import org.janex.reader.ContainerReader;
import org.janex.reader.JanexReader;
import org.janex.reader.internal.codec.zstd.Zstandard;

/// Checks caller authentication decisions and complete-content requirements before dependency acquisition.
public final class ReaderAuthenticationTest {
    /// Prevents instantiation.
    private ReaderAuthenticationTest() {
    }

    /// Reads an owned byte string from the trusted native oracle stream.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] value = new byte[input.readInt()];
        input.readFully(value);
        return value;
    }

    /// Compares unsigned defaults, exact pinned inputs, required signatures, and rejection behavior.
    ///
    /// Signed fixtures have their Janex profile and trust verified by the native test before they
    /// enter this stream. The callback pins that exact declaration and document; it is not a Java
    /// implementation of OpenPGP or CMS cryptography.
    ///
    /// @param args oracle stream path and reusable snapshot path
    /// @throws Exception if fixture I/O fails or preparation violates the expected policy boundary
    public static void main(String[] args) throws Exception {
        Path snapshot = Paths.get(args[1]);
        byte[] emptyJar = new byte[22];
        emptyJar[0] = 'P';
        emptyJar[1] = 'K';
        emptyJar[2] = 5;
        emptyJar[3] = 6;
        try (DataInputStream input = new DataInputStream(Files.newInputStream(Paths.get(args[0])))) {
            int count = input.readInt();
            for (int i = 0; i < count; i++) {
                Files.write(snapshot, bytes(input));
                int type = input.readUnsignedByte();
                byte[] document = bytes(input);
                byte[] payload = bytes(input);
                boolean trusted = input.readBoolean();
                boolean integrity = input.readBoolean();
                int verified = input.readInt();
                boolean complete = input.readBoolean();
                for (int mode = 0; mode <= 4; mode++) {
                    final int policyMode = mode;
                    int[] calls = new int[2];
                    IOException rejection = new IOException("fixture policy rejection");
                    RuntimeException runtimeFailure = new IllegalStateException("fixture policy failure");
                    JanexReader.AuthenticationPolicy policy = mode == 0 ? null : (verification, original) -> {
                        calls[0]++;
                        check(verification.type().ordinal() == type, "Policy received wrong type");
                        check(Arrays.equals(original, document), "Policy received re-encoded metadata");
                        check(Arrays.equals(verification.payload(), payload), "Policy received changed payload");
                        if (policyMode == 4) {
                            throw runtimeFailure;
                        }
                        if (policyMode == 3 || policyMode == 2 && type < 2 || type >= 2 && !trusted) {
                            throw rejection;
                        }
                        Arrays.fill(original, (byte) 0);
                        Arrays.fill(verification.payload(), (byte) 0);
                    };
                    JanexReader.DependencyResolver resolver = (uri, checksum) -> {
                        check(calls[0] == (policyMode == 0 ? 0 : 1), "Dependency preceded authentication");
                        calls[1]++;
                        check(uri.equals("https://example.invalid/library.jar"), "Unexpected dependency");
                        Checksum.decode(checksum).verify(emptyJar);
                        return new JanexReader.Dependency("library.jar", emptyJar);
                    };
                    boolean expected = integrity && mode < 3 && (mode != 2 || type >= 2)
                            && (type < 2 || mode != 0 && trusted && complete);
                    try (JanexReader reader = new JanexReader(snapshot, (encoded, length, dictionary) -> {
                        byte[] decoded = new byte[length];
                        check(Zstandard.decompress(encoded, 0, encoded.length, decoded, 0, length, dictionary) == length, "Decoded length");
                        return decoded;
                    }, resolver, policy)) {
                        check(expected, "Preparation accepted a rejected fixture at " + i + ", mode " + mode);
                        check(calls[1] == 0, "Constructor acquired a dependency");
                        ContainerReader.IntegrityReport report = reader.integrity();
                        check(report.checksumsVerified() == verified && report.completeSecureCoverage() == complete, "Retained integrity report");
                        JanexReader.Launch launch = reader.launch(null);
                        check(launch.mainClass.equals("Main") && launch.resources != null, "Prepared application");
                        check(calls[1] == 1, "Selected dependency not acquired exactly once");
                        reader.close();
                        check(reader.integrity() == report, "Integrity report lost on closure");
                    } catch (IOException | RuntimeException failure) {
                        if (expected) {
                            throw new AssertionError("Valid preparation failed at " + i + ", mode " + mode, failure);
                        }
                        check(calls[1] == 0, "Failed preparation acquired a dependency");
                        if (mode == 3 || mode == 2 && type < 2 || mode > 0 && mode < 3 && type >= 2 && !trusted) {
                            check(failure == rejection, "Policy rejection was replaced or downgraded");
                        } else if (mode == 4) {
                            check(failure == runtimeFailure, "Policy runtime failure was replaced");
                        }
                    }
                    check(calls[0] == (mode == 0 ? 0 : 1), "Policy call count");
                }
            }
            check(input.read() == -1, "Trailing authentication fixtures");
            System.out.println("Verified " + count + " authentication fixtures");
        }
    }

    /// Requires a test invariant.
    private static void check(boolean value, String message) {
        if (!value) {
            throw new AssertionError(message);
        }
    }
}

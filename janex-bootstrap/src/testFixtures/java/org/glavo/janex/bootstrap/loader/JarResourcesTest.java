// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap.loader;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.zip.*;

import org.glavo.janex.reader.*;

/// Checks lazy external resources with a decoded payload larger than the test JVM's heap.
public final class JarResourcesTest {
    /// Prevents instantiation.
    private JarResourcesTest() {
    }

    /// Checks multi-release selection, bounded preparation, deferred CRC failures, and handle cleanup.
    /// @param arguments path to an empty valid Janex container in a private test directory
    /// @throws Exception if a resource assertion fails
    public static void main(String[] arguments) throws Exception {
        Path snapshot = Paths.get(arguments[0]);
        Path jar = snapshot.resolveSibling("deferred.jar");
        createJar(jar);
        corruptUnusedEntry(jar);
        List<ResourceRequest> requests = Collections.singletonList(new ResourceRequest(false, jar, "library.jar"));
        for (String version : new String[]{"8", "17"}) {
            String[] context = {"windows", "x86-64", "run", version, "test"};
            ResourcePlan plan = JanexReader.prepareResources(snapshot, requests, Collections.<String, String>emptyMap(),
                    context, ReadLimits.DEFAULT, 128L * 1024 * 1024);
            ResourceIndex.Resource saved;
            ResourceIndex index = new ResourceIndex(plan);
            try {
                Map<String, ResourceIndex.Resource> files = index.roots().get(0).files();
                saved = files.get("sample.txt");
                String expected = version.equals("8") ? "base" : "selected";
                check(new String(saved.readBytes(), StandardCharsets.UTF_8).equals(expected));
                check(new String(saved.readBytes(), StandardCharsets.UTF_8).equals(expected));
                check(files.get("unused.bin").length() == 96 * 1024 * 1024);
                byte[] manifest = files.get("META-INF/MANIFEST.MF").readBytes();
                check(!new String(manifest, StandardCharsets.UTF_8).contains("Class-Path"));
                try {
                    files.get("bad.bin").readBytes();
                    throw new AssertionError("Corrupt payload accepted");
                } catch (IOException expectedFailure) {
                    check(expectedFailure.getMessage().contains("CRC"));
                }
            } finally {
                index.close();
                index.close();
            }
            try {
                saved.readBytes();
                throw new AssertionError("Closed resource index accepted");
            } catch (IOException expectedFailure) {
                check(index.isClosed());
            }
        }
        try {
            JanexReader.prepareResources(snapshot, requests, Collections.<String, String>emptyMap(),
                    null, ReadLimits.DEFAULT, 1024);
            throw new AssertionError("Logical resource limit ignored");
        } catch (IOException expected) {
            check(expected.getMessage().contains("limit"));
        }
        Files.delete(jar);
    }

    /// Writes a large unused deflated entry using a fixed-size buffer.
    private static void createJar(Path path) throws IOException {
        try (ZipOutputStream output = new ZipOutputStream(Files.newOutputStream(path))) {
            entry(output, "META-INF/MANIFEST.MF", "Manifest-Version: 1.0\r\nMulti-Release: true\r\nClass-Path: ignored.jar\r\n\r\n");
            entry(output, "sample.txt", "base");
            entry(output, "META-INF/versions/17/sample.txt", "selected");
            output.putNextEntry(new ZipEntry("unused.bin"));
            byte[] block = new byte[8192];
            for (int i = 0; i < 96 * 128; i++) output.write(block);
            output.closeEntry();
            byte[] bad = "bad-payload".getBytes(StandardCharsets.UTF_8);
            CRC32 crc = new CRC32();
            crc.update(bad);
            ZipEntry entry = new ZipEntry("bad.bin");
            entry.setMethod(ZipEntry.STORED);
            entry.setSize(bad.length);
            entry.setCrc(crc.getValue());
            output.putNextEntry(entry);
            output.write(bad);
            output.closeEntry();
        }
    }

    /// Writes one small deflated entry.
    private static void entry(ZipOutputStream output, String name, String value) throws IOException {
        output.putNextEntry(new ZipEntry(name));
        output.write(value.getBytes(StandardCharsets.UTF_8));
        output.closeEntry();
    }

    /// Corrupts only a stored payload, leaving both directory and local-header metadata intact.
    private static void corruptUnusedEntry(Path path) throws IOException {
        byte[] bytes = Files.readAllBytes(path);
        byte[] marker = "bad-payload".getBytes(StandardCharsets.UTF_8);
        for (int i = 0; i <= bytes.length - marker.length; i++) {
            if (Arrays.equals(Arrays.copyOfRange(bytes, i, i + marker.length), marker)) {
                bytes[i] ^= 1;
                Files.write(path, bytes);
                return;
            }
        }
        throw new AssertionError("Missing fixture payload");
    }

    /// Rejects an unexpected observable result.
    private static void check(boolean condition) {
        if (!condition) throw new AssertionError();
    }
}

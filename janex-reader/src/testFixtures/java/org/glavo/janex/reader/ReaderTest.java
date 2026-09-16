// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.math.BigInteger;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.glavo.janex.reader.internal.Conditions;
import org.glavo.janex.reader.internal.Input;
import org.glavo.janex.reader.internal.JarArchive;

/// Exercises deterministic encoding and version boundaries independently of the native writer.
public final class ReaderTest {
    /// Prevents instantiation.
    private ReaderTest() {
    }

    /// Runs boundary checks without external test dependencies.
    ///
    /// @param arguments unused
    /// @throws Exception if an accepted value fails or a malformed value is accepted
    public static void main(String[] arguments) throws Exception {
        check(new Input(new byte[]{0}).map().isEmpty());
        reject(() -> new Input(new byte[]{1, (byte) 0xa0}).map());
        reject(() -> new Input(new byte[]{3, (byte) 0xa1, 0, 0, 0}).end());
        reject(() -> new Input(new byte[]{4, (byte) 0xa1, 0x18, 0, 0}).map());
        reject(() -> new Input(new byte[]{5, (byte) 0xa2, 1, 0, 0, 0}).map());
        reject(() -> new Input(new byte[]{5, (byte) 0xa2, 0, 0, 0, 1}).map());
        reject(() -> new Input(new byte[]{4, (byte) 0xa1, 0, 0x61, (byte) 0xff}).map());
        reject(() -> new Input(new byte[]{1, (byte) 0xbf}).map());
        reject(() -> new Input(new byte[]{5, (byte) 0xa1, 0, (byte) 0xf9, 0x7e, 1}).map());
        reject(() -> new Input(new byte[]{7, (byte) 0xa1, 0, (byte) 0xfa, 0x3f, (byte) 0x80, 0, 0}).map());
        Map<Object, Object> map = new Input(new byte[]{5, (byte) 0xa1, 0, (byte) 0xf9, 0x3c, 0}).map();
        check(map.size() == 1);
        byte[] maximum = new byte[10];
        Arrays.fill(maximum, (byte) 0xff);
        maximum[9] = 1;
        check(new Input(maximum).uint() == -1L);
        maximum[9] = 2;
        reject(() -> new Input(maximum).uint());
        check(new Input(new byte[]{(byte) 0x80, 0}).uint() == 0);
        reject(() -> new Input(new byte[]{(byte) 0x80}).uint());
        byte[] unsigned = {11, (byte) 0xa1, 0, 0x1b, (byte) 0xff, (byte) 0xff, (byte) 0xff, (byte) 0xff,
                (byte) 0xff, (byte) 0xff, (byte) 0xff, (byte) 0xff};
        check(Input.number(Input.get(new Input(unsigned).map(), 0)) == -1L);
        check(Conditions.range("vers:jep322/>=8", "1.8.0_452-b9"));
        reject(() -> Conditions.range("vers:jep322/>=8", "1.8.0_452-b09"));
        reject(() -> Conditions.range("vers:jep322/>=8", "1.8.0+1"));
        reject(() -> Conditions.range("vers:jep322/>=8", "1.8.0_452-ea-extra"));
        check(Conditions.range("vers:jep322/>=11|<17|>=21", "25.0.1+8-LTS"));
        check(!Conditions.range("vers:jep322/>=11|<17|>=21", "17"));
        check(!Conditions.range("vers:jep322/>=11|!=17|<21", "17"));
        check(Conditions.range("vers:jep322/>=11|!=17|<21", "18"));
        check(!Conditions.range("vers:jep322/>=25", "25-ea"));
        check(Conditions.range("vers:jep322/8u452", "1.8.0_452"));
        reject(() -> Conditions.range("vers:jep322/>=11|>=17", "25"));
        reject(() -> Conditions.range("vers:jep322/17|<21", "25"));
        reject(() -> Conditions.range("vers:jep322/17.0", "25"));
        reject(() -> Conditions.range("vers:jep322/17|11", "25"));
        Map<Object, Object> reserved = new Input(new byte[]{3, (byte) 0xa1, 0, 0}).map();
        reject(() -> Conditions.matches(reserved));
        // Unknown extension tags remain opaque rather than being interpreted as timestamps.
        Map<Object, Object> extension = new Input(new byte[]{5, (byte) 0xa1, 0x18, 99, (byte) 0xc2, 0x40}).map();
        check(extension.containsKey(BigInteger.valueOf(99)));
        archives();
        resourcePlan();
    }

    /// Checks that selected-resource descriptions do not expose mutable preparation state.
    private static void resourcePlan() {
        byte[] bytes = {42};
        int[][] extents = {{0, 0, 1}};
        ResourcePlan.Source inline = new ResourcePlan.Source(bytes, -1, 0, new int[0], new int[0][3]);
        ResourcePlan.Source extent = new ResourcePlan.Source(null, -1, 0, new int[0], extents);
        int[][] transforms = {{10, 0}};
        BigInteger[] times = {BigInteger.ONE.shiftLeft(100), null, null};
        ResourcePlan.File file = new ResourcePlan.File(1, transforms, times, 0);
        Map<String, ResourcePlan.File> files = new java.util.LinkedHashMap<String, ResourcePlan.File>();
        files.put("value", file);
        ResourcePlan.Root root = new ResourcePlan.Root("example.jar", false, files);
        byte[][][] pools = {{new byte[0], new byte[]{42}}};
        ResourcePlan plan = new ResourcePlan(java.nio.file.Paths.get("snapshot.janex"), ReadLimits.DEFAULT,
                Arrays.asList(inline, extent), pools, java.util.Collections.emptyMap(), Arrays.asList(root));
        bytes[0] = 0;
        extents[0][2] = 0;
        transforms[0][0] = 0;
        times[0] = null;
        pools[0][1][0] = 0;
        files.clear();
        check(plan.sources().get(0).inline()[0] == 42);
        check(plan.sources().get(1).extents()[0][2] == 1);
        check(plan.roots().get(0).files().get("value").transforms()[0][0] == 10);
        check(file.times()[0].equals(BigInteger.ONE.shiftLeft(100)) && file.permissions() == 0);
        check(plan.pools()[0][1][0] == 42);
        inline.inline()[0] = 0;
        extent.extents()[0][2] = 0;
        file.transforms()[0][0] = 0;
        file.times()[0] = null;
        plan.pools()[0][1][0] = 0;
        check(inline.inline()[0] == 42 && extent.extents()[0][2] == 1);
        check(file.transforms()[0][0] == 10 && file.times()[0] != null);
        check(plan.pools()[0][1][0] == 42);
        try {
            plan.roots().clear();
            throw new AssertionError("Mutable resource roots");
        } catch (UnsupportedOperationException expected) {
            // Mutating a returned collection must leave the plan unchanged.
        }
        try {
            root.files().clear();
            throw new AssertionError("Mutable resource paths");
        } catch (UnsupportedOperationException expected) {
            check(root.files().size() == 1);
        }
    }

    /// Checks ZIP framing, central-directory identity, and deflate integrity before resource import.
    private static void archives() throws Exception {
        ByteArrayOutputStream buffer = new ByteArrayOutputStream();
        try (ZipOutputStream zip = new ZipOutputStream(buffer)) {
            zip.putNextEntry(new ZipEntry("empty/"));
            zip.closeEntry();
            zip.putNextEntry(new ZipEntry("value"));
            zip.write(new byte[]{1, 2, 3});
            zip.closeEntry();
        }
        byte[] original = buffer.toByteArray();
        List<JarArchive.Entry> entries = JarArchive.read(original);
        check(entries.size() == 2 && entries.get(0).bytes().length == 0);
        check(Arrays.equals(entries.get(1).bytes(), new byte[]{1, 2, 3}));
        reject(() -> JarArchive.read(Arrays.copyOf(original, original.length - 1)));
        reject(() -> JarArchive.read(new byte[0]));
        int directory = -1;
        for (int index = 0; index < original.length - 3; index++) {
            if (original[index] == 'P' && original[index + 1] == 'K' && original[index + 2] == 1 && original[index + 3] == 2) {
                directory = index;
                break;
            }
        }
        check(directory >= 0);
        byte[] unspecifiedMode = original.clone();
        unspecifiedMode[directory + 5] = 3;
        unspecifiedMode[directory + 38] = 0x10;
        unspecifiedMode[directory + 40] = (byte) 0xff;
        unspecifiedMode[directory + 41] = (byte) 0xff;
        check(JarArchive.read(unspecifiedMode).get(0).mode() == -1);
        // A FIFO remains unsupported; accepting the sentinel must not relax type validation.
        byte[] fifo = original.clone();
        fifo[directory + 5] = 3;
        fifo[directory + 41] = 0x10;
        reject(() -> JarArchive.read(fifo));
        byte[] name = original.clone();
        name[directory + 46] ^= 1;
        reject(() -> JarArchive.read(name));
        byte[] crc = original.clone();
        crc[directory + 16] ^= 1;
        reject(() -> JarArchive.read(crc));
        byte[] count = original.clone();
        count[count.length - 12] = 3;
        reject(() -> JarArchive.read(count));
        byte[] size = original.clone();
        Arrays.fill(size, directory + 24, directory + 28, (byte) 0xff);
        reject(() -> JarArchive.read(size));
    }

    /// Fails the test when an expected invariant does not hold.
    private static void check(boolean condition) {
        if (!condition) {
            throw new AssertionError();
        }
    }

    /// Requires a malformed value to fail with the reader's checked format exception.
    private static void reject(Action action) throws Exception {
        try {
            action.run();
        } catch (IOException expected) {
            return;
        }
        throw new AssertionError("Malformed input accepted");
    }

    /// One potentially failing parser operation.
    private interface Action {
        /// Executes a parser check.
        void run() throws Exception;
    }
}

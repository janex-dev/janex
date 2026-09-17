// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.ByteArrayOutputStream;
import java.io.ByteArrayInputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.math.BigInteger;
import java.time.Instant;
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
        check(extension.containsKey(99L));
        archives();
        resourcePlan();
        dataPools();
        repeatedPoolEntries();
        poolIndexFraming();
        timestamps();
        cborIntegers();
    }

    /// Checks integer representation boundaries and unsigned bit preservation.
    private static void cborIntegers() throws Exception {
        check(new Input(new byte[]{0}).cbor(0).equals(0L));
        check(new Input(new byte[]{0x18, 24}).cbor(0).equals(24L));
        check(new Input(new byte[]{0x20}).cbor(0).equals(-1L));
        for (long bits : new long[]{0x1_0000_0000L, Long.MAX_VALUE, Long.MIN_VALUE, -1L}) {
            BigInteger magnitude = new BigInteger(Long.toUnsignedString(bits));
            for (int major = 0; major <= 1; major++) {
                byte[] encoded = java.nio.ByteBuffer.allocate(9)
                        .put((byte) ((major << 5) | 27)).putLong(bits).array();
                Object value = new Input(encoded).cbor(0);
                BigInteger expected = major == 0 ? magnitude : magnitude.negate().subtract(BigInteger.ONE);
                if (expected.bitLength() <= 63) {
                    check(value instanceof Long && value.equals(expected.longValueExact()));
                } else {
                    check(value instanceof BigInteger && value.equals(expected));
                }
                if (major == 0) {
                    check(Input.number(value) == bits);
                } else {
                    reject(() -> Input.number(value));
                }
                BigInteger[] parts = expected.divideAndRemainder(BigInteger.valueOf(1_000_000_000));
                check(Input.timestamp(value).equals(Instant.ofEpochSecond(parts[0].longValueExact(), parts[1].longValue())));
            }
        }
        reject(() -> Input.number(1));
    }

    /// Checks exact negative normalization and both Instant boundaries without saturation.
    private static void timestamps() throws Exception {
        BigInteger minimum = new BigInteger("-31557014167219200000000000");
        BigInteger maximum = new BigInteger("31556889864403199999999999");
        check(Input.timestamp(minimum).equals(Instant.MIN));
        check(Input.timestamp(maximum).equals(Instant.MAX));
        check(Input.timestamp(0L).equals(Instant.EPOCH));
        check(Input.timestamp(-1L).equals(Instant.ofEpochSecond(-1, 999_999_999)));
        check(Input.timestamp(-1_000_000_001L).equals(Instant.ofEpochSecond(-2, 999_999_999)));
        reject(() -> Input.timestamp(minimum.subtract(BigInteger.ONE)));
        reject(() -> Input.timestamp(maximum.add(BigInteger.ONE)));
        reject(() -> Input.timestamp(BigInteger.ONE.shiftLeft(127)));
        reject(() -> Input.timestamp(BigInteger.ONE.shiftLeft(127).negate()));
    }

    /// Checks contiguous-pool ownership, read-only views, framing, and stream boundaries.
    private static void dataPools() throws Exception {
        byte[] encoded = {3, 0, 1, (byte) 0xff, 2, (byte) 0xc0, (byte) 0x80};
        DataPool pool = DataPool.decode(encoded, ReadLimits.DEFAULT);
        for (int end = 0; end < encoded.length; end++) {
            final byte[] prefix = Arrays.copyOf(encoded, end);
            reject(() -> DataPool.decode(prefix, ReadLimits.DEFAULT));
        }
        Arrays.fill(encoded, (byte) 0);
        check(pool.size() == 3 && pool.byteLength(0) == 0);
        check(pool.view(1).get() == (byte) 0xff);
        java.nio.ByteBuffer view = pool.view(2);
        check(view.position() == 0 && view.limit() == 2 && view.isReadOnly() && !view.hasArray());
        view.get();
        check(pool.view(2).position() == 0);
        DataPool.Cursor cursor = pool.cursor(2);
        check(cursor.remaining() == 2 && cursor.readUnsignedByte() == 0xc0);
        check(pool.cursor(2).readUnsignedByte() == 0xc0);
        check(cursor.readUnsignedByte() == 0x80 && cursor.remaining() == 0);
        reject(() -> cursor.readUnsignedByte());
        check(cursor.remaining() == 0);
        check(pool.cursor(0).remaining() == 0);
        reject(() -> pool.cursor(0).readUnsignedByte());
        reject(() -> pool.cursor(-1));
        reject(() -> pool.cursor(3));
        try {
            view.put(0, (byte) 0);
            throw new AssertionError("Mutable pool view");
        } catch (java.nio.ReadOnlyBufferException expected) {
            check(pool.view(2).get() == (byte) 0xc0);
        }
        byte[] copy = {7, 7, 7, 7};
        pool.copyTo(2, copy, 1);
        check(Arrays.equals(copy, new byte[]{7, (byte) 0xc0, (byte) 0x80, 7}));
        try {
            pool.copyTo(2, copy, 3);
            throw new AssertionError("Out-of-bounds copy accepted");
        } catch (IndexOutOfBoundsException expected) {
            check(copy[3] == 7);
        }
        reject(() -> pool.view(-1));
        reject(() -> pool.byteLength(3));
        reject(() -> DataPool.decode(new byte[]{1, 1, 42}, ReadLimits.DEFAULT));
        reject(() -> DataPool.decode(new byte[]{1, 0, 0}, ReadLimits.DEFAULT));
        reject(() -> DataPool.decode(new byte[]{1, 0}, new ReadLimits(1, 1, 1)));

        byte[] large = new byte[20000];
        for (int i = 0; i < large.length; i++) large[i] = (byte) i;
        DataPool original = DataPool.copyOf(new byte[][]{new byte[0], new byte[]{42}, large});
        large[0] = 42;
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        original.writeIndex(new DataOutputStream(bytes) {
            /// Overwrites supplied buffers after writing to detect leaked pool storage.
            @Override
            public void write(byte[] value, int offset, int length) throws IOException {
                super.write(value, offset, length);
                Arrays.fill(value, (byte) 0);
            }
        });
        check(original.view(1).get() == 42 && original.view(2).get() == 0);
        final byte[][] retained = {null};
        DataInputStream input = new DataInputStream(new ByteArrayInputStream(bytes.toByteArray()) {
            /// Retains the destination to verify that subsequent mutation cannot affect the pool.
            @Override
            public synchronized int read(byte[] value, int offset, int length) {
                retained[0] = value;
                return super.read(value, offset, length);
            }
        });
        DataPool restored = DataPool.readIndex(input, ReadLimits.DEFAULT);
        check(input.read() == -1);
        Arrays.fill(retained[0], (byte) 42);
        for (int i = 0; i < original.size(); i++) check(original.view(i).equals(restored.view(i)));
        reject(() -> DataPool.readIndex(new DataInputStream(new ByteArrayInputStream(bytes.toByteArray())),
                new ReadLimits(20000, 3, 1)));
        reject(() -> DataPool.readIndex(new DataInputStream(new ByteArrayInputStream(new byte[4])), ReadLimits.DEFAULT));
    }

    /// Checks that all pool readers preserve indices without requiring distinct entry bytes.
    private static void repeatedPoolEntries() throws Exception {
        byte[][] entries = {new byte[0], {(byte) 0xff}, {'x'}, {(byte) 0xff}, new byte[0]};
        DataPool decoded = DataPool.decode(new byte[]{5, 0, 1, (byte) 0xff, 1, 'x', 1, (byte) 0xff, 0},
                ReadLimits.DEFAULT);
        DataPool copied = DataPool.copyOf(entries);
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        DataOutputStream output = new DataOutputStream(bytes);
        decoded.writeIndex(output);
        output.writeByte(42);
        DataInputStream input = new DataInputStream(new ByteArrayInputStream(bytes.toByteArray()));
        DataPool restored = DataPool.readIndex(input, ReadLimits.DEFAULT);
        check(input.read() == 42 && input.read() == -1);
        for (DataPool pool : new DataPool[]{decoded, copied, restored}) {
            check(pool.size() == entries.length);
            for (int i = 0; i < entries.length; i++) {
                check(pool.view(i).equals(java.nio.ByteBuffer.wrap(entries[i])));
            }
            reject(() -> pool.view(entries.length));
        }
    }

    /// Checks the private index's independent length table and exact payload boundaries.
    private static void poolIndexFraming() throws Exception {
        byte[] wire = java.nio.ByteBuffer.allocate(23)
                .putInt(3).putInt(3).putInt(0).putInt(1).putInt(2)
                .put(new byte[]{42, 43, 44}).array();
        DataPool pool = DataPool.readIndex(new DataInputStream(new ByteArrayInputStream(wire)), ReadLimits.DEFAULT);
        check(pool.size() == 3 && pool.view(1).get() == 42 && pool.view(2).get(1) == 44);
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        pool.writeIndex(new DataOutputStream(output));
        check(Arrays.equals(wire, output.toByteArray()));
        for (int end = 0; end < wire.length; end++) {
            byte[] truncated = Arrays.copyOf(wire, end);
            reject(() -> DataPool.readIndex(new DataInputStream(new ByteArrayInputStream(truncated)), ReadLimits.DEFAULT));
        }
        for (int[] fields : new int[][]{
                {0, 0}, {-1, 0}, {Integer.MAX_VALUE, 0}, {1, -1, 0},
                {1, 1, 1}, {2, 1, 0, -1}, {2, 1, 0, 2}, {2, 2, 0, 1},
                {3, Integer.MAX_VALUE, 0, Integer.MAX_VALUE, 1}}) {
            java.nio.ByteBuffer malformed = java.nio.ByteBuffer.allocate(fields.length * 4);
            for (int field : fields) malformed.putInt(field);
            reject(() -> DataPool.readIndex(new DataInputStream(new ByteArrayInputStream(malformed.array())),
                    ReadLimits.DEFAULT));
        }
        DataPool empty = DataPool.readIndex(new DataInputStream(new ByteArrayInputStream(new byte[]{
                0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0})), ReadLimits.DEFAULT);
        check(empty.size() == 1 && empty.byteLength(0) == 0);
    }

    /// Checks that public resource accessors do not expose owned mutable storage.
    private static void resourcePlan() throws IOException {
        byte[] bytes = {42};
        ResourcePlan.Extent[] extents = {new ResourcePlan.Extent(0, 0, 1)};
        ResourcePlan.Source inline = new ResourcePlan.Source(bytes, -1, 0, new int[0], new ResourcePlan.Extent[0], null);
        ResourcePlan.Source extent = new ResourcePlan.Source(null, -1, 0, new int[0], extents, null);
        ResourcePlan.ClassFileTransform[] transforms = {new ResourcePlan.ClassFileTransform(10, 0)};
        ResourcePlan.File file = new ResourcePlan.File(1, transforms, Instant.MAX, null, null, 0);
        Map<String, ResourcePlan.File> files = new java.util.LinkedHashMap<String, ResourcePlan.File>();
        files.put("value", file);
        ResourcePlan.Root root = new ResourcePlan.Root("example.jar", false, files);
        DataPool immutable = DataPool.copyOf(new byte[][]{new byte[0], new byte[]{42}});
        DataPool[] pools = {immutable};
        ResourcePlan plan = new ResourcePlan(java.nio.file.Paths.get("snapshot.janex"), ReadLimits.DEFAULT,
                Arrays.asList(inline, extent), pools, java.util.Collections.emptyMap(), Arrays.asList(root));
        bytes[0] = 0;
        files.clear();
        check(plan.sources().get(0).inline()[0] == 42);
        check(plan.sources().get(1).extents().get(0).length() == 1);
        check(plan.roots().get(0).files().get("value").transforms().get(0).decodedLength() == 10);
        check(file.creationTime().equals(Instant.MAX) && file.permissions() == 0);
        check(file.lastModifiedTime() == null && file.lastAccessTime() == null);
        check(plan.pools()[0] == immutable && plan.pools()[0].view(1).get() == 42);
        inline.inline()[0] = 0;
        try {
            extent.extents().set(0, new ResourcePlan.Extent(1, 1, 1));
            throw new AssertionError("Mutable resource extents");
        } catch (UnsupportedOperationException expected) {
            check(extent.extents().get(0).sourceIndex() == 0 && extent.extents().get(0).offset() == 0);
        }
        try {
            file.transforms().set(0, new ResourcePlan.ClassFileTransform(0, 1));
            throw new AssertionError("Mutable resource transforms");
        } catch (UnsupportedOperationException expected) {
            check(file.transforms().get(0).dataPoolIndex() == 0);
        }
        plan.pools()[0] = null;
        check(inline.inline()[0] == 42 && extent.extents().get(0).length() == 1);
        check(file.transforms().get(0).decodedLength() == 10 && file.creationTime() != null);
        check(plan.pools()[0] == immutable && plan.pools()[0].view(1).get() == 42);
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

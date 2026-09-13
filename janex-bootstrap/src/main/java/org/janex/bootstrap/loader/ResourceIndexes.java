// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.bootstrap.loader;

import java.io.*;
import java.math.BigInteger;
import java.util.*;

import org.janex.reader.ReadLimits;
import org.janex.reader.ResourcePlan;

/// Serializes selected resources into the Host-compatible private bootstrap index.
public final class ResourceIndexes {
    /// Prevents instantiation.
    private ResourceIndexes() {
    }

    /// Encodes one resource plan without opening its snapshot or changing the plan.
    ///
    /// @param plan selected resources with inherited byte limits
    /// @return owned private-index bytes
    /// @throws IOException if the encoded index or a string exceeds the plan's byte limit
    public static byte[] encode(ResourcePlan plan) throws IOException {
        ReadLimits limits = plan.limits();
        IndexBuffer buffer = new IndexBuffer(limits);
        DataOutputStream output = new DataOutputStream(buffer);
        output.writeBytes("JNXRES01");
        output.writeInt(limits.maxBytes());
        output.writeInt(limits.maxElements());
        string(output, plan.snapshot().toString(), limits);
        output.writeInt(plan.sources().size());
        for (ResourcePlan.Source source : plan.sources()) {
            byte[] inline = source.inline();
            if (inline != null) {
                output.writeByte(0);
                output.writeInt(inline.length);
                output.write(inline);
            } else if (source.offset() >= 0) {
                output.writeByte(1);
                output.writeLong(source.offset());
                output.writeInt(source.storedLength());
                int[] filters = source.filters();
                output.writeInt(filters.length);
                for (int length : filters) {
                    output.writeInt(length);
                }
            } else {
                output.writeByte(2);
                int[][] extents = source.extents();
                output.writeInt(extents.length);
                for (int[] extent : extents) {
                    for (int value : extent) {
                        output.writeInt(value);
                    }
                }
            }
        }
        String[][] pools = plan.pools();
        output.writeInt(pools.length);
        for (String[] pool : pools) {
            output.writeInt(pool.length);
            for (String value : pool) {
                string(output, value, limits);
            }
        }
        output.writeInt(plan.requirements().size());
        for (Map.Entry<String, String> requirement : plan.requirements().entrySet()) {
            string(output, requirement.getKey(), limits);
            string(output, requirement.getValue(), limits);
        }
        output.writeInt(plan.roots().size());
        for (ResourcePlan.Root root : plan.roots()) {
            string(output, root.name(), limits);
            output.writeBoolean(root.module());
            output.writeInt(root.files().size());
            for (Map.Entry<String, ResourcePlan.File> entry : root.files().entrySet()) {
                ResourcePlan.File file = entry.getValue();
                string(output, entry.getKey(), limits);
                output.writeInt(file.source());
                if (file.source() >= 0) {
                    int[][] transforms = file.transforms();
                    output.writeInt(transforms.length);
                    for (int[] transform : transforms) {
                        output.writeInt(transform[0]);
                        output.writeInt(transform[1]);
                    }
                }
                BigInteger[] times = file.times();
                int flags = file.permissions() == null ? 0 : 8;
                for (int i = 0; i < times.length; i++) {
                    if (times[i] != null) {
                        flags |= 1 << i;
                    }
                }
                output.writeByte(flags);
                for (BigInteger time : times) {
                    if (time != null) {
                        byte[] raw = time.toByteArray();
                        byte[] padded = new byte[16];
                        Arrays.fill(padded, time.signum() < 0 ? (byte) -1 : 0);
                        System.arraycopy(raw, 0, padded, 16 - raw.length, raw.length);
                        output.write(padded);
                    }
                }
                if (file.permissions() != null) {
                    output.writeInt(file.permissions());
                }
            }
        }
        return buffer.bytes.toByteArray();
    }

    /// Writes exact UTF-16 units under the default per-value byte limit without closing the stream.
    public static void string(DataOutputStream output, String value) throws IOException {
        string(output, value, ReadLimits.DEFAULT);
    }

    /// Checks a string's encoded byte length before writing its count and UTF-16 units.
    private static void string(DataOutputStream output, String value, ReadLimits limits) throws IOException {
        limits.bytes(value.length() * 2L);
        output.writeInt(value.length());
        output.writeChars(value);
    }

    /// Checks private-index growth before accepting each write.
    private static final class IndexBuffer extends OutputStream {
        /// Accumulated bytes.
        private final ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        /// Inherited per-index size limit.
        private final ReadLimits limits;

        /// Creates an empty buffer governed by the selected resource policy.
        private IndexBuffer(ReadLimits limits) {
            this.limits = limits;
        }

        /// Appends one byte after checking the prospective size.
        @Override
        public void write(int value) throws IOException {
            limits.bytes((long) bytes.size() + 1);
            bytes.write(value);
        }

        /// Appends a valid byte range after checking the prospective size.
        @Override
        public void write(byte[] value, int offset, int length) throws IOException {
            if (offset < 0 || length < 0 || offset > value.length - length) {
                throw new IndexOutOfBoundsException();
            }
            limits.bytes((long) bytes.size() + length);
            bytes.write(value, offset, length);
        }
    }
}


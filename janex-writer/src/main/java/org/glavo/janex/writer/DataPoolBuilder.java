// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import org.glavo.janex.reader.ReadLimits;

/// Interns opaque byte sequences with stable indices and transactional append rollback.
final class DataPoolBuilder {
    /// Owned byte sequences in index order.
    private final List<byte[]> values = new ArrayList<>();
    /// Existing indices by exact bytes.
    private final Map<ByteBuffer, Integer> indices = new HashMap<>();
    /// Inherited limits on byte sequences and collections.
    private final ReadLimits limits;

    /// Creates a pool with the required empty byte sequence at index zero.
    DataPoolBuilder(ReadLimits limits) throws IOException {
        this.limits = limits;
        intern(new byte[0]);
    }

    /// Returns an existing index or stores an owned copy of a bounded byte sequence.
    int intern(byte[] value) throws IOException {
        Integer index = indices.get(ByteBuffer.wrap(value));
        if (index != null) return index;
        limits.bytes(value.length);
        limits.elements((long) values.size() + 1);
        index = values.size();
        byte[] copy = value.clone();
        values.add(copy);
        indices.put(ByteBuffer.wrap(copy), index);
        return index;
    }

    /// Returns an append checkpoint.
    int size() { return values.size(); }

    /// Removes entries appended since a valid checkpoint.
    void truncate(int size) {
        while (values.size() > size) indices.remove(ByteBuffer.wrap(values.remove(values.size() - 1)));
    }

    /// Encodes all entries in index order under the inherited byte limit.
    byte[] encode() throws IOException {
        Encoding output = new Encoding();
        output.uint(values.size());
        for (byte[] value : values) {
            output.sized(value);
            limits.bytes(output.size());
        }
        return output.toByteArray();
    }
}

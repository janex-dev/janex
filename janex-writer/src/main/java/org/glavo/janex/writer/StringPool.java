// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.writer;

import java.io.IOException;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import org.glavo.janex.reader.ReadLimits;

/// Interns lossless UTF-8 strings with stable indices and transactional append rollback.
final class StringPool {
    /// Strings in index order.
    private final List<String> values = new ArrayList<>();
    /// Existing indices by exact text.
    private final Map<String, Integer> indices = new HashMap<>();
    /// Inherited limits on strings and collections.
    private final ReadLimits limits;

    /// Creates a pool with the required empty string at index zero.
    StringPool(ReadLimits limits) throws IOException {
        this.limits = limits;
        intern("");
    }

    /// Returns an existing index or appends one bounded, well-formed string.
    int intern(String value) throws IOException {
        Integer index = indices.get(value);
        if (index != null) return index;
        limits.bytes(Encoding.utf8(value).length);
        limits.elements((long) values.size() + 1);
        index = values.size();
        values.add(value);
        indices.put(value, index);
        return index;
    }

    /// Returns an append checkpoint.
    int size() { return values.size(); }

    /// Removes strings appended since a valid checkpoint.
    void truncate(int size) {
        while (values.size() > size) indices.remove(values.remove(values.size() - 1));
    }

    /// Encodes all strings in index order under the inherited byte limit.
    byte[] encode() throws IOException {
        Encoding output = new Encoding();
        output.uint(values.size());
        for (String value : values) {
            output.sized(Encoding.utf8(value));
            limits.bytes(output.size());
        }
        return output.toByteArray();
    }
}

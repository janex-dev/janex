// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader.internal;

import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.zip.CRC32;
import java.util.zip.DataFormatException;
import java.util.zip.Inflater;

import static org.glavo.janex.reader.internal.Input.require;

/// Describes one stored or deflated entry in an unchanged, caller-owned JAR snapshot.
/// No file handle or decoded content is retained by this immutable descriptor.
public final class JarSource {
    /// Snapshot containing the compressed payload.
    private final Path path;
    /// Physical payload offset.
    private final long offset;
    /// Compressed payload length.
    private final int stored;
    /// Decoded payload length.
    private final int length;
    /// ZIP compression method, either stored (0) or deflated (8).
    private final int method;
    /// Unsigned CRC32 of the decoded payload.
    private final long checksum;

    /// Retains coordinates already validated against the archive directory and read limits.
    JarSource(Path path, long offset, int stored, int length, int method, long checksum) {
        this.path = path;
        this.offset = offset;
        this.stored = stored;
        this.length = length;
        this.method = method;
        this.checksum = checksum;
    }

    /// Returns the unchanged snapshot path, which must remain present while this source is used.
    public Path path() {
        return path;
    }

    /// Returns the decoded byte length.
    public int length() {
        return length;
    }

    /// Opens the snapshot, reads and verifies this entry, and closes the file before returning.
    /// @return independently owned decoded bytes
    /// @throws IOException if reading, deflate framing, length, or CRC validation fails
    public byte[] read() throws IOException {
        try (RandomAccessFile file = new RandomAccessFile(path.toFile(), "r")) {
            return read(file);
        }
    }

    /// Reads and verifies this entry through a caller-owned handle for [#path()].
    /// The handle remains open and its position advances to the end of the compressed payload.
    /// Callers must serialize use of the handle and keep the snapshot unchanged.
    /// @param file handle for the snapshot named by this descriptor
    /// @return independently owned decoded bytes
    /// @throws IOException if reading, deflate framing, length, or CRC validation fails
    public byte[] read(RandomAccessFile file) throws IOException {
        file.seek(offset);
        byte[] bytes = new byte[stored];
        file.readFully(bytes);
        return decode(bytes, 0, stored, length, method, checksum);
    }

    /// Decodes an exact ZIP payload and verifies its size, framing, and CRC32.
    static byte[] decode(byte[] bytes, int start, int stored, int decoded, int method, long crc) throws IOException {
        byte[] result;
        if (method == 0) {
            require(stored == decoded, "Stored JAR entry size mismatch");
            result = start == 0 && stored == bytes.length ? bytes : Arrays.copyOfRange(bytes, start, start + stored);
        } else {
            Inflater inflater = new Inflater(true);
            try {
                inflater.setInput(bytes, start, stored);
                result = new byte[decoded];
                int position = 0;
                while (position < decoded && !inflater.finished()) {
                    int produced = inflater.inflate(result, position, decoded - position);
                    require(produced > 0 || inflater.finished(), "Incomplete JAR deflate stream");
                    position += produced;
                }
                require(inflater.inflate(new byte[1]) == 0 && inflater.finished() && position == decoded && inflater.getRemaining() == 0,
                        "JAR deflate size or framing mismatch");
            } catch (DataFormatException failure) {
                throw new IOException("Invalid JAR deflate stream", failure);
            } finally {
                inflater.end();
            }
        }
        CRC32 checksum = new CRC32();
        checksum.update(result);
        require(checksum.getValue() == crc, "JAR entry CRC mismatch");
        return result;
    }
}

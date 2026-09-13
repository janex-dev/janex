// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.janex.reader;

import java.io.*;
import java.nio.ByteBuffer;
import java.nio.channels.SeekableByteChannel;
import java.nio.file.*;
import java.util.*;

/// Compares container framing, original encodings, and integrity reports with native results.
public final class ContainerTest {
    /// Prevents instantiation.
    private ContainerTest() {
    }

    /// Reads an owned byte string from trusted test data.
    private static byte[] bytes(DataInputStream input) throws IOException {
        byte[] value = new byte[input.readInt()];
        input.readFully(value);
        return value;
    }

    /// Runs all container vectors and checks channel ownership and verification boundaries.
    ///
    /// @param args oracle stream path and reusable snapshot path
    /// @throws Exception if source I/O fails or a native/Java comparison differs
    public static void main(String[] args) throws Exception {
        Path snapshot = Paths.get(args[1]);
        try (DataInputStream input = new DataInputStream(Files.newInputStream(Paths.get(args[0])))) {
            int count = input.readInt();
            for (int i = 0; i < count; i++) {
                Files.write(snapshot, bytes(input));
                long tail = input.readLong();
                boolean valid = input.readBoolean();
                TrackingChannel channel = new TrackingChannel(Files.newByteChannel(snapshot));
                ContainerReader reader;
                try {
                    reader = new ContainerReader(channel, tail);
                } catch (IOException failure) {
                    if (valid) {
                        throw new AssertionError("Valid container rejected at " + i, failure);
                    }
                    check(!channel.isOpen(), "Failed construction retained its channel");
                    continue;
                }
                try {
                    check(valid, "Malformed container accepted at " + i);
                    check(reader.start() == input.readLong() && reader.end() == input.readLong(), "Container range");
                    check(reader.physicalSize() == Files.size(snapshot), "Physical length");
                    byte[] metadata = bytes(input);
                    check(Arrays.equals(reader.metadata(), metadata), "Metadata encoding");
                    byte[] verificationInput = bytes(input);
                    check(Arrays.equals(reader.verificationInput(), verificationInput), "Original verification input");
                    int type = input.readUnsignedByte();
                    check(reader.verification().type().ordinal() == type, "Verification type");
                    byte[] payload = bytes(input);
                    check(Arrays.equals(reader.verification().payload(), payload), "Verification payload");
                    // Returned byte arrays must not change subsequent reads or integrity verification.
                    Arrays.fill(reader.metadata(), (byte) 0);
                    Arrays.fill(reader.verificationInput(), (byte) 0);
                    Arrays.fill(reader.verification().payload(), (byte) 0);
                    check(Arrays.equals(reader.metadata(), metadata), "Aliased metadata");
                    check(Arrays.equals(reader.verificationInput(), verificationInput), "Aliased verification input");
                    check(Arrays.equals(reader.verification().payload(), payload), "Aliased payload");
                    int sections = input.readInt();
                    check(reader.sections().size() == sections, "Section count");
                    for (ContainerReader.Section section : reader.sections()) {
                        check(section.id() == input.readLong() && section.type() == input.readLong(), "Section identity");
                        check(section.length() == input.readLong() && section.offset() == input.readLong(), "Section range");
                        byte[] sectionMetadata = bytes(input);
                        check(Arrays.equals(section.metadata(), sectionMetadata), "Section metadata encoding");
                        boolean info = input.readBoolean();
                        byte[] typeInfo = info ? bytes(input) : null;
                        check(Arrays.equals(section.typeInfo(), typeInfo), "Type-info encoding");
                        boolean digest = input.readBoolean();
                        byte[] expectedDigest = digest ? bytes(input) : null;
                        check(Arrays.equals(section.checksum() == null ? null : section.checksum().encode(), expectedDigest), "Section checksum");
                        byte[] body = bytes(input);
                        check(Arrays.equals(reader.readSection(section.id()), body), "Section body");
                        check(reader.section(section.id()) == section, "Section lookup");
                        check(reader.readSectionRange(section.id(), body.length, 0).length == 0, "Empty end range");
                        int middle = body.length / 2;
                        check(Arrays.equals(reader.readSectionRange(section.id(), middle, body.length - middle),
                                Arrays.copyOfRange(body, middle, body.length)), "Section-relative range");
                        reject(() -> reader.readSectionRange(section.id(), -1, 0));
                        reject(() -> reader.readSectionRange(section.id(), 0, -1));
                        reject(() -> reader.readSectionRange(section.id(), Long.MAX_VALUE, 1));
                        reject(() -> reader.readSectionRange(section.id(), body.length, 1));
                        Arrays.fill(section.metadata(), (byte) 0);
                        if (info) {
                            Arrays.fill(section.typeInfo(), (byte) 0);
                        }
                        check(Arrays.equals(section.metadata(), sectionMetadata) && Arrays.equals(section.typeInfo(), typeInfo), "Aliased section metadata");
                    }
                    boolean verified = input.readBoolean();
                    int verifiedCount = verified ? input.readInt() : 0;
                    boolean complete = verified && input.readBoolean();
                    channel.maximumRead = 0;
                    channel.bytesRead = 0;
                    try {
                        ContainerReader.IntegrityReport report = reader.verifyChecksums();
                        check(verified, "Corruption accepted at " + i);
                        check(report.checksumsVerified() == verifiedCount && report.completeSecureCoverage() == complete, "Integrity report");
                        long read = channel.bytesRead;
                        reader.verifyChecksums();
                        check(channel.bytesRead == read * 2, "Container reader cached a checksum scan");
                    } catch (IOException failure) {
                        if (verified) {
                            throw new AssertionError("Valid integrity rejected at " + i, failure);
                        }
                    }
                    check(channel.maximumRead <= 32768, "Unbounded integrity buffer");
                    reader.close();
                    reader.close();
                    check(!channel.isOpen() && Arrays.equals(reader.metadata(), metadata), "Close ownership or retained metadata");
                    reject(reader::verifyChecksums);
                    if (!reader.sections().isEmpty()) {
                        reject(() -> reader.readSection(reader.sections().get(0).id()));
                    }
                } finally {
                    reader.close();
                }
            }
            check(input.read() == -1, "Trailing container fixtures");
            ownership(snapshot);
            System.out.println("Verified " + count + " container vectors");
        }
    }

    /// Checks that source exceptions retain identity and failed constructors close their sources.
    private static void ownership(Path path) throws Exception {
        TrackingChannel channel = new TrackingChannel(Files.newByteChannel(path));
        channel.failure = new IOException("fixture read failure");
        try {
            new ContainerReader(channel, 0);
            throw new AssertionError("Source failure ignored");
        } catch (IOException failure) {
            check(failure == channel.failure && !channel.isOpen(), "Source failure or close ownership lost");
        }
    }

    /// Requires a test invariant.
    private static void check(boolean value, String message) {
        if (!value) {
            throw new AssertionError(message);
        }
    }

    /// Requires a source or format operation to fail with an I/O diagnostic.
    private static void reject(Action action) throws Exception {
        try {
            action.run();
        } catch (IOException expected) {
            return;
        }
        throw new AssertionError("Invalid container operation accepted");
    }

    /// Runs one operation whose result is irrelevant to its failure contract.
    private interface Action {
        /// Performs the checked operation.
        void run() throws Exception;
    }

    /// Observes buffering and ownership without changing source contents.
    private static final class TrackingChannel implements SeekableByteChannel {
        /// Owned file channel.
        private final SeekableByteChannel delegate;
        /// Largest destination-buffer remainder observed during reads.
        int maximumRead;
        /// Successfully read byte count.
        long bytesRead;
        /// Optional exact exception to throw instead of reading.
        IOException failure;

        /// Takes ownership of the fixture's source channel.
        TrackingChannel(SeekableByteChannel delegate) {
            this.delegate = delegate;
        }

        /// Records the requested buffer size and delegates reads or injects the selected failure.
        @Override
        public int read(ByteBuffer buffer) throws IOException {
            if (failure != null) {
                throw failure;
            }
            maximumRead = Math.max(maximumRead, buffer.remaining());
            int count = delegate.read(buffer);
            bytesRead += Math.max(0, count);
            return count;
        }

        /// Rejects writes to this read-only fixture.
        @Override
        public int write(ByteBuffer buffer) {
            throw new UnsupportedOperationException();
        }

        /// Returns the underlying source position.
        @Override
        public long position() throws IOException {
            return delegate.position();
        }

        /// Repositions the underlying source and returns this wrapper.
        @Override
        public SeekableByteChannel position(long position) throws IOException {
            delegate.position(position);
            return this;
        }

        /// Returns the physical fixture length.
        @Override
        public long size() throws IOException {
            return delegate.size();
        }

        /// Rejects truncation of this read-only fixture.
        @Override
        public SeekableByteChannel truncate(long size) {
            throw new UnsupportedOperationException();
        }

        /// Returns whether the owned delegate remains open.
        @Override
        public boolean isOpen() {
            return delegate.isOpen();
        }

        /// Closes the owned delegate.
        @Override
        public void close() throws IOException {
            delegate.close();
        }
    }
}

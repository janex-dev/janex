// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.*;
import java.nio.ByteBuffer;
import java.nio.channels.SeekableByteChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;

import org.glavo.janex.reader.internal.Conditions;
import org.glavo.janex.reader.internal.Input;
import org.glavo.janex.reader.internal.ZipDirectory;

import static org.glavo.janex.reader.internal.Input.*;

/// Reads Janex container framing and recorded integrity independently of publisher authentication.
///
/// Opening validates metadata, lengths, identities, and known section magic. It does not verify
/// checksums, parse section bodies, or validate signatures. The caller must keep the source bytes
/// unchanged while using this reader or retaining a verification result. Operations reposition
/// the owned channel and may leave its position changed after failure. Instances are not thread-safe.
/// Closing is idempotent; metadata remains available after closure, but I/O operations fail.
public final class ContainerReader implements Closeable {
    /// Owned source channel.
    private final SeekableByteChannel source;
    /// Policy shared by framing, metadata, and buffered section reads.
    private final ReadLimits limits;
    /// Physical source length observed when opened.
    private final long physicalSize;
    /// Parsed framing and original metadata encodings.
    private final Frame frame;
    /// Whether the owned channel has been closed.
    private boolean closed;

    /// Opens a file and discovers a standalone footer or a footer before a JAR tail.
    ///
    /// @param path source file, kept unchanged by the caller
    /// @throws IOException if opening, framing, metadata, or boundary discovery fails
    public ContainerReader(Path path) throws IOException {
        this(Files.newByteChannel(path), -1);
    }

    /// Opens a file using the exact external-tail length without interpreting that tail.
    ///
    /// @param path source file, kept unchanged by the caller
    /// @param externalTailLength tail byte length, or -1 to discover a standalone or JAR boundary
    /// @throws IOException if opening, the supplied length, or container framing is invalid
    public ContainerReader(Path path, long externalTailLength) throws IOException {
        this(Files.newByteChannel(path), externalTailLength);
    }

    /// Opens a file with explicit boundary discovery and allocation policy.
    ///
    /// @param path source file, kept unchanged by the caller
    /// @param externalTailLength tail byte length, or -1 for automatic discovery
    /// @param limits nonnull policy for buffered values, collections, and nesting
    /// @throws IOException if opening, parsing, or the supplied limits reject the container
    public ContainerReader(Path path, long externalTailLength, ReadLimits limits) throws IOException {
        this(Files.newByteChannel(path), externalTailLength, limits);
    }

    /// Takes ownership of a channel and parses its container framing.
    ///
    /// The channel is closed if construction fails. Automatic discovery accepts ordinary and
    /// ZIP64 JAR tails and rejects ambiguous boundaries. Reads follow the channel's blocking and
    /// interruption behavior; closing this reader closes the channel.
    ///
    /// @param source readable, seekable channel whose contents must remain unchanged
    /// @param externalTailLength tail byte length, or -1 for automatic discovery
    /// @throws IOException if source I/O, boundary discovery, or framing validation fails
    public ContainerReader(SeekableByteChannel source, long externalTailLength) throws IOException {
        this(source, externalTailLength, ReadLimits.DEFAULT);
    }

    /// Takes ownership of a channel and parses framing under the supplied policy.
    ///
    /// Ownership, closure, and I/O behavior are the same as [#ContainerReader(SeekableByteChannel, long)].
    /// The channel is also closed if the policy is null.
    ///
    /// @param source readable, seekable channel whose contents must remain unchanged
    /// @param externalTailLength tail byte length, or -1 for automatic discovery
    /// @param limits nonnull policy inherited by nested decoding and buffered reads
    /// @throws IOException if source I/O, discovery, framing, or a resource limit fails
    public ContainerReader(SeekableByteChannel source, long externalTailLength, ReadLimits limits) throws IOException {
        this.source = Objects.requireNonNull(source);
        try {
            this.limits = Objects.requireNonNull(limits);
            physicalSize = source.size();
            require(externalTailLength >= -1 && externalTailLength <= physicalSize, "Invalid external-tail length");
            frame = externalTailLength == -1 ? discover() : parse(physicalSize - externalTailLength);
        } catch (Throwable failure) {
            try {
                source.close();
            } catch (IOException close) {
                failure.addSuppressed(close);
            }
            throw failure;
        }
    }

    /// Returns the physical offset of the Janex magic, excluding the external header.
    public long start() {
        return frame.start;
    }

    /// Returns the physical offset immediately after the Janex footer, before the external tail.
    public long end() {
        return frame.end;
    }

    /// Returns the physical source length observed during construction.
    public long physicalSize() {
        return physicalSize;
    }

    /// Returns the immutable policy used by this reader, including after closure.
    public ReadLimits limits() {
        return limits;
    }

    /// Returns an owned copy of the original file-metadata CBOR map, excluding its Sized prefix.
    ///
    /// Unknown fields and their exact encodings are retained. This is not the complete
    /// verification input; use [#verificationInput()] for checksum or signature verification.
    public byte[] metadata() {
        return frame.metadata.clone();
    }

    /// Returns an immutable list of section descriptors in physical order.
    public List<Section> sections() {
        return frame.sections;
    }

    /// Returns a section descriptor by its opaque unsigned 64-bit identifier.
    ///
    /// @param id all 64 identifier bits, including values represented by negative longs
    /// @return the matching immutable descriptor
    /// @throws IOException if the identifier is not present
    public Section section(long id) throws IOException {
        Section section = frame.byId.get(id);
        require(section != null, "Missing section " + Long.toUnsignedString(id));
        return section;
    }

    /// Returns the immutable verification declaration without asserting signature validity or trust.
    public Verification verification() {
        return frame.verification;
    }

    /// Returns an owned copy of the exact bytes from metadata magic through the verification type.
    ///
    /// The original Sized encoding is retained. Callers must verify these bytes directly rather
    /// than re-encoding decoded metadata. The signature payload is not included.
    public byte[] verificationInput() {
        return frame.verificationInput.clone();
    }

    /// Reads a complete section without verifying its checksum or decoding its body.
    ///
    /// @param id opaque unsigned 64-bit section identifier
    /// @return owned encoded bytes, including section magic when present
    /// @throws IOException if closed, missing, larger than the buffered byte limit, or unreadable
    public byte[] readSection(long id) throws IOException {
        return readSectionRange(id, 0, section(id).length);
    }

    /// Reads a bounded range of encoded section bytes without authenticating them.
    ///
    /// @param id opaque unsigned 64-bit section identifier
    /// @param offset nonnegative section-relative byte offset
    /// @param length nonnegative byte count within the section and buffered byte limit
    /// @return owned bytes; an empty range at the section end is valid
    /// @throws IOException if closed, the range or identifier is invalid, or source I/O fails
    public byte[] readSectionRange(long id, long offset, long length) throws IOException {
        Section section = section(id);
        require(offset >= 0 && length >= 0 && offset <= section.length - length, "Range exceeds section");
        return read(section.offset + offset, length);
    }

    /// Verifies the metadata checksum, when present, and every recorded section and external-region checksum.
    ///
    /// Section and region data are streamed with bounded buffering. No result is cached, and
    /// missing checksums do not cause this operation to fail. Signature payloads are not verified.
    /// A caller retaining the report must ensure the source remains unchanged.
    ///
    /// @return counts and secure-coverage information, without any assertion of publisher trust
    /// @throws IOException if closed, a recorded checksum does not match, or source I/O fails
    public IntegrityReport verifyChecksums() throws IOException {
        checkOpen();
        if (frame.verification.type == Verification.Type.CHECKSUM) {
            Checksum.decode(frame.verification.payload).verify(frame.verificationInput);
        }
        int count = 0;
        boolean complete = true;
        for (Section section : frame.sections) {
            if (section.checksum == null) {
                complete = false;
            } else {
                verifyRange(section.checksum, section.offset, section.length);
                count++;
                complete &= section.checksum.algorithm().isSecure();
            }
        }
        for (int key = 1; key <= 2; key++) {
            long offset = key == 1 ? 0 : frame.end;
            long length = key == 1 ? frame.start : physicalSize - frame.end;
            if (!has(frame.values, key)) {
                complete = false;
                continue;
            }
            Map<Object, Object> region = map(get(frame.values, key));
            if (has(region, 1)) {
                Checksum checksum = Checksum.decode(binary(get(region, 1)));
                verifyRange(checksum, offset, length);
                count++;
                complete &= length == 0 || checksum.algorithm().isSecure();
            } else {
                complete &= length == 0;
            }
        }
        return new IntegrityReport(count, complete);
    }

    /// Closes the owned source channel; repeated calls have no effect.
    ///
    /// @throws IOException if the underlying close fails; this reader remains closed
    @Override
    public void close() throws IOException {
        if (!closed) {
            closed = true;
            source.close();
        }
    }

    /// Rejects source operations after closure.
    private void checkOpen() throws IOException {
        require(!closed, "Janex container reader is closed");
    }

    /// Reads an exact bounded physical range for format-layer consumers.
    byte[] read(long offset, long length) throws IOException {
        checkOpen();
        int size = limits.bytes(length);
        require(offset >= 0 && offset <= physicalSize - size, "Janex range exceeds snapshot");
        byte[] result = new byte[size];
        source.position(offset);
        readFully(ByteBuffer.wrap(result));
        return result;
    }

    /// Fills a buffer using the owned channel's blocking behavior.
    private void readFully(ByteBuffer buffer) throws IOException {
        while (buffer.hasRemaining()) {
            if (source.read(buffer) < 0) {
                throw new EOFException("Truncated Janex source");
            }
        }
    }

    /// Verifies one exact encoded range with fixed-size buffering.
    private void verifyRange(Checksum checksum, long offset, long length) throws IOException {
        MessageDigest digest = checksum.algorithm().newDigest();
        source.position(offset);
        ByteBuffer buffer = ByteBuffer.allocate((int) Math.min(length, 32768));
        while (length != 0) {
            buffer.clear();
            buffer.limit((int) Math.min(length, buffer.capacity()));
            readFully(buffer);
            length -= buffer.position();
            buffer.flip();
            digest.update(buffer);
        }
        checksum.verifyDigest(digest.digest());
    }

    /// Finds one valid container boundary, propagating I/O and resource-limit failures.
    private Frame discover() throws IOException {
        Set<Long> ends = new TreeSet<Long>();
        ends.add(physicalSize);
        for (ZipDirectory directory : ZipDirectory.find(physicalSize, this::read, limits)) {
            ends.add(directory.base());
        }
        Frame result = null;
        for (long end : ends) {
            Frame candidate;
            try {
                candidate = parse(end);
            } catch (Invalid invalid) {
                continue;
            }
            require(result == null, "Ambiguous Janex boundary");
            result = candidate;
        }
        require(result != null, "No Janex footer before a valid JAR tail");
        return result;
    }

    /// Parses framing and common schemas without touching section bodies or verifying digests.
    private Frame parse(long end) throws IOException {
        require(end >= 24, "Missing Janex footer");
        Input footer = new Input(read(end - 24, 24), limits);
        require(Arrays.equals(footer.take(8), "JANEXEND".getBytes(StandardCharsets.US_ASCII)), "Invalid Janex end marker");
        long metadataLength = footer.little(8);
        long length = footer.little(8);
        require(length >= 32 && metadataLength >= 24 && metadataLength <= length - 8 && length <= end,
                "Invalid Janex footer lengths");
        long start = end - length;
        require(Arrays.equals(read(start, 8), new byte[]{'J', 'A', 'N', 'E', 'X', 0, 0, 0}), "Invalid Janex magic");
        Input metadata = new Input(read(end - metadataLength, metadataLength - 24), limits);
        require(Arrays.equals(metadata.take(8), "METADATA".getBytes(StandardCharsets.US_ASCII)), "Invalid metadata magic");
        if (metadata.little(4) != 0 || metadata.little(4) != 1) {
            throw new IOException("Unsupported Janex version");
        }
        byte[] raw = metadata.sized();
        require(raw.length != 0, "Missing section table");
        Input cbor = new Input(raw, true, limits);
        Map<Object, Object> values = map(cbor.cbor(0));
        cbor.end();
        require(!values.isEmpty(), "Empty sized CBOR map must use zero length");
        for (Object key : values.keySet()) {
            if (key instanceof String) {
                Conditions.nonempty(key);
            } else {
                number(key);
            }
        }
        for (int key = 3; key <= 4; key++) {
            if (has(values, key)) {
                Conditions.nonempty(get(values, key));
            }
        }
        int type = metadata.u8();
        byte[] verificationInput = metadata.prefix();
        Verification verification = new Verification(type, metadata.sized());
        metadata.end();
        for (int key = 1; key <= 2; key++) {
            if (has(values, key)) {
                Map<Object, Object> region = integers(map(get(values, key)));
                require(number(get(region, 0)) == (key == 1 ? start : physicalSize - end), "External region length mismatch");
                if (has(region, 1)) {
                    Checksum.decode(binary(get(region, 1)));
                }
            }
        }
        List<Section> sections = new ArrayList<Section>();
        Map<Long, Section> byId = new LinkedHashMap<Long, Section>();
        long offset = start + 8;
        for (Object item : list(get(values, 0))) {
            Map<Object, Object> value = integers(map(item));
            Section section = new Section(value, cbor, offset);
            require(section.length >= 0 && section.length <= end - metadataLength - offset, "Section exceeds Janex body");
            require(byId.put(section.id, section) == null, "Duplicate section ID");
            if (section.type == 0x4c4f4f50424f4c42L || section.type == 0x50504158454e414aL) {
                require(section.length >= 8 && new Input(read(offset, 8), limits).little(8) == section.type, "Incorrect section magic");
            }
            sections.add(section);
            offset += section.length;
        }
        require(offset == end - metadataLength, "Section lengths do not cover Janex body");
        return new Frame(start, end, raw, values, sections, byId, verification, verificationInput);
    }

    /// Retains one structurally validated container candidate.
    private static final class Frame {
        /// Physical Janex start.
        final long start;
        /// Physical Janex end.
        final long end;
        /// Original CBOR metadata bytes.
        final byte[] metadata;
        /// Decoded private metadata used for region checks.
        final Map<Object, Object> values;
        /// Immutable sections in physical order.
        final List<Section> sections;
        /// Section lookup by opaque identifier.
        final Map<Long, Section> byId;
        /// Parsed verification declaration.
        final Verification verification;
        /// Original signature or metadata-checksum input.
        final byte[] verificationInput;

        /// Takes ownership of one parsed candidate's metadata and descriptors.
        Frame(long start, long end, byte[] metadata, Map<Object, Object> values, List<Section> sections,
              Map<Long, Section> byId, Verification verification, byte[] verificationInput) {
            this.start = start;
            this.end = end;
            this.metadata = metadata;
            this.values = values;
            this.sections = Collections.unmodifiableList(sections);
            this.byId = byId;
            this.verification = verification;
            this.verificationInput = verificationInput;
        }
    }

    /// Describes a section without interpreting its body or asserting integrity.
    public static final class Section {
        /// Opaque unsigned 64-bit identifier.
        private final long id;
        /// Unsigned 64-bit section type.
        private final long type;
        /// Encoded byte count including any section magic.
        private final long length;
        /// Physical offset of the first section byte.
        private final long offset;
        /// Optional recorded digest.
        private final Checksum checksum;
        /// Exact section-info CBOR map.
        private final byte[] metadata;
        /// Exact type-specific CBOR map, or null if absent.
        private final byte[] typeInfo;
        /// Decoded type-specific metadata for internal format consumers, or null if absent.
        final Map<Object, Object> info;

        /// Validates common fields and retains original metadata bytes.
        private Section(Map<Object, Object> value, Input cbor, long offset) throws IOException {
            id = number(get(value, 1));
            type = number(get(value, 0));
            length = number(get(value, 2));
            this.offset = offset;
            checksum = has(value, 3) ? Checksum.decode(binary(get(value, 3))) : null;
            info = has(value, 4) ? integers(map(get(value, 4))) : null;
            metadata = cbor.encodedMap(value);
            typeInfo = info == null ? null : cbor.encodedMap(info);
        }

        /// Returns all 64 bits of the unsigned section identifier.
        public long id() {
            return id;
        }

        /// Returns all 64 bits of the unsigned section type.
        public long type() {
            return type;
        }

        /// Returns the section's encoded byte length, including any magic.
        public long length() {
            return length;
        }

        /// Returns the section's zero-based physical byte offset.
        public long offset() {
            return offset;
        }

        /// Returns the immutable recorded checksum, or null when absent.
        public Checksum checksum() {
            return checksum;
        }

        /// Returns an owned copy of the exact SectionInfoObject CBOR encoding, including extensions.
        public byte[] metadata() {
            return metadata.clone();
        }

        /// Returns an owned copy of the exact type-info CBOR map, or null when absent.
        public byte[] typeInfo() {
            return typeInfo == null ? null : typeInfo.clone();
        }
    }

    /// Retains a verification declaration without claiming that its payload has been authenticated.
    public static final class Verification {
        /// Verification mechanisms defined by Janex 0.1.
        public enum Type {
            /// No verification payload.
            NONE,
            /// A checksum over the exact verification input.
            CHECKSUM,
            /// A detached binary OpenPGP signature, requiring external profile and trust validation.
            OPENPGP,
            /// A detached DER CMS signature, requiring external profile and trust validation.
            CMS
        }

        /// Mechanism identified by the verification type byte.
        private final Type type;
        /// Original payload bytes, excluding their Sized prefix.
        private final byte[] payload;

        /// Validates payload framing without parsing signature internals.
        private Verification(int type, byte[] payload) throws IOException {
            require(type >= 0 && type <= 3, "Unsupported Janex verification type");
            this.type = Type.values()[type];
            this.payload = payload;
            if (this.type == Type.NONE) {
                require(payload.length == 0, "None verification payload must be empty");
            } else if (this.type == Type.CHECKSUM) {
                Checksum.decode(payload);
            } else {
                require(payload.length != 0, "Empty signature payload");
            }
        }

        /// Returns the declared mechanism, without asserting successful verification.
        public Type type() {
            return type;
        }

        /// Returns an owned copy of the exact payload bytes, empty for None.
        public byte[] payload() {
            return payload.clone();
        }
    }

    /// Reports recorded content integrity without asserting metadata authentication or publisher trust.
    public static final class IntegrityReport {
        /// Number of checked section and external-region digests, excluding metadata.
        private final int checksumsVerified;
        /// Whether secure digests cover every section and both external regions are constrained.
        private final boolean completeSecureCoverage;

        /// Retains the result of one complete checksum scan.
        private IntegrityReport(int checksumsVerified, boolean completeSecureCoverage) {
            this.checksumsVerified = checksumsVerified;
            this.completeSecureCoverage = completeSecureCoverage;
        }

        /// Returns the number of checked section and external-region digests, excluding metadata.
        public int checksumsVerified() {
            return checksumsVerified;
        }

        /// Returns whether every section and external region has the required secure content coverage.
        ///
        /// Both external regions must be declared, including empty ones. Empty regions may omit
        /// a digest. This result does not imply that metadata is authenticated or a signer is trusted.
        public boolean completeSecureCoverage() {
            return completeSecureCoverage;
        }
    }
}

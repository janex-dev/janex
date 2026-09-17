// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.reader;

import java.io.*;
import java.nio.file.Path;
import java.time.Instant;
import java.util.*;

import org.glavo.janex.reader.internal.Conditions;
import org.glavo.janex.reader.internal.Input;
import org.glavo.janex.reader.internal.JarArchive;
import org.glavo.janex.reader.internal.JarSource;
import org.glavo.janex.reader.internal.ModuleRequirement;
import org.glavo.janex.reader.internal.codec.ZstandardFrames;

import static org.glavo.janex.reader.internal.Input.*;

/// Reads a Janex 0.1 snapshot independently of the native Host.
///
/// Constructors verify every recorded container checksum before interpreting resources. Signed
/// packages require a caller-supplied authentication policy and complete secure content coverage.
/// Use [ContainerReader] to inspect a container without preparing it for execution. Ordinary resource payloads
/// remain in the snapshot; dictionary-backed sources are decoded during launch preparation.
/// The caller must keep the snapshot unchanged until the launched application exits.
/// The static prepareResources entry point instead consumes snapshots already verified by its caller.
/// Instances are not thread-safe. Launch selection is single-use; closure is idempotent.
public final class JanexReader implements Closeable {
    /// Owned container over the caller's immutable snapshot.
    private final ContainerReader container;
    /// Policy shared by nested decoding and launch-resource preparation.
    private final ReadLimits limits;
    /// Snapshot path encoded into the private resource index.
    private final Path path;
    /// Complete integrity scan result; null only for internally prepared snapshots.
    private final ContainerReader.IntegrityReport integrity;
    /// Caller-supplied Zstandard decoder, shared with the runtime resource reader.
    private final BlobDecoder decoder;
    /// Optional acquisition policy for external JARs, never owned or closed by this reader.
    private final DependencyResolver resolver;
    /// Pool sections whose type-specific page directories are interpreted on first reference.
    private final Map<Long, ContainerReader.Section> poolSections = new LinkedHashMap<Long, ContainerReader.Section>();
    /// Opened pools indexed by their opaque section IDs.
    private final Map<Long, Pool> pools = new LinkedHashMap<Long, Pool>();
    /// Validated application descriptors in section order.
    private final List<Application> applications = new ArrayList<Application>();
    /// Sources in dependency order, shared by all selected roots.
    private final List<Source> sources = new ArrayList<Source>();
    /// Decoded data pools in selection order.
    private final List<DataPool> dataPools = new ArrayList<DataPool>();
    /// Interned pool references.
    private final Map<List<Long>, Integer> dataPoolIds = new HashMap<List<Long>, Integer>();
    /// Aggregate logical resource size selected for this launch.
    private long logicalBytes;
    /// Optional Host-selected condition context, independent of overridable JVM properties.
    private String[] context;
    /// Aggregate expanded resource allowance; ordinary readers use their byte limit.
    private long logicalLimit;
    /// Whether prepared resources retain their root directory for filesystem lookup.
    private boolean includeRoot;
    /// Whether launch selection has started, including a failed attempt.
    private boolean selected;
    /// Whether the snapshot handle has been closed.
    private boolean closed;

    /// Opens and checks an immutable snapshot using the reader's portable Zstandard decoder.
    ///
    /// @param path snapshot kept unchanged until consumers finish reading its selected resources
    /// @throws IOException if framing, recorded integrity, or the supported profile is invalid
    public JanexReader(Path path) throws IOException {
        this(path, JanexReader::decode);
    }

    /// Opens an immutable snapshot with the portable decoder and a caller-owned acquisition policy.
    ///
    /// @param path snapshot kept unchanged until consumers finish reading its selected resources
    /// @param resolver external dependency policy, or null to reject selected external entries
    /// @throws IOException if framing, recorded integrity, or the supported profile is invalid
    public JanexReader(Path path, DependencyResolver resolver) throws IOException {
        this(path, JanexReader::decode, resolver);
    }

    /// Decodes complete frames into an exactly sized owned buffer.
    private static byte[] decode(byte[] input, int length, byte[] dictionary) throws IOException {
        byte[] output = new byte[length];
        try {
            int written = org.glavo.janex.reader.internal.codec.zstd.Zstandard.decompress(
                    input, 0, input.length, output, 0, length, dictionary);
            require(written == length, "Zstd decoded length mismatch");
            return output;
        } catch (RuntimeException failure) {
            throw new IOException("Invalid Zstd content", failure);
        }
    }

    /// Opens and checks one immutable snapshot, closing the handle if validation fails.
    ///
    /// @param path local snapshot whose lifetime is controlled by the caller
    /// @param decoder Zstandard decoder producing exactly the requested number of bytes
    /// @throws IOException if framing, recorded integrity, or the supported profile is invalid
    public JanexReader(Path path, BlobDecoder decoder) throws IOException {
        this(path, decoder, null);
    }

    /// Opens a snapshot with a caller-supplied external dependency policy.
    ///
    /// Only None and Checksum verification are permitted. Signed packages require the constructor
    /// accepting an [AuthenticationPolicy]. Every recorded container checksum is verified once.
    ///
    /// @param path local snapshot kept unchanged until the application exits
    /// @param decoder decoder producing exactly the requested number of bytes
    /// @param resolver resolver called only for selected external entries, or null to reject them
    /// @throws IOException if opening or validating the snapshot fails
    public JanexReader(Path path, BlobDecoder decoder, DependencyResolver resolver) throws IOException {
        this(path, decoder, resolver, null);
    }

    /// Opens a snapshot under a caller-supplied authentication and dependency policy.
    ///
    /// Authentication runs after framing validation and before content verification, section-body
    /// interpretation, or dependency acquisition. A signed package must also have complete secure content coverage.
    /// Failure closes the owned reader and never falls back to unsigned preparation.
    ///
    /// @param path snapshot kept unchanged until all returned resources are no longer used
    /// @param decoder Zstandard decoder producing exactly the requested number of bytes
    /// @param resolver selected external-JAR resolver, or null to reject external dependencies
    /// @param policy authentication policy, or null to allow only None and Checksum verification
    /// @throws IOException if parsing, authentication, integrity, or supported section schemas fail
    public JanexReader(Path path, BlobDecoder decoder, DependencyResolver resolver, AuthenticationPolicy policy) throws IOException {
        this(path, decoder, resolver, policy, ReadLimits.DEFAULT);
    }

    /// Opens and verifies a snapshot with explicit decoding and preparation limits.
    ///
    /// Authentication and ownership follow the constructor accepting [AuthenticationPolicy].
    /// The policy also bounds decoded blobs, imported JAR data, expanded resources, and private
    /// index output. Dependency resolvers and decoders must impose their own acquisition and
    /// working-memory bounds; this reader checks their returned values before further use.
    ///
    /// @param path snapshot kept unchanged until all returned resources are no longer used
    /// @param decoder Zstandard decoder producing exactly the requested number of bytes
    /// @param resolver selected external-JAR resolver, or null to reject external dependencies
    /// @param policy authentication policy, or null to allow only None and Checksum verification
    /// @param limits nonnull policy inherited by nested decoding and preparation
    /// @throws IOException if opening, authentication, validation, or a resource limit fails
    public JanexReader(Path path, BlobDecoder decoder, DependencyResolver resolver,
                       AuthenticationPolicy policy, ReadLimits limits) throws IOException {
        this.limits = Objects.requireNonNull(limits);
        this.logicalLimit = limits.maxBytes();
        this.path = path;
        this.decoder = Objects.requireNonNull(decoder);
        this.resolver = resolver;
        container = new ContainerReader(path, -1, limits);
        try {
            ContainerReader.Verification verification = container.verification();
            boolean signed = verification.type() == ContainerReader.Verification.Type.OPENPGP
                    || verification.type() == ContainerReader.Verification.Type.CMS;
            if (policy == null) {
                require(!signed, "Signed packages require a caller authentication policy or Janex Host");
            } else {
                policy.authenticate(verification, container.verificationInput());
            }
            integrity = container.verifyChecksums();
            require(!signed || integrity.completeSecureCoverage(), "Signed execution requires secure checksums covering the complete container");
            loadSections();
        } catch (Throwable failure) {
            try {
                container.close();
            } catch (IOException close) {
                failure.addSuppressed(close);
            }
            throw failure;
        }
    }

    /// Opens only pool directories from a snapshot already verified by the launch preparer.
    private JanexReader(Path path, ReadLimits limits, String[] context, long logicalLimit) throws IOException {
        this.path = Objects.requireNonNull(path);
        this.limits = Objects.requireNonNull(limits);
        this.context = context == null ? null : context.clone();
        this.logicalLimit = logicalLimit;
        this.decoder = JanexReader::decode;
        this.resolver = null;
        this.integrity = null;
        this.includeRoot = true;
        container = new ContainerReader(path, -1, limits);
        try {
            for (ContainerReader.Section section : container.sections()) {
                if (section.type() == 0x4c4f4f50424f4c42L) {
                    poolSections.put(section.id(), section);
                }
            }
        } catch (Throwable failure) {
            try {
                container.close();
            } catch (IOException close) {
                failure.addSuppressed(close);
            }
            throw failure;
        }
    }

    /// Prepares selected resources from caller-verified, unchanged snapshots.
    ///
    /// This method does not authenticate publishers or repeat the container-integrity scan.
    /// The caller must verify the package and each external JAR under its launch policy before
    /// calling, and retain the same snapshots unchanged while their resources are used.
    /// External JAR payloads remain deferred except for manifests and symbolic-link targets;
    /// payload framing and CRC failures can therefore occur when consumers read a resource.
    /// Format parsing and decoding limits still apply. No application code is executed.
    ///
    /// @param path verified container snapshot
    /// @param requests selected roots in lookup order
    /// @param requirements module names and optional exact versions (empty means unconstrained)
    /// @param context OS, architecture, invocation, Java version, and vendor; null uses this JVM
    /// @param limits bounds for individual values and collections
    /// @param logicalLimit nonnegative aggregate expanded resource byte allowance
    /// @return immutable resource descriptions independent of the closed preparation reader
    /// @throws IOException if a snapshot, resource description, or resource limit is invalid
    public static ResourcePlan prepareResources(Path path, List<ResourceRequest> requests,
            Map<String, String> requirements, String[] context, ReadLimits limits, long logicalLimit) throws IOException {
        require(context == null || context.length == 5, "Invalid runtime context");
        require(logicalLimit >= 0, "Invalid logical resource limit");
        limits.elements(requests.size());
        try (JanexReader reader = new JanexReader(path, limits, context, logicalLimit)) {
            List<Root> roots = new ArrayList<Root>();
            for (ResourceRequest request : requests) {
                if (request.path == null) {
                    roots.add(reader.root(reader.reference(request.pool, request.index), request.module, false));
                } else {
                    roots.add(reader.jarRoot(request.jarName, JarArchive.index(request.path, limits), request.module, false));
                }
            }
            reader.prepareDictionaries();
            return reader.resources(roots, requirements);
        }
    }

    /// Returns the physical offset of the snapshot's external tail.
    ///
    /// The offset equals the file length when no tail is present. It refers to the snapshot
    /// validated during construction and remains available after this reader is closed.
    ///
    /// @return zero-based byte offset immediately after the Janex container
    public long externalTailOffset() {
        return container.end();
    }

    /// Returns the immutable integrity report retained from construction, including after closure.
    ///
    /// This report does not assert signature validity or publisher trust. Authentication decisions
    /// belong to the supplied [AuthenticationPolicy]; the reader does not persist those decisions.
    public ContainerReader.IntegrityReport integrity() {
        return integrity;
    }

    /// Decides whether a declared verification mechanism and its exact input may be used for launching.
    ///
    /// For OpenPGP or CMS, an implementation must validate the Janex signature profile, cryptographic
    /// signature, required signer identities, and its trust, algorithm, time, and revocation policies.
    /// It must throw on failure. None and Checksum do not authenticate a publisher; a policy requiring
    /// authentication must reject both. The format reader does not provide a cryptographic verifier.
    @FunctionalInterface
    public interface AuthenticationPolicy {
        /// Authenticates a declaration or rejects preparation before application data is interpreted.
        ///
        /// This callback also receives unsigned declarations, allowing policies to require signing.
        /// A normal return permits preparation to continue; it does not waive content-integrity checks.
        /// Arguments are independent of the source channel. Modifying the input copy cannot change
        /// the reader's retained bytes. The policy and any resources it uses remain caller-owned.
        ///
        /// @param verification immutable declaration; signature internals have not been validated
        /// @param verificationInput owned copy of the original bytes that were signed or checksummed
        /// @throws IOException if signature validation or caller policy rejects this package
        void authenticate(ContainerReader.Verification verification, byte[] verificationInput) throws IOException;
    }

    /// Copies an exact physical range through the owned container reader.
    private byte[] read(long offset, long length) throws IOException {
        return container.read(offset, length);
    }

    /// Creates a nested cursor under this reader's policy.
    private Input input(byte[] bytes) throws IOException {
        return new Input(bytes, limits);
    }

    /// Returns the immutable decoding and preparation policy, including after closure.
    public ReadLimits limits() {
        return limits;
    }

    /// Validates applications and indexes pool locations after authentication and the integrity scan.
    private void loadSections() throws IOException {
        Set<String> applicationIds = new HashSet<String>();
        for (ContainerReader.Section section : container.sections()) {
            if (section.type() == 0x4c4f4f50424f4c42L) {
                poolSections.put(section.id(), section);
            } else if (section.type() == 0x50504158454e414aL) {
                require(section.info != null, "Missing section type information");
                Application application = new Application(container.readSection(section.id()), section.typeInfo(), limits);
                require(applicationIds.add(application.id()), "Duplicate application ID");
                applications.add(application);
            }
        }
    }

    /// Returns an immutable list of validated applications, available after this reader is closed.
    public List<Application> applications() {
        return Collections.unmodifiableList(applications);
    }

    /// Verifies a recorded metadata or page checksum over exact bytes.
    private static void verify(byte[] checksum, byte[] bytes) throws IOException {
        Checksum.decode(checksum).verify(bytes);
    }

    /// Describes an independently decoded range and its reversed filter output lengths.
    private final class Encoding {
        /// Unsigned stored length; allocation limits are applied when the range is read.
        final long stored;
        /// Unsigned filter output sizes in decoding order, checked before use.
        final long[] filters;
        /// Optional dictionary BlobRefs in decoding order.
        final long[][] dictionaries;

        /// Reads the binary BlobEncoding schema, rejecting unsupported methods.
        Encoding(Input input) throws IOException {
            stored = input.uint();
            filters = new long[limits.elements(input.uint())];
            dictionaries = new long[filters.length][];
            for (int i = filters.length - 1; i >= 0; i--) {
                filters[i] = input.uint();
                require(input.u8() == 1, "Unsupported blob filter");
                Map<Object, Object> properties = integers(input.map());
                if (has(properties, 0)) {
                    List<Object> ref = list(get(properties, 0));
                    require(ref.size() == 2, "Invalid dictionary BlobRef");
                    dictionaries[i] = new long[]{number(ref.get(0)), number(ref.get(1))};
                }
            }
        }

        /// Returns whether any stage requires an external dictionary.
        boolean hasDictionary() {
            for (long[] dictionary : dictionaries) {
                if (dictionary != null) {
                    return true;
                }
            }
            return false;
        }

        /// Returns the final decoded length.
        long length() {
            return filters.length == 0 ? stored : filters[filters.length - 1];
        }

        /// Decodes all filter stages with exact output-size checks.
        byte[] decode(byte[] bytes) throws IOException {
            for (int i = 0; i < filters.length; i++) {
                int length = limits.bytes(filters[i]);
                long[] ref = dictionaries[i];
                byte[] dictionary = ref == null ? new byte[0] : bytes(reference(ref[0], ref[1]), true);
                try {
                    ZstandardFrames.validate(bytes, limits);
                    bytes = decoder.decode(bytes, length, dictionary);
                    require(bytes.length == length, "Zstd decoded length mismatch");
                } catch (RuntimeException failure) {
                    throw new IOException("Invalid Zstd blob", failure);
                }
            }
            return bytes;
        }
    }

    /// One pool's page directory and lazily registered entries.
    private final class Pool {
        /// Physical start of pool bytes after the magic.
        final long start;
        /// Encoded pool byte length.
        final long length;
        /// Page entry shift.
        final int shift;
        /// Logical blob count, independent of the number of cached entries.
        final long count;
        /// Page descriptors in index order.
        final List<Object> pages;
        /// Decoded entry payloads, indexed by blob ID.
        final Map<Long, Input> entries = new HashMap<Long, Input>();
        /// Registered source IDs, indexed by blob ID.
        final Map<Long, Integer> sourceIds = new HashMap<Long, Integer>();
        /// Registered physical ranges, used to reject overlap.
        final TreeMap<Long, Long> ranges = new TreeMap<Long, Long>();

        /// Validates the page directory without reading stored resource data.
        Pool(long start, long length, Map<Object, Object> info) throws IOException {
            this.start = start;
            this.length = length;
            count = number(get(info, 0));
            long shiftValue = number(get(info, 1));
            require(shiftValue >= 8 && shiftValue <= 12, "Invalid page entry shift");
            shift = (int) shiftValue;
            pages = list(get(info, 2));
            require(count >= 0 && pages.size() == (count == 0 ? 0 : 1 + ((count - 1) >> shift)), "Incorrect page count");
            for (Object page : pages) {
                List<Object> fields = list(page);
                require(fields.size() == 2 || fields.size() == 3, "Invalid page descriptor");
                Input bytes = input(binary(fields.get(1)));
                Encoding encoding = new Encoding(bytes);
                require(!encoding.hasDictionary(), "Blob table pages must not reference dictionaries");
                bytes.end();
                if (fields.size() == 3) {
                    Checksum.decode(binary(fields.get(2)));
                }
                range(number(fields.get(0)), encoding.stored);
            }
        }

        /// Registers a nonoverlapping physical range relative to this pool.
        void range(long offset, long size) throws IOException {
            require(offset >= 0 && size >= 0 && offset <= length - size, "Blob range exceeds pool");
            if (size == 0) {
                return;
            }
            Map.Entry<Long, Long> before = ranges.floorEntry(offset);
            Map.Entry<Long, Long> after = ranges.ceilingEntry(offset);
            require((before == null || before.getValue() <= offset) && (after == null || offset + size <= after.getKey()),
                    "Overlapping blob ranges");
            ranges.put(offset, offset + size);
        }

        /// Loads the containing page and returns a fresh entry cursor.
        Input entry(long index) throws IOException {
            require(index >= 0 && index < count, "Blob index exceeds pool");
            if (!entries.containsKey(index)) {
                long first = (index >> shift) << shift;
                List<Object> fields = list(pages.get((int) (index >> shift)));
                Input encoded = input(binary(fields.get(1)));
                Encoding encoding = new Encoding(encoded);
                byte[] decoded = encoding.decode(read(start + number(fields.get(0)), encoding.stored));
                if (fields.size() == 3) {
                    verify(binary(fields.get(2)), decoded);
                }
                Input page = input(decoded);
                for (long i = first; i < Math.min(count, first + (1 << shift)); i++) {
                    int tag = page.u8();
                    byte[] payload = page.sized();
                    byte[] record = new byte[payload.length + 1];
                    record[0] = (byte) tag;
                    System.arraycopy(payload, 0, record, 1, payload.length);
                    Input entry = input(record);
                    if (tag == 0) {
                        entry.u8();
                        long offset = entry.uint();
                        Encoding stored = new Encoding(entry);
                        range(offset, stored.stored);
                        entry.end();
                    } else if (tag == 1) {
                        entry.u8();
                        int count = limits.elements(entry.uint());
                        require(count != 0, "Empty extents");
                        for (int extent = 0; extent < count; extent++) {
                            entry.uint();
                            entry.uint();
                            require(entry.uint() != 0, "Zero-length blob extent");
                        }
                        entry.end();
                    }
                    entries.put(i, input(record));
                }
                page.end();
            }
            return entries.get(index).duplicate();
        }

        /// Registers a source in dependency order, permitting only stored extent targets.
        int source(long index, boolean storedOnly) throws IOException {
            Input entry = entry(index);
            int kind = entry.u8();
            require(!storedOnly || kind == 0, "Extent target must be stored");
            Integer existing = sourceIds.get(index);
            if (existing != null) {
                return existing;
            }
            Source result = new Source();
            if (kind == 0) {
                result.offset = start + entry.uint();
                result.encoding = new Encoding(entry);
                result.length = limits.bytes(result.encoding.length());
                limits.bytes(result.encoding.stored);
                for (long length : result.encoding.filters) {
                    limits.bytes(length);
                }
            } else {
                require(kind == 1, "Unsupported blob entry");
                result.extents = new int[limits.elements(entry.uint())][3];
                require(result.extents.length != 0, "Empty extents");
                long total = 0;
                for (int[] extent : result.extents) {
                    extent[0] = source(entry.uint(), true);
                    extent[1] = limits.bytes(entry.uint());
                    extent[2] = limits.bytes(entry.uint());
                    require(extent[2] > 0 && extent[1] <= sources.get(extent[0]).length - extent[2], "Invalid extent range");
                    total += extent[2];
                }
                result.length = limits.bytes(total);
            }
            entry.end();
            int id = add(result);
            sourceIds.put(index, id);
            return id;
        }
    }

    /// One source shared by selected files and structural decoding.
    private static final class Source {
        /// Inline bytes, or null for a stored or extents source.
        byte[] inline;
        /// Deferred external JAR payload, or null for container and inline sources.
        JarSource jar;
        /// Physical stored offset.
        long offset;
        /// Stored encoding, or null for inline and extents sources.
        Encoding encoding;
        /// Extent triples, or null for inline and stored sources.
        int[][] extents;
        /// Decoded byte length.
        int length;
    }

    /// Appends a bounded source descriptor.
    private int add(Source source) throws IOException {
        limits.elements(sources.size() + 1L);
        int index = sources.size();
        sources.add(source);
        return index;
    }

    /// Registers owned inline bytes.
    private int inline(byte[] bytes) throws IOException {
        limits.bytes(bytes.length);
        Source source = new Source();
        source.inline = bytes;
        source.length = bytes.length;
        return add(source);
    }

    /// Resolves a binary BlobRef to a source ID.
    private int reference(Input input) throws IOException {
        return reference(input.uint(), input.uint());
    }

    /// Resolves a CBOR BlobRef to a source ID.
    private int reference(Object value) throws IOException {
        List<Object> ref = list(value);
        require(ref.size() == 2, "Invalid BlobRef");
        return reference(number(ref.get(0)), number(ref.get(1)));
    }

    /// Checks section identity and resolves one blob source.
    private int reference(long pool, long index) throws IOException {
        Pool opened = pools.get(pool);
        if (opened == null) {
            ContainerReader.Section section = poolSections.get(pool);
            require(section != null, "Unknown BlobPool section");
            require(section.info != null, "Missing BlobPool type information");
            opened = new Pool(section.offset() + 8, section.length() - 8, section.info);
            pools.put(pool, opened);
        }
        return opened.source(index, false);
    }

    /// Reads a complete logical blob; ordinary file data is instead read lazily by ResourceIndex.
    private byte[] bytes(int index) throws IOException {
        return bytes(index, false);
    }

    /// Reads a logical blob, forbidding dictionary references in dictionary source encodings.
    private byte[] bytes(int index, boolean dictionary) throws IOException {
        Source source = sources.get(index);
        require(!dictionary || source.encoding == null || !source.encoding.hasDictionary(),
                "Dictionary decoding must not reference another dictionary");
        if (source.inline != null) {
            return source.inline;
        }
        if (source.jar != null) {
            return source.jar.read();
        }
        if (source.encoding != null) {
            return source.encoding.decode(read(source.offset, source.encoding.stored));
        }
        byte[] result = new byte[source.length];
        int offset = 0;
        for (int[] extent : source.extents) {
            System.arraycopy(bytes(extent[0], dictionary), extent[1], result, offset, extent[2]);
            offset += extent[2];
        }
        return result;
    }

    /// Resolves and interns a complete data pool.
    private int dataPool(long pool, long index) throws IOException {
        List<Long> key = Arrays.asList(pool, index);
        Integer previous = dataPoolIds.get(key);
        if (previous != null) {
            return previous;
        }
        DataPool values = DataPool.decode(bytes(reference(pool, index)), limits);
        int id = dataPools.size();
        dataPools.add(values);
        dataPoolIds.put(key, id);
        return id;
    }

    /// Decodes a root data-pool entry as UTF-8 for a resource path or name.
    private String string(int pool, long index) throws IOException {
        DataPool values = dataPools.get(pool);
        require(index >= 0 && index < values.size(), "Invalid data-pool index");
        return Input.utf8(values.view((int) index));
    }

    /// Reads a nonempty indexed, inline, or concatenated string.
    private String name(Input input, int pool) throws IOException {
        long index = input.uint();
        if (index != 0) {
            return string(pool, index);
        }
        String value = input.string();
        if (!value.isEmpty()) {
            return value;
        }
        int count = limits.elements(input.uint());
        require(count >= 2, "Concatenation needs at least two strings");
        StringBuilder result = new StringBuilder();
        long length = 0;
        for (int i = 0; i < count; i++) {
            String part = string(pool, input.uint());
            length += limits.text(part);
            limits.bytes(length);
            result.append(part);
        }
        require(result.length() != 0, "Empty resource name");
        return result.toString();
    }

    /// One merged directory, file, link, or tombstone.
    private static final class Node {
        /// Source ID, or -1 for a directory, -2 for a link, and -3 for a tombstone.
        int source = -1;
        /// CLASSFILE steps in decoding order; pool references are resolved during materialization.
        ResourcePlan.ClassFileTransform[] transforms = new ResourcePlan.ClassFileTransform[0];
        /// Relative symbolic-link target, when present.
        String target;
        /// Exact resource metadata.
        Map<Object, Object> metadata = Collections.emptyMap();
        /// Whether this file's sources and explicit transform pools still need registration.
        boolean pending;
        /// Deferred inline bytes, or null for a blob reference.
        byte[] inline;
        /// Deferred file BlobRef, when inline bytes are absent.
        long[] reference;
        /// Optional explicit pool references in decoding order.
        long[][] transformPools;
    }

    /// Reads content descriptors and resolves explicit transform pools.
    private Node content(Input input, int pool, boolean directory) throws IOException {
        Node node = new Node();
        int kind = input.u8();
        require(kind == 0 || kind == 1, "Unsupported content source");
        node.source = 0;
        node.pending = true;
        if (kind == 0) {
            node.inline = input.sized();
        } else {
            node.reference = new long[]{input.uint(), input.uint()};
        }
        node.transforms = new ResourcePlan.ClassFileTransform[limits.elements(input.uint())];
        node.transformPools = new long[node.transforms.length][];
        require(!directory || node.transforms.length == 0, "Directory content cannot have transforms");
        for (int i = node.transforms.length - 1; i >= 0; i--) {
            int decodedLength = limits.bytes(input.uint());
            require(input.u8() == 1, "Unsupported content transform");
            Map<Object, Object> properties = integers(input.map());
            if (has(properties, 0)) {
                List<Object> ref = list(get(properties, 0));
                require(ref.size() == 2, "Invalid transform pool reference");
                node.transformPools[i] = new long[]{number(ref.get(0)), number(ref.get(1))};
            }
            node.transforms[i] = new ResourcePlan.ClassFileTransform(decodedLength, pool);
        }
        if (directory) {
            materialize(node);
        }
        return node;
    }

    /// Registers sources and override pools only for selected files or required directory arrays.
    private void materialize(Node node) throws IOException {
        if (node.pending) {
            node.source = node.inline != null ? inline(node.inline) : reference(node.reference[0], node.reference[1]);
            for (int i = 0; i < node.transforms.length; i++) {
                long[] reference = node.transformPools[i];
                if (reference != null) {
                    node.transforms[i] = new ResourcePlan.ClassFileTransform(
                            node.transforms[i].decodedLength(), dataPool(reference[0], reference[1]));
                }
            }
            node.pending = false;
        }
    }

    /// Requires a normalized root-relative resource path or one valid entry component.
    private void path(String path, boolean component) throws IOException {
        path(path, component, false);
    }

    /// Checks UTF-8 path length and component count, optionally allowing relative navigation.
    private void path(String path, boolean component, boolean navigation) throws IOException {
        limits.text(path);
        require(!component || !path.isEmpty(), "Empty entry name");
        if (path.isEmpty()) {
            return;
        }
        String[] parts = path.split("/", -1);
        limits.elements(parts.length);
        require(!component || parts.length == 1, "Entry name contains slash");
        for (String part : parts) {
            require(!part.isEmpty() && (navigation || !part.equals(".") && !part.equals("..")), "Invalid resource path");
        }
    }

    /// Returns a child path without adding a leading slash.
    private static String join(String parent, String name) {
        return parent.isEmpty() ? name : parent + "/" + name;
    }

    /// Compares valid Unicode text in UTF-8 order without allocating encoded copies.
    private static int compareText(String left, String right) {
        int l = 0;
        int r = 0;
        while (l < left.length() && r < right.length()) {
            int a = left.codePointAt(l);
            int b = right.codePointAt(r);
            if (a != b) {
                return Integer.compare(a, b);
            }
            l += Character.charCount(a);
            r += Character.charCount(b);
        }
        return Integer.compare(left.length() - l, right.length() - r);
    }

    /// Inserts missing directory parents while rejecting file/directory conflicts.
    private void directory(Map<String, Node> tree, String path) throws IOException {
        for (String current = path;;) {
            Node existing = tree.get(current);
            require(existing == null || existing.source == -1, "Resource file/directory conflict");
            if (existing == null) {
                limits.elements(tree.size() + 1L);
                tree.put(current, new Node());
            }
            int slash = current.lastIndexOf('/');
            if (slash < 0) {
                break;
            }
            current = current.substring(0, slash);
        }
    }

    /// Reads and merges all layers, validating unmatched layers as well.
    private Root root(Object reference, boolean module, boolean agent) throws IOException {
        return root(reference(reference), module, agent);
    }

    /// Reads a root whose source identity has already been resolved.
    private Root root(int source, boolean module, boolean agent) throws IOException {
        Input input = input(bytes(source));
        int pool = dataPool(input.uint(), input.uint());
        Map<Object, Object> metadata = input.map();
        for (Object key : metadata.keySet()) {
            Conditions.nonempty(key);
        }
        String jarName = metadata.containsKey("janex.java.jar_name") ? text(metadata.get("janex.java.jar_name")) : "resources.jar";
        require(jarName.endsWith(".jar") && jarName.indexOf('/') < 0 && jarName.indexOf('\\') < 0 && jarName.indexOf(0) < 0,
                "Invalid root JAR filename");
        Map<String, Node> tree = new TreeMap<String, Node>(JanexReader::compareText);
        tree.put("", new Node());
        int layers = limits.elements(input.uint());
        for (int layer = 0; layer < layers; layer++) {
            boolean matches = Conditions.matches(input.map(), context);
            Map<String, Node> records = new LinkedHashMap<String, Node>();
            Set<String> tombstones = new LinkedHashSet<String>();
            Set<String> directories = new HashSet<String>();
            directories.add("");
            Set<String> files = new HashSet<String>();
            String previousDirectory = null;
            int directoryCount = limits.elements(input.uint());
            for (int d = 0; d < directoryCount; d++) {
                String dir = string(pool, input.uint());
                path(dir, false);
                require(previousDirectory == null || compareText(previousDirectory, dir) < 0,
                        "Directories are not sorted and unique");
                previousDirectory = dir;
                Node directory = new Node();
                directory.metadata = resourceMetadata(input.map(), -1);
                require(records.put(dir, directory) == null, "Directory conflicts with entry");
                for (String current = dir;;) {
                    directories.add(current);
                    limits.elements(directories.size());
                    int slash = current.lastIndexOf('/');
                    if (slash < 0) {
                        break;
                    }
                    current = current.substring(0, slash);
                }
                int entries = limits.elements(input.uint());
                Node content = content(input, pool, true);
                Input data = input(bytes(content.source));
                String previousName = null;
                for (int e = 0; e < entries; e++) {
                    long type = data.little(4);
                    String name = name(data, pool);
                    path(name, true);
                    String fullPath = join(dir, name);
                    limits.text(fullPath);
                    require(previousName == null || compareText(previousName, name) < 0,
                            "Directory entries are not sorted and unique");
                    previousName = name;
                    Node node;
                    if (type == 0x00534552) {
                        node = content(data, pool, false);
                        node.metadata = resourceMetadata(data.map(), node.source);
                    } else {
                        node = new Node();
                        require(type == 0x4c4d5953 || type == 0x424d4f54, "Unsupported directory entry");
                        node.source = type == 0x4c4d5953 ? -2 : -3;
                        if (node.source == -2) {
                            node.target = name(data, pool);
                            path(node.target, false, true);
                            node.metadata = resourceMetadata(data.map(), -2);
                        }
                    }
                    if (node.source == -3) {
                        tombstones.add(fullPath);
                    } else {
                        require(records.put(fullPath, node) == null, "Conflicting resource paths");
                        files.add(fullPath);
                        limits.elements(files.size());
                    }
                }
                data.end();
            }
            require(Collections.disjoint(directories, files), "Conflicting implicit directory");
            if (matches) {
                for (String name : tombstones) {
                    Node old = tree.get(name);
                    if (old != null && old.source != -1) {
                        tree.remove(name);
                    }
                }
                for (Map.Entry<String, Node> record : records.entrySet()) {
                    String name = record.getKey();
                    if (record.getValue().source == -1) {
                        directory(tree, name);
                    } else {
                        int slash = name.lastIndexOf('/');
                        directory(tree, slash < 0 ? "" : name.substring(0, slash));
                        require(!tree.containsKey(name) || tree.get(name).source != -1, "Entry conflicts with directory");
                    }
                    tree.put(name, record.getValue());
                    limits.elements(tree.size());
                }
            }
        }
        input.end();
        return finishRoot(jarName, module, tree, agent);
    }

    /// Rewrites manifests and expands aliases for an imported or embedded root.
    private Root finishRoot(String jarName, boolean module, Map<String, Node> tree, boolean agent) throws IOException {
        Map<String, List<String>> children = new HashMap<String, List<String>>();
        for (String name : tree.keySet()) {
            if (!name.isEmpty()) {
                int slash = name.lastIndexOf('/');
                String parent = slash < 0 ? "" : name.substring(0, slash);
                children.computeIfAbsent(parent, ignored -> new ArrayList<String>()).add(name);
            }
        }
        Map<String, Node> expanded = new LinkedHashMap<String, Node>();
        expand(tree, children, "", "", expanded, new HashSet<String>(), agent);
        return new Root(jarName, module, expanded);
    }

    /// Identifies stale signature files directly inside META-INF, including expanded aliases.
    private static boolean jarSignature(String path) {
        int slash = path.lastIndexOf('/');
        if (slash < 0 || !asciiEquals(path.substring(0, slash), "META-INF")) {
            return false;
        }
        char[] units = path.substring(slash + 1).toCharArray();
        for (int i = 0; i < units.length; i++) {
            if (units[i] >= 'a' && units[i] <= 'z') {
                units[i] -= 'a' - 'A';
            }
        }
        String name = new String(units);
        return name.startsWith("SIG-") || name.endsWith(".SF") || name.endsWith(".RSA")
                || name.endsWith(".DSA") || name.endsWith(".EC");
    }

    /// Compares a resource path with an uppercase ASCII marker without Unicode case folding.
    private static boolean asciiEquals(String value, String marker) {
        if (value.length() != marker.length()) {
            return false;
        }
        for (int i = 0; i < value.length(); i++) {
            char unit = value.charAt(i);
            if (unit >= 'a' && unit <= 'z') {
                unit -= 'a' - 'A';
            }
            if (unit != marker.charAt(i)) {
                return false;
            }
        }
        return true;
    }

    /// Rewrites a JAR manifest for selected runtime resources.
    ///
    /// Removes the main Class-Path attribute and signature-related attributes in all sections.
    /// Other attributes, including agent capabilities, are retained. A missing Manifest-Version
    /// is supplied as 1.0. The input is not modified. Original-content checksums do not describe
    /// the returned bytes.
    ///
    /// @param bytes complete original manifest bytes
    /// @return newly encoded manifest bytes
    /// @throws IOException if the manifest cannot be parsed or encoded
    public static byte[] runtimeManifest(byte[] bytes) throws IOException {
        java.util.jar.Manifest manifest = new java.util.jar.Manifest(new ByteArrayInputStream(bytes));
        java.util.jar.Attributes main = manifest.getMainAttributes();
        main.remove(java.util.jar.Attributes.Name.CLASS_PATH);
        if (main.getValue(java.util.jar.Attributes.Name.MANIFEST_VERSION) == null) {
            main.put(java.util.jar.Attributes.Name.MANIFEST_VERSION, "1.0");
        }
        List<java.util.jar.Attributes> attributes = new ArrayList<java.util.jar.Attributes>(manifest.getEntries().values());
        attributes.add(main);
        for (java.util.jar.Attributes section : attributes) {
            Iterator<Object> keys = section.keySet().iterator();
            while (keys.hasNext()) {
                String key = keys.next().toString().toLowerCase(Locale.ROOT);
                if (key.equals("signature-version") || key.equals("magic") || key.endsWith("-digest") || key.contains("-digest-")) {
                    keys.remove();
                }
            }
        }
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        manifest.write(output);
        return output.toByteArray();
    }

    /// Checks resource metadata, preserving exact timestamps and mode zero.
    private static Map<Object, Object> resourceMetadata(Map<Object, Object> metadata, int kind) throws IOException {
        integers(metadata);
        if (has(metadata, 0)) {
            require(kind >= 0, "Checksum is only valid for files");
            Checksum.decode(binary(get(metadata, 0)));
        }
        if (has(metadata, 1)) {
            text(get(metadata, 1));
        }
        for (int key = 2; key <= 4; key++) {
            if (has(metadata, key)) {
                metadata.put((long) key, timestamp(get(metadata, key)));
            }
        }
        if (has(metadata, 5)) {
            long mode = number(get(metadata, 5));
            require(kind >= -1 && mode >= 0 && mode <= 4095, "Invalid resource permissions");
        }
        return metadata;
    }

    /// Resolves links component by component without escaping the resource root.
    private String resolve(Map<String, Node> tree, String path) throws IOException {
        path(path, false, true);
        Deque<String> pending = new ArrayDeque<String>();
        if (!path.isEmpty()) {
            Collections.addAll(pending, path.split("/", -1));
        }
        int followed = 0;
        String current = "";
        while (!pending.isEmpty()) {
            String component = pending.removeFirst();
            Node parent = tree.get(current);
            require(parent != null && parent.source == -1, "Symbolic link traverses a file");
            if (component.equals(".")) {
                continue;
            }
            if (component.equals("..")) {
                require(!current.isEmpty(), "Symbolic link escapes resource root");
                int slash = current.lastIndexOf('/');
                current = slash < 0 ? "" : current.substring(0, slash);
                continue;
            }
            String next = join(current, component);
            limits.text(next);
            Node node = tree.get(next);
            require(node != null, "Dangling symbolic link");
            if (node.source == -2) {
                limits.depth(++followed);
                String[] target = node.target.split("/", -1);
                limits.elements((long) pending.size() + target.length);
                for (int i = target.length - 1; i >= 0; i--) {
                    pending.addFirst(target[i]);
                }
            } else {
                current = next;
            }
        }
        return current;
    }

    /// Expands directory aliases into the selected resource tree.
    private void expand(Map<String, Node> tree, Map<String, List<String>> children, String canonical, String alias,
                        Map<String, Node> output, Set<String> active, boolean agent) throws IOException {
        Node node = tree.get(canonical);
        if (node.source == -2) {
            canonical = resolve(tree, canonical);
            node = tree.get(canonical);
        }
        limits.text(alias);
        if (node.source >= 0) {
            if (jarSignature(alias)) {
                return;
            }
            materialize(node);
            if (!agent && asciiEquals(alias, "META-INF/MANIFEST.MF")) {
                require(node.transforms.length == 0, "Manifest cannot contain class-file transforms");
                byte[] original = bytes(node.source);
                if (has(node.metadata, 0)) {
                    verify(binary(get(node.metadata, 0)), original);
                }
                Node manifest = new Node();
                manifest.source = inline(runtimeManifest(original));
                manifest.metadata = node.metadata;
                node = manifest;
            }
        }
        if (!alias.isEmpty() || includeRoot) {
            require(output.put(alias, node) == null, "Duplicate expanded resource");
            limits.elements(output.size());
        }
        if (node.source >= 0) {
            logicalBytes += node.transforms.length == 0 ? sources.get(node.source).length : node.transforms[node.transforms.length - 1].decodedLength();
            require(logicalBytes <= logicalLimit, "Logical resource byte limit exceeded");
            return;
        }
        limits.depth(active.size());
        require(active.add(canonical), "Directory-link cycle");
        String prefix = canonical.isEmpty() ? "" : canonical + "/";
        for (String child : children.getOrDefault(canonical, Collections.emptyList())) {
            String name = child.substring(prefix.length());
            expand(tree, children, child, join(alias, name), output, active, agent);
        }
        active.remove(canonical);
    }

    /// One selected application or agent resource root.
    private static final class Root {
        /// Original JAR filename.
        final String name;
        /// Whether this root belongs to the module path.
        final boolean module;
        /// Fully merged and expanded resource paths.
        final Map<String, Node> files;

        /// Retains one selected resource tree.
        Root(String name, boolean module, Map<String, Node> files) {
            this.name = name;
            this.module = module;
            this.files = files;
        }
    }

    /// Complete launch data selected for the current runtime.
    public static final class Launch {
        /// Binary class name, or empty when supplied by a module.
        public String mainClass = "";
        /// Main module, or empty for a classpath launch.
        public String mainModule = "";
        /// Preset arguments before user arguments.
        public final List<String> arguments = new ArrayList<String>();
        /// JVM options in application order.
        public final List<String> options = new ArrayList<String>();
        /// Selected module names and exact versions; an empty version accepts any descriptor version.
        /// The launcher checks these requirements against the runtime and indexed module path.
        public final Map<String, String> moduleRequirements = new LinkedHashMap<String, String>();
        /// Selected resources referencing the snapshot, independent of launcher serialization.
        public ResourcePlan resources;
        /// Selected roots for preparation in a child JVM; populated only by prepareHandoff.
        public final List<ResourceRequest> requests = new ArrayList<ResourceRequest>();
        /// Remaining logical byte allowance after parent-side agent preparation.
        public long resourceAllowance;
        /// Selected agent roots in option order, or null when absent.
        public ResourcePlan agentResources;
        /// One unsplit option for each root in agentResources; an empty string omits the option.
        public final List<String> agentOptions = new ArrayList<String>();
        /// Original file checksums by resource path, one map per agent root, verified before manifest rewriting.
        public final List<Map<String, byte[]>> agentChecksums = new ArrayList<Map<String, byte[]>>();
        /// Selected physical and external classpath entries.
        private final List<Object> classPath = new ArrayList<Object>();
        /// Selected physical and external module-path entries.
        private final List<Object> modulePath = new ArrayList<Object>();
        /// Selected agents, checked after overlays are applied.
        private final List<Object> agents = new ArrayList<Object>();
        /// Whether an entry point remains selected.
        private boolean entryPoint;

        /// Creates an initially empty launch selection.
        private Launch() {
        }
    }

    /// Selects an application and returns transport-independent resource descriptions.
    ///
    /// This method may be called once, including calls that fail. Returned data is caller-owned.
    /// The snapshot must remain unchanged and present while the returned resource descriptions are used.
    ///
    /// @param application explicit application ID, or null to require exactly one application
    /// @return owned launch data for the current Java runtime
    /// @throws IOException if closed, already selected, or selection, parsing, or a launch requirement fails
    public Launch launch(String application) throws IOException {
        return launch(application, false);
    }

    /// Selects startup configuration and prepares agents and module inventory for a child JVM.
    /// Classpath roots remain unexpanded. External JARs are acquired under this reader's policy
    /// referenced directly from their persistent files. Files must remain unchanged until the child
    /// exits. This consumes the same single-use selection as launch.
    /// @param application explicit application ID, or null to require exactly one application
    /// @return selected startup data, module resources, and compact child resource requests
    /// @throws IOException if selection, acquisition, or preparation fails, including a resolver
    /// returning an external dependency without a persistent file
    public Launch prepareHandoff(String application) throws IOException {
        return launch(application, true);
    }

    /// Selects one application, optionally leaving classpath resource expansion to a child.
    private Launch launch(String application, boolean handoff) throws IOException {
        require(!closed && !selected, "Janex reader is closed or already selected");
        selected = true;
        Application selected = null;
        for (Application candidate : applications) {
            if (application == null || application.equals(candidate.id())) {
                require(selected == null, "Multiple applications; set -Djanex.application=ID");
                selected = candidate;
            }
        }
        require(selected != null, "No matching Janex application");
        require(selected.type().equals("janex.java"), "Unsupported application type");
        Map<Object, Object> descriptor = selected.descriptor;
        Map<Object, Object> configuration = integers(map(get(descriptor, 0)));
        Launch launch = new Launch();
        overlay(configuration, launch, true, 0, 0, limits);
        require(launch.entryPoint, "No entry point for the current Java runtime");
        require(feature() >= 9 || launch.modulePath.isEmpty() && launch.mainModule.isEmpty(),
                "Modules require Java 9 or later");
        List<Root> roots = new ArrayList<Root>();
        for (Object entry : launch.classPath) {
            if (!handoff) roots.add(pathEntry(entry, false));
            else launch.requests.add(request(entry, false));
        }
        for (Object entry : launch.modulePath) {
            Map<Object, Object> reference = map(entry);
            ModuleRequirement requirement = number(get(reference, 0)) == 1
                    ? ModuleRequirement.parse(text(get(reference, 1)), true) : null;
            if (requirement == null) {
                if (!handoff) {
                    roots.add(pathEntry(entry, true));
                } else {
                    ResourceRequest request = request(entry, true);
                    launch.requests.add(request);
                    roots.add(request.path == null ? root(reference(request.pool, request.index), true, false)
                            : jarRoot(request.jarName, JarArchive.index(request.path, limits), true, false));
                }
            } else {
                String previous = launch.moduleRequirements.get(requirement.name());
                require(previous == null || previous.isEmpty() || requirement.version().isEmpty()
                        || previous.equals(requirement.version()), "Conflicting required module versions: " + requirement.name());
                if (previous == null || !requirement.version().isEmpty()) {
                    launch.moduleRequirements.put(requirement.name(), requirement.version());
                }
            }
        }
        long beforeAgents = logicalBytes;
        List<Root> agents = new ArrayList<Root>();
        for (Object value : launch.agents) {
            Map<Object, Object> agent = integers(map(value));
            Root root = pathEntry(get(agent, 0), false, true);
            agents.add(root);
            Map<String, byte[]> checksums = new LinkedHashMap<String, byte[]>();
            for (Map.Entry<String, Node> entry : root.files.entrySet()) {
                Node node = entry.getValue();
                if (node.source >= 0 && has(node.metadata, 0)) {
                    checksums.put(entry.getKey(), binary(get(node.metadata, 0)).clone());
                }
            }
            launch.agentChecksums.add(checksums);
            String option = text(get(agent, 1));
            require(option.indexOf(0) < 0, "Java agent option contains a NUL character");
            launch.agentOptions.add(option);
        }
        launch.resourceAllowance = logicalLimit - (logicalBytes - beforeAgents);
        prepareDictionaries();
        launch.resources = resources(roots, launch.moduleRequirements);
        if (!agents.isEmpty()) {
            launch.agentResources = resources(agents, Collections.emptyMap());
        }
        return launch;
    }

    /// Resolves a selected reference to its verified persistent file.
    private ResourceRequest request(Object value, boolean module) throws IOException {
        Map<Object, Object> entry = map(value);
        if (number(get(entry, 0)) == 0) {
            List<Object> reference = list(get(entry, 1));
            return new ResourceRequest(module, number(reference.get(0)), number(reference.get(1)));
        }
        require(resolver != null, "External dependency requires a resolver");
        String uri = text(get(entry, 1));
        byte[] checksum = has(entry, 2) ? binary(get(entry, 2)) : null;
        Dependency dependency = Objects.requireNonNull(resolver.resolve(uri, checksum));
        limits.bytes(dependency.bytes.length);
        require(dependency.path != null, "External handoff requires a persistent dependency file");
        return new ResourceRequest(module, dependency.path, dependency.jarName);
    }

    /// Resolves dictionary-backed sources before publishing their identities.
    private void prepareDictionaries() throws IOException {
        // Resolve dictionary-backed data before publishing selected resource descriptions.
        // Resolving dictionaries can register additional sources, so finish before writing the count.
        long inlineLength = 0;
        for (int i = 0; i < sources.size(); i++) {
            Source source = sources.get(i);
            if (source.inline != null || source.encoding != null && source.encoding.hasDictionary()) {
                inlineLength += source.length;
                limits.bytes(inlineLength);
            }
            if (source.encoding != null && source.encoding.hasDictionary()) {
                source.inline = bytes(i);
            }
        }
    }

    /// Describes selected roots with compact source and data-pool references.
    private ResourcePlan resources(List<Root> roots, Map<String, String> requirements) throws IOException {
        SortedSet<Integer> usedSources = new TreeSet<Integer>();
        SortedSet<Integer> usedPools = new TreeSet<Integer>();
        for (Root root : roots) {
            for (Node node : root.files.values()) {
                if (node.source >= 0) {
                    collectSource(node.source, usedSources);
                    for (ResourcePlan.ClassFileTransform transform : node.transforms) {
                        usedPools.add(transform.dataPoolIndex());
                    }
                }
            }
        }
        Map<Integer, Integer> sourceIds = new HashMap<Integer, Integer>();
        for (int id : usedSources) {
            sourceIds.put(id, sourceIds.size());
        }
        Map<Integer, Integer> poolIds = new HashMap<Integer, Integer>();
        for (int id : usedPools) {
            poolIds.put(id, poolIds.size());
        }
        List<ResourcePlan.Source> selectedSources = new ArrayList<ResourcePlan.Source>();
        for (int id : usedSources) {
            Source source = sources.get(id);
            int[] filters = new int[source.inline == null && source.encoding != null ? source.encoding.filters.length : 0];
            for (int i = 0; i < filters.length; i++) {
                filters[i] = limits.bytes(source.encoding.filters[i]);
            }
            int[][] extents = new int[source.inline == null && source.extents != null ? source.extents.length : 0][3];
            for (int i = 0; i < extents.length; i++) {
                extents[i][0] = sourceIds.get(source.extents[i][0]);
                extents[i][1] = source.extents[i][1];
                extents[i][2] = source.extents[i][2];
            }
            selectedSources.add(new ResourcePlan.Source(source.inline, source.encoding == null ? -1 : source.offset,
                    source.inline == null && source.encoding != null ? limits.bytes(source.encoding.stored) : 0,
                    filters, extents, source.jar));
        }
        DataPool[] selectedPools = new DataPool[usedPools.size()];
        for (int id : usedPools) {
            selectedPools[poolIds.get(id)] = dataPools.get(id);
        }
        List<ResourcePlan.Root> selectedRoots = new ArrayList<ResourcePlan.Root>();
        for (Root root : roots) {
            Map<String, ResourcePlan.File> files = new LinkedHashMap<String, ResourcePlan.File>();
            for (Map.Entry<String, Node> entry : root.files.entrySet()) {
                Node node = entry.getValue();
                ResourcePlan.ClassFileTransform[] transforms = new ResourcePlan.ClassFileTransform[node.transforms.length];
                for (int i = 0; i < transforms.length; i++) {
                    ResourcePlan.ClassFileTransform transform = node.transforms[i];
                    int poolIndex = poolIds.get(transform.dataPoolIndex());
                    transforms[i] = poolIndex == transform.dataPoolIndex() ? transform
                            : new ResourcePlan.ClassFileTransform(transform.decodedLength(), poolIndex);
                }
                Integer permissions = has(node.metadata, 5) ? (int) number(get(node.metadata, 5)) : null;
                files.put(node.source == -1 && !entry.getKey().isEmpty() ? entry.getKey() + "/" : entry.getKey(),
                        new ResourcePlan.File(node.source < 0 ? node.source : sourceIds.get(node.source), transforms,
                                (Instant) get(node.metadata, 2), (Instant) get(node.metadata, 3),
                                (Instant) get(node.metadata, 4), permissions));
            }
            selectedRoots.add(new ResourcePlan.Root(root.name, root.module, files));
        }
        return new ResourcePlan(path, limits, selectedSources, selectedPools, requirements, selectedRoots);
    }

    /// Collects reachable source IDs while preserving the existing dependency order.
    private void collectSource(int id, Set<Integer> used) {
        Source source = sources.get(id);
        if (used.add(id) && source.inline == null && source.extents != null) {
            for (int[] extent : source.extents) {
                collectSource(extent[0], used);
            }
        }
    }

    /// Resolves an embedded root or imports a caller-acquired external JAR.
    private Root pathEntry(Object value, boolean module) throws IOException {
        return pathEntry(value, module, false);
    }

    /// Resolves a path entry, retaining original manifests for agent integrity verification.
    private Root pathEntry(Object value, boolean module, boolean agent) throws IOException {
        Map<Object, Object> entry = integers(map(value));
        long kind = number(get(entry, 0));
        if (kind == 0) {
            return root(get(entry, 1), module, agent);
        }
        require(kind == 1, "Unsupported Java path entry");
        String uri = Conditions.nonempty(get(entry, 1));
        byte[] checksum = has(entry, 2) ? binary(get(entry, 2)) : null;
        require(resolver != null, "External dependencies require a dependency resolver");
        Dependency dependency = Objects.requireNonNull(resolver.resolve(uri, checksum));
        require(dependency.jarName.endsWith(".jar") && dependency.jarName.indexOf('/') < 0
                && dependency.jarName.indexOf('\\') < 0 && dependency.jarName.indexOf(0) < 0, "Invalid dependency JAR filename");
        return jarRoot(dependency, module, agent);
    }

    /// Imports bounded JAR entries and applies increasing multi-release layers for this runtime.
    private Root jarRoot(Dependency dependency, boolean module, boolean agent) throws IOException {
        return jarRoot(dependency.jarName, JarArchive.read(dependency.bytes, limits), module, agent);
    }

    /// Applies JAR resource semantics while retaining deferred payloads for snapshot-backed entries.
    private Root jarRoot(String jarName, List<JarArchive.Entry> entries, boolean module, boolean agent) throws IOException {
        boolean multiRelease = false;
        for (JarArchive.Entry entry : entries) {
            if (entry.name().equals("META-INF/MANIFEST.MF")) {
                require(entry.kind() != 0120000, "JAR manifest must be a regular file");
                java.util.jar.Manifest manifest = new java.util.jar.Manifest(new ByteArrayInputStream(entry.bytes()));
                multiRelease = "true".equalsIgnoreCase(manifest.getMainAttributes().getValue("Multi-Release"));
            }
        }
        SortedMap<Integer, Map<String, Node>> layers = new TreeMap<Integer, Map<String, Node>>();
        for (JarArchive.Entry entry : entries) {
            boolean directory = entry.name().endsWith("/");
            String name = directory ? entry.name().substring(0, entry.name().length() - 1) : entry.name();
            require(!name.isEmpty() && name.indexOf(0) < 0, "Invalid JAR resource path");
            path(name, false);
            int version = 0;
            if (multiRelease && name.startsWith("META-INF/versions/")) {
                String tail = name.substring("META-INF/versions/".length());
                int slash = tail.indexOf('/');
                String number = slash < 0 ? tail : tail.substring(0, slash);
                if (number.matches("[1-9][0-9]*") && number.length() <= 10) {
                    long parsed = Long.parseLong(number);
                    if (parsed >= 9 && parsed <= Integer.MAX_VALUE) {
                        version = (int) parsed;
                        name = slash < 0 ? "" : tail.substring(slash + 1);
                        require(!name.equals("META-INF") && !name.startsWith("META-INF/"), "Multi-Release JAR cannot version META-INF");
                        require(!name.isEmpty() || directory, "Multi-Release version root is not a directory");
                    }
                }
            }
            Node node = new Node();
            if (entry.kind() == 0120000) {
                node.source = -2;
                node.target = utf8(entry.bytes());
                require(node.target.indexOf(0) < 0, "Invalid JAR symbolic-link target");
                require(!node.target.isEmpty(), "Empty JAR symbolic-link target");
                path(node.target, false, true);
            } else if (!directory) {
                if (entry.source() == null) {
                    node.source = inline(entry.bytes());
                } else {
                    Source source = new Source();
                    source.jar = entry.source();
                    source.length = source.jar.length();
                    node.source = add(source);
                }
            }
            if (entry.mode() >= 0 && node.source >= -1) {
                node.metadata = new LinkedHashMap<Object, Object>();
                node.metadata.put(5L, (long) (entry.mode() & 07777));
            }
            Map<String, Node> layer = layers.computeIfAbsent(version, ignored -> new TreeMap<String, Node>(JanexReader::compareText));
            require(layer.put(name, node) == null, "Conflicting JAR resource paths");
        }
        Map<String, Node> tree = new TreeMap<String, Node>(JanexReader::compareText);
        Map<String, Node> validation = new TreeMap<String, Node>(JanexReader::compareText);
        tree.put("", new Node());
        validation.put("", new Node());
        for (Map.Entry<Integer, Map<String, Node>> layer : layers.entrySet()) {
            mergeJarLayer(validation, layer.getValue());
            if (layer.getKey() <= (context == null ? feature() : Conditions.feature(context[3]))) {
                mergeJarLayer(tree, layer.getValue());
            }
        }
        return finishRoot(jarName, module, tree, agent);
    }

    /// Merges one JAR layer without permitting implicit file/directory replacement.
    private void mergeJarLayer(Map<String, Node> tree, Map<String, Node> layer) throws IOException {
        for (Map.Entry<String, Node> entry : layer.entrySet()) {
            String name = entry.getKey();
            if (entry.getValue().source == -1) {
                directory(tree, name);
            } else {
                int slash = name.lastIndexOf('/');
                directory(tree, slash < 0 ? "" : name.substring(0, slash));
                require(!tree.containsKey(name) || tree.get(name).source != -1, "JAR file/directory conflict");
            }
            tree.put(name, entry.getValue());
            limits.elements(tree.size());
        }
    }

    /// Validates every configuration branch without selecting a runtime or resolving resources.
    static void validateConfiguration(Map<Object, Object> config, ReadLimits limits) throws IOException {
        overlay(config, new Launch(), false, 0, 0, limits);
    }

    /// Applies matching overlays in order while validating all nodes; pending counts queued siblings.
    private static void overlay(Map<Object, Object> config, Launch launch, boolean parent, int depth,
                                int pending, ReadLimits limits) throws IOException {
        limits.depth(depth);
        integers(config);
        boolean matches = parent;
        if (has(config, 0)) {
            Map<Object, Object> condition = map(get(config, 0));
            if (parent) {
                matches = Conditions.matches(condition);
            } else {
                Conditions.validate(condition);
            }
        }
        if (has(config, 1)) {
            Object value = get(config, 1);
            String mainClass = "";
            String mainModule = "";
            if (value != null) {
                Map<Object, Object> point = integers(map(value));
                mainClass = has(point, 0) ? Conditions.nonempty(get(point, 0)) : "";
                mainModule = has(point, 1) ? Conditions.nonempty(get(point, 1)) : "";
                require(!mainClass.isEmpty() || !mainModule.isEmpty(), "Empty Java entry point");
            }
            if (matches) {
                launch.entryPoint = value != null;
                launch.mainClass = mainClass;
                launch.mainModule = mainModule;
            }
        }
        for (int key = 2; key <= 3; key++) {
            if (has(config, key) && get(config, key) != null) {
                for (Object value : list(get(config, key))) {
                    validatePathEntry(value, key == 2);
                }
            }
        }
        append(config, 2, launch.modulePath, matches, limits);
        append(config, 3, launch.classPath, matches, limits);
        if (has(config, 4) && get(config, 4) != null) {
            for (Object value : list(get(config, 4))) {
                Map<Object, Object> agent = integers(map(value));
                validatePathEntry(get(agent, 0), false);
                text(get(agent, 1));
            }
        }
        append(config, 4, launch.agents, matches, limits);
        appendStrings(config, 5, launch.options, matches, limits);
        appendStrings(config, 7, launch.arguments, matches, limits);
        if (has(config, 6)) {
            List<Object> children = list(get(config, 6));
            int remaining = matches ? limits.elements((long) pending + children.size()) : pending;
            for (Object child : children) {
                if (matches) {
                    remaining--;
                }
                overlay(map(child), launch, matches, depth + 1, remaining, limits);
            }
        }
    }

    /// Checks path-reference structure and virtual-module placement, including inactive overlays.
    private static void validatePathEntry(Object value, boolean module) throws IOException {
        Map<Object, Object> reference = integers(map(value));
        long kind = number(get(reference, 0));
        require(kind == 0 || kind == 1, "Unsupported Java path entry");
        if (kind == 0) {
            List<Object> blob = list(get(reference, 1));
            require(blob.size() == 2, "Invalid path BlobRef");
            number(blob.get(0));
            number(blob.get(1));
        } else {
            ModuleRequirement.parse(Conditions.nonempty(get(reference, 1)), module);
            if (has(reference, 2)) {
                Checksum.decode(binary(get(reference, 2)));
            }
        }
    }

    /// Appends or clears a selected list while validating its outer structure.
    private static void append(Map<Object, Object> config, int key, List<Object> target, boolean matches, ReadLimits limits) throws IOException {
        if (has(config, key)) {
            Object value = get(config, key);
            List<Object> items = value == null ? Collections.emptyList() : list(value);
            if (matches) {
                if (value == null) {
                    target.clear();
                } else {
                    limits.elements((long) target.size() + items.size());
                    target.addAll(items);
                }
            }
        }
    }

    /// Appends or clears a string list without splitting argument values.
    private static void appendStrings(Map<Object, Object> config, int key, List<String> target, boolean matches, ReadLimits limits) throws IOException {
        if (has(config, key)) {
            Object value = get(config, key);
            List<String> items = new ArrayList<String>();
            if (value != null) {
                for (Object item : list(value)) {
                    items.add(text(item));
                }
            }
            if (matches) {
                if (value == null) {
                    target.clear();
                } else {
                    limits.elements((long) target.size() + items.size());
                    target.addAll(items);
                }
            }
        }
    }

    /// Returns the current Java feature version used to select launch capabilities.
    ///
    /// @return the current runtime's feature version, at least 8
    /// @throws IOException if the runtime reports an invalid or unsupported Java version
    public static int feature() throws IOException {
        return Conditions.feature();
    }

    /// Decodes a complete Zstandard input using the caller's codec implementation.
    @FunctionalInterface
    public interface BlobDecoder {
        /// Decodes all frames, consuming the complete input and checking frame integrity.
        ///
        /// @param input encoded bytes that must not be modified
        /// @param length required decoded byte length, between zero and the reader's byte limit
        /// @param dictionary decoded raw or formatted dictionary bytes, or an empty array when absent;
        ///         the array must not be modified and must initialize every frame
        /// @return a new array containing exactly the decoded bytes
        /// @throws IOException if the input is invalid or cannot produce the required output
        byte[] decode(byte[] input, int length, byte[] dictionary) throws IOException;
    }

    /// Acquires an external JAR under the caller's network, cache, and integrity policy.
    @FunctionalInterface
    public interface DependencyResolver {
        /// Resolves one selected external path entry; failure aborts launch preparation.
        ///
        /// @param uri external URI from the verified application descriptor
        /// @param checksum encoded algorithm byte and digest, or null when absent; must not be modified
        /// @return owned JAR bytes whose declared checksum has been verified when present
        /// @throws IOException if acquisition, validation, or the caller's policy fails
        Dependency resolve(String uri, byte[] checksum) throws IOException;
    }

    /// Owns acquired JAR bytes and their original filename for automatic-module naming.
    public static final class Dependency {
        /// Original filename ending in .jar, without path separators.
        public final String jarName;
        /// Complete archive bytes; must remain unchanged while the reader imports them.
        public final byte[] bytes;
        /// Persistent file containing these bytes, or null for memory-only resolution.
        /// The resolver must retain it unchanged until consumers finish reading resources.
        public final Path path;

        /// Retains the supplied array without copying it.
        ///
        /// @param jarName original JAR filename, validated during import
        /// @param bytes acquired archive bytes, validated during import
        /// @throws NullPointerException if either argument is null
        public Dependency(String jarName, byte[] bytes) {
            this(jarName, bytes, null);
        }

        /// Retains archive bytes and their optional persistent source without copying or taking ownership.
        /// @param jarName nonnull original JAR filename
        /// @param bytes nonnull acquired bytes, which must remain unchanged during import
        /// @param path matching persistent file retained by the resolver, or null for memory-only use
        /// @throws NullPointerException if jarName or bytes is null
        public Dependency(String jarName, byte[] bytes, Path path) {
            this.path = path == null ? null : path.toAbsolutePath();
            this.jarName = Objects.requireNonNull(jarName);
            this.bytes = Objects.requireNonNull(bytes);
        }
    }

    /// Closes the owned handle; the caller retains ownership of the snapshot file.
    @Override
    public void close() throws IOException {
        if (!closed) {
            closed = true;
            container.close();
        }
    }
}

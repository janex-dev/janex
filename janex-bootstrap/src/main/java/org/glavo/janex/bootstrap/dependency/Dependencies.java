// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

package org.glavo.janex.bootstrap.dependency;

import java.io.*;
import java.net.*;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.channels.FileLock;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicReference;

import org.glavo.janex.reader.Checksum;
import org.glavo.janex.reader.JanexReader;
import org.glavo.janex.reader.PackageUrl;

/// Acquires exact external JARs using the Host's dependency cache representation.
///
/// HTTP requires a secure checksum; HTTPS uses platform TLS verification. Declared checksums
/// are always checked, including on cache hits. Downloads never publish partial cache entries.
/// Each returned archive is independent of subsequent cache changes. No POMs are consulted.
public final class Dependencies implements JanexReader.DependencyResolver {
    /// Maximum raw archive bytes accepted from either transport or cache.
    private static final int MAX_BYTES = 512 * 1024 * 1024;
    /// Total download time, including redirects and body reads.
    private static final long TIMEOUT_MILLIS = 60_000;
    /// Cache-only acquisition policy.
    private final boolean offline;
    /// Whether successful downloads replace existing cache entries.
    private final boolean refresh;
    /// Repository used by Maven PURLs without an explicit repository qualifier.
    private final String repository;

    /// Captures launch policy without opening the cache or initiating requests.
    public Dependencies() throws IOException {
        offline = bool("janex.offline");
        refresh = bool("janex.refreshDependencies");
        repository = System.getProperty("janex.mavenRepository", "https://repo.maven.apache.org/maven2/");
        require(!offline || !refresh, "Offline mode cannot refresh dependencies");
    }

    /// Resolves one selected dependency, copying and checking every cache hit.
    ///
    /// Cache access may block waiting for a lock. All Janex checksum algorithms are supported.
    /// An offline miss or corrupt entry fails without creating cache directories or lock files.
    @Override
    public JanexReader.Dependency resolve(String uri, byte[] checksum) throws IOException {
        Address address = address(uri, repository);
        Checksum expected = checksum == null ? null : Checksum.decode(checksum);
        require(!address.url.getScheme().equals("http") || expected != null && expected.algorithm().isSecure(),
                "HTTP dependencies require a secure checksum");
        ByteArrayOutputStream identity = new ByteArrayOutputStream();
        identity.write(address.url.toASCIIString().getBytes(StandardCharsets.UTF_8));
        identity.write(0);
        if (checksum != null) {
            identity.write(checksum);
        }
        String key = hex(Checksum.compute(Checksum.Algorithm.SHA256, identity.toByteArray()).digest());
        Path directory = cacheDirectory();
        // FileChannel locks overlap within a JVM instead of blocking. Serialize local callers.
        synchronized (Dependencies.class) {
            if (!offline) {
                Files.createDirectories(directory);
            }
            Path path = directory.resolve(key + ".cache");
            Set<StandardOpenOption> options = EnumSet.of(StandardOpenOption.READ);
            if (!offline) {
                options.add(StandardOpenOption.WRITE);
                options.add(StandardOpenOption.CREATE);
            }
            try (FileChannel channel = FileChannel.open(directory.resolve(key + ".lock"), options);
                 FileLock lock = channel.lock(0, Long.MAX_VALUE, offline)) {
                require(lock.isValid(), "Dependency cache lock is unavailable");
                if (!refresh) {
                    try {
                        return new JanexReader.Dependency(address.name, cached(path, expected));
                    } catch (NoSuchFileException missing) {
                        // A missing entry is acquired online below.
                    } catch (IOException invalid) {
                        if (offline) {
                            throw invalid;
                        }
                    }
                }
                require(!offline, "Dependency is unavailable in the offline cache");
                byte[] bytes = download(address.url);
                byte[] digest = verify(bytes, expected);
                Path temporary = Files.createTempFile(directory, key + ".", ".tmp");
                try {
                    try (FileChannel output = FileChannel.open(temporary, StandardOpenOption.WRITE)) {
                        write(output, "JNXDEP01".getBytes(StandardCharsets.US_ASCII));
                        write(output, digest);
                        write(output, bytes);
                        output.force(true);
                    }
                    Files.move(temporary, path, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
                } finally {
                    Files.deleteIfExists(temporary);
                }
                return new JanexReader.Dependency(address.name, bytes);
            } catch (NoSuchFileException missing) {
                throw new IOException("Dependency is unavailable in the offline cache", missing);
            }
        }
    }

    /// Reads one cache record and verifies both its stored digest and optional declared checksum.
    private static byte[] cached(Path path, Checksum checksum) throws IOException {
        try (DataInputStream input = new DataInputStream(Files.newInputStream(path))) {
            byte[] header = new byte[40];
            input.readFully(header);
            require(Arrays.equals(Arrays.copyOf(header, 8), "JNXDEP01".getBytes(StandardCharsets.US_ASCII)), "Invalid dependency cache header");
            byte[] bytes = bounded(input, MAX_BYTES);
            require(MessageDigest.isEqual(verify(bytes, checksum), Arrays.copyOfRange(header, 8, 40)), "Dependency cache checksum mismatch");
            return bytes;
        }
    }

    /// Writes a complete byte array to a channel without closing it.
    private static void write(FileChannel channel, byte[] bytes) throws IOException {
        ByteBuffer buffer = ByteBuffer.wrap(bytes);
        while (buffer.hasRemaining()) {
            channel.write(buffer);
        }
    }

    /// Returns a SHA-256 cache digest after checking any declared pin.
    private static byte[] verify(byte[] bytes, Checksum checksum) throws IOException {
        byte[] sha256 = Checksum.compute(Checksum.Algorithm.SHA256, bytes).digest();
        if (checksum != null) {
            if (checksum.algorithm() == Checksum.Algorithm.SHA256) {
                require(MessageDigest.isEqual(sha256, checksum.digest()), "Dependency checksum mismatch");
            } else {
                checksum.verify(bytes);
            }
        }
        return sha256;
    }

    /// Returns lowercase hexadecimal bytes for the shared cache key.
    private static String hex(byte[] bytes) {
        StringBuilder result = new StringBuilder(bytes.length * 2);
        for (byte value : bytes) {
            result.append(Character.forDigit((value & 255) >>> 4, 16));
            result.append(Character.forDigit(value & 15, 16));
        }
        return result.toString();
    }

    /// Enforces a caller-visible deadline even if platform DNS or TLS blocks beyond socket timeouts.
    private static byte[] download(URI uri) throws IOException {
        AtomicReference<HttpURLConnection> active = new AtomicReference<HttpURLConnection>();
        ExecutorService executor = Executors.newSingleThreadExecutor(task -> {
            Thread thread = new Thread(task, "janex-dependency-download");
            thread.setDaemon(true);
            return thread;
        });
        Future<byte[]> result = executor.submit(() -> request(uri, active));
        try {
            return result.get(TIMEOUT_MILLIS, TimeUnit.MILLISECONDS);
        } catch (TimeoutException failure) {
            throw new IOException("Dependency download timed out", failure);
        } catch (InterruptedException failure) {
            Thread.currentThread().interrupt();
            throw new InterruptedIOException("Dependency download interrupted");
        } catch (ExecutionException failure) {
            Throwable cause = failure.getCause();
            if (cause instanceof IOException) {
                throw (IOException) cause;
            }
            throw new IOException("Dependency download failed", cause);
        } finally {
            boolean unfinished = !result.isDone();
            result.cancel(true);
            executor.shutdownNow();
            HttpURLConnection connection = active.get();
            if (unfinished && connection != null) {
                // Some providers wait for a blocked read inside disconnect; keep cleanup off the caller.
                Thread cleanup = new Thread(connection::disconnect, "janex-dependency-disconnect");
                cleanup.setDaemon(true);
                cleanup.start();
            }
        }
    }

    /// Downloads the original representation through at most five validated redirects.
    private static byte[] request(URI uri, AtomicReference<HttpURLConnection> active) throws IOException {
        for (int redirects = 0; redirects <= 5; redirects++) {
            require(!Thread.currentThread().isInterrupted(), "Dependency download interrupted");
            HttpURLConnection connection = (HttpURLConnection) uri.toURL().openConnection();
            active.set(connection);
            connection.setInstanceFollowRedirects(false);
            connection.setConnectTimeout(10_000);
            connection.setReadTimeout((int) TIMEOUT_MILLIS);
            connection.setUseCaches(false);
            connection.setRequestProperty("Accept-Encoding", "identity");
            try {
                require(!Thread.currentThread().isInterrupted(), "Dependency download interrupted");
                int status = connection.getResponseCode();
                if (status == 301 || status == 302 || status == 303 || status == 307 || status == 308) {
                    require(redirects < 5, "Dependency redirect limit exceeded");
                    String location = connection.getHeaderField("Location");
                    require(location != null, "Dependency redirect has no Location");
                    URI next;
                    try {
                        URI relative = new URI(location);
                        String resolved = location.startsWith("?")
                                ? uri.getScheme() + "://" + uri.getRawAuthority() + uri.getRawPath() + location
                                : uri.resolve(relative).toString();
                        next = url(resolved);
                    } catch (URISyntaxException | IllegalArgumentException failure) {
                        throw new IOException("Invalid dependency redirect", failure);
                    }
                    require(!uri.getScheme().equals("https") || next.getScheme().equals("https"), "Dependency redirect downgrades HTTPS");
                    uri = next;
                    continue;
                }
                require(status == 200, "Dependency server returned HTTP " + status);
                String encoding = connection.getHeaderField("Content-Encoding");
                require(encoding == null || encoding.equals("identity"), "Unsupported dependency Content-Encoding");
                String length = connection.getHeaderField("Content-Length");
                long expected = -1;
                if (length != null) {
                    require(length.matches("[0-9]+"), "Invalid dependency Content-Length");
                    try {
                        expected = Long.parseLong(length);
                    } catch (NumberFormatException failure) {
                        throw new IOException("Invalid dependency Content-Length", failure);
                    }
                    require(expected <= MAX_BYTES, "Dependency byte limit exceeded");
                }
                try (InputStream input = connection.getInputStream()) {
                    byte[] bytes = bounded(input, MAX_BYTES);
                    require(expected < 0 || bytes.length == expected, "Truncated dependency response");
                    return bytes;
                }
            } finally {
                connection.disconnect();
            }
        }
        throw new IOException("Dependency redirect limit exceeded");
    }

    /// Reads at most the limit plus one byte without closing the input.
    private static byte[] bounded(InputStream input, int limit) throws IOException {
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        byte[] buffer = new byte[32768];
        for (int length; (length = input.read(buffer, 0, (int) Math.min(buffer.length, (long) limit - output.size() + 1))) != -1;) {
            require(length <= limit - output.size(), "Dependency byte limit exceeded");
            output.write(buffer, 0, length);
        }
        return output.toByteArray();
    }

    /// Resolved transport URL and original artifact filename.
    static final class Address {
        /// Canonical transport URL used to derive the shared cache identity.
        final URI url;
        /// Original filename used for automatic modules.
        final String name;

        /// Retains the validated address components.
        Address(URI url, String name) {
            this.url = url;
            this.name = name;
        }
    }

    /// Maps an HTTP(S) URL or canonical Maven PURL to one exact JAR.
    static Address address(String value, String repository) throws IOException {
        if (!value.startsWith("pkg:")) {
            URI uri = url(value);
            String path = uri.getRawPath();
            String name = decode(path.substring(path.lastIndexOf('/') + 1));
            segment(name);
            require(name.endsWith(".jar"), "HTTP dependency URL must name a .jar file");
            return new Address(uri, name);
        }
        PackageUrl purl = PackageUrl.parse(value);
        require(purl.type().equals("maven") && purl.subpath() == null, "Expected a canonical Maven PURL without a subpath");
        String group = purl.namespace();
        String artifact = purl.name();
        String version = purl.version();
        require(group != null && version != null, "Maven dependency requires a group, artifact, and exact version");
        for (String component : group.split("\\.", -1)) {
            segment(component);
        }
        segment(artifact);
        segment(version);
        require(!version.endsWith("-SNAPSHOT") && !version.equals("LATEST") && !version.equals("RELEASE")
                && !version.matches(".*[\\[\\](),].*"), "Maven dependency requires an exact release or timestamped snapshot");
        Map<String, String> qualifiers = purl.qualifiers();
        for (String key : qualifiers.keySet()) {
            require(Arrays.asList("classifier", "type", "repository_url").contains(key), "Unsupported Maven qualifier: " + key);
        }
        String kind = qualifiers.getOrDefault("type", "jar");
        String classifier;
        switch (kind) {
            case "jar":
            case "bundle":
            case "maven-plugin":
            case "ejb":
                classifier = "";
                break;
            case "test-jar":
                classifier = "tests";
                break;
            case "java-source":
                classifier = "sources";
                break;
            case "javadoc":
                classifier = "javadoc";
                break;
            case "ejb-client":
                classifier = "client";
                break;
            default:
                throw new IOException("Maven Java paths must resolve to JAR artifacts");
        }
        classifier = qualifiers.getOrDefault("classifier", classifier);
        if (!classifier.isEmpty()) {
            segment(classifier);
        }
        String name = artifact + "-" + version + (classifier.isEmpty() ? "" : "-" + classifier) + ".jar";
        String baseVersion = version.replaceFirst("-[0-9]{8}\\.[0-9]{6}-[0-9]+$", "-SNAPSHOT");
        URI base = url(qualifiers.getOrDefault("repository_url", repository));
        require(base.getRawQuery() == null, "Maven repository URL must not have a query");
        StringBuilder target = new StringBuilder(base.toASCIIString());
        if (target.charAt(target.length() - 1) != '/') {
            target.append('/');
        }
        for (String component : group.split("\\.")) {
            target.append(encode(component, true)).append('/');
        }
        target.append(encode(artifact, true)).append('/').append(encode(baseVersion, true)).append('/').append(encode(name, true));
        return new Address(url(target.toString()), name);
    }

    /// Validates transport constraints and normalizes host, default port, and dot segments.
    private static URI url(String value) throws IOException {
        try {
            URI uri = new URI(value);
            String scheme = uri.getScheme();
            require(scheme != null && (scheme.equalsIgnoreCase("http") || scheme.equalsIgnoreCase("https"))
                    && uri.getRawUserInfo() == null && uri.getRawFragment() == null, "Dependency URL must use HTTP(S) without credentials or a fragment");
            scheme = scheme.toLowerCase(Locale.ROOT);
            URL parsed = uri.toURL();
            String host = parsed.getHost();
            require(!host.isEmpty() && parsed.getUserInfo() == null, "Invalid dependency URL host");
            host = host(host);
            int port = parsed.getPort();
            require(port >= -1 && port <= 65535, "Invalid dependency URL port");
            if ((scheme.equals("http") && port == 80) || (scheme.equals("https") && port == 443)) {
                port = -1;
            }
            String path = uri.getRawPath();
            if (path == null || path.isEmpty()) {
                path = "/";
            }
            List<String> segments = new ArrayList<String>();
            String[] rawSegments = path.substring(1).split("/", -1);
            for (int index = 0; index < rawSegments.length; index++) {
                String segment = rawSegments[index];
                String dots = segment.replaceAll("(?i)%2e", ".");
                if (dots.equals("..")) {
                    if (!segments.isEmpty()) {
                        segments.remove(segments.size() - 1);
                    }
                } else if (!dots.equals(".")) {
                    segments.add(segment);
                }
                if (index == rawSegments.length - 1 && (dots.equals(".") || dots.equals(".."))) {
                    segments.add("");
                }
            }
            String query = uri.getRawQuery();
            URI normalized = new URI(scheme + "://" + host + (port < 0 ? "" : ":" + port) + "/" + String.join("/", segments)
                    + (query == null ? "" : "?" + query.replace("'", "%27")));
            // URI.toASCIIString normalizes Unicode to NFC, changing the Host's byte identity.
            return new URI(encode(normalized.toString(), false));
        } catch (URISyntaxException | IllegalArgumentException failure) {
            throw new IOException("Invalid dependency URL", failure);
        }
    }

    /// Normalizes literal IP addresses without DNS and rejects lossy IDN mappings.
    private static String host(String value) throws IOException {
        if (value.startsWith("[")) {
            require(value.endsWith("]") && value.indexOf('%') < 0, "Invalid dependency IPv6 host");
            byte[] bytes = InetAddress.getByName(value).getAddress();
            if (bytes.length == 4) {
                byte[] mapped = new byte[16];
                mapped[10] = (byte) 255;
                mapped[11] = (byte) 255;
                System.arraycopy(bytes, 0, mapped, 12, 4);
                bytes = mapped;
            }
            int[] words = new int[8];
            int best = -1;
            int length = 1;
            for (int index = 0; index < 8; index++) {
                words[index] = (bytes[index * 2] & 255) * 256 + (bytes[index * 2 + 1] & 255);
            }
            for (int index = 0; index < 8;) {
                int start = index;
                while (index < 8 && words[index] == 0) {
                    index++;
                }
                if (index - start > length) {
                    best = start;
                    length = index - start;
                }
                index++;
            }
            StringBuilder host = new StringBuilder("[");
            for (int index = 0; index < 8;) {
                if (index == best) {
                    host.append("::");
                    index += length;
                } else {
                    if (host.length() > 1 && host.charAt(host.length() - 1) != ':') {
                        host.append(':');
                    }
                    host.append(Integer.toHexString(words[index++]));
                }
            }
            return host.append(']').toString();
        }
        String ascii = IDN.toASCII(value).toLowerCase(Locale.ROOT);
        boolean nonAscii = false;
        for (int index = 0; index < value.length(); index++) {
            nonAscii |= value.charAt(index) > 127;
        }
        require(!nonAscii || IDN.toUnicode(ascii).equalsIgnoreCase(value), "Ambiguous international host; use its ASCII Punycode form");
        String numeric = ascii.endsWith(".") ? ascii.substring(0, ascii.length() - 1) : ascii;
        String[] parts = numeric.split("\\.", -1);
        String last = parts[parts.length - 1];
        if (!last.matches("[0-9]+") && !last.matches("0x[0-9a-f]*")) {
            return ascii;
        }
        require(parts.length <= 4, "Invalid dependency IPv4 host");
        long address = 0;
        for (int index = 0; index < parts.length; index++) {
            String part = parts[index];
            int radix = 10;
            if (part.startsWith("0x")) {
                radix = 16;
                part = part.substring(2);
            } else if (part.length() > 1 && part.startsWith("0")) {
                radix = 8;
                part = part.substring(1);
            }
            long number;
            try {
                number = part.isEmpty() ? 0 : Long.parseLong(part, radix);
            } catch (NumberFormatException failure) {
                throw new IOException("Invalid dependency IPv4 host", failure);
            }
            long limit = index == parts.length - 1 ? 1L << (8 * (5 - parts.length)) : 256;
            require(number >= 0 && number < limit, "Invalid dependency IPv4 host");
            address += index == parts.length - 1 ? number : number << (8 * (3 - index));
        }
        return (address >>> 24) + "." + (address >>> 16 & 255) + "." + (address >>> 8 & 255) + "." + (address & 255);
    }

    /// Rejects components that alter Maven layout or native filename identity.
    private static void segment(String value) throws IOException {
        require(!value.isEmpty() && !value.equals(".") && !value.equals(".."), "Invalid dependency path component");
        for (int index = 0; index < value.length(); index++) {
            char ch = value.charAt(index);
            require(!Character.isISOControl(ch) && "/\\:<>\"|?*".indexOf(ch) < 0, "Invalid dependency path component");
        }
    }

    /// Decodes percent escapes as strict UTF-8 without treating plus signs as spaces.
    private static String decode(String value) throws IOException {
        try {
            byte[] raw = value.getBytes(StandardCharsets.UTF_8);
            ByteArrayOutputStream output = new ByteArrayOutputStream();
            for (int index = 0; index < raw.length; index++) {
                int ch = raw[index] & 255;
                if (ch == '%') {
                    require(index + 2 < raw.length, "Truncated percent escape");
                    int high = Character.digit((char) raw[++index], 16);
                    int low = Character.digit((char) raw[++index], 16);
                    require(high >= 0 && low >= 0, "Invalid percent escape");
                    ch = high * 16 + low;
                }
                output.write(ch);
            }
            return StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(output.toByteArray())).toString();
        } catch (java.nio.charset.CharacterCodingException failure) {
            throw new IOException("Dependency name is not UTF-8", failure);
        }
    }

    /// Encodes a transport path segment or raw URL with Host-compatible sets.
    private static String encode(String value, boolean pathSegment) {
        String escaped = pathSegment ? " \"#<>?`{}/%" : "";
        StringBuilder result = new StringBuilder();
        for (byte valueByte : value.getBytes(StandardCharsets.UTF_8)) {
            int ch = valueByte & 255;
            if (ch < 32 || ch >= 127 || escaped.indexOf(ch) >= 0) {
                result.append('%').append("0123456789ABCDEF".charAt(ch >>> 4)).append("0123456789ABCDEF".charAt(ch & 15));
            } else {
                result.append((char) ch);
            }
        }
        return result.toString();
    }

    /// Locates the same platform cache directory used by the Rust Host.
    private static Path cacheDirectory() throws IOException {
        String explicit = System.getProperty("janex.dependencyCache");
        if (explicit != null) {
            require(!explicit.isEmpty(), "Empty dependency cache directory");
            return Paths.get(explicit);
        }
        String os = System.getProperty("os.name");
        String home = System.getenv("HOME");
        if (os.startsWith("Windows")) {
            String local = System.getenv("LOCALAPPDATA");
            require(local != null, "Cannot locate dependency cache; set janex.dependencyCache");
            return Paths.get(local, "Janex", "Cache", "dependencies");
        }
        if (os.startsWith("Mac")) {
            require(home != null, "Cannot locate dependency cache; set janex.dependencyCache");
            return Paths.get(home, "Library", "Caches", "janex", "dependencies");
        }
        String xdg = System.getenv("XDG_CACHE_HOME");
        if (xdg != null && Paths.get(xdg).isAbsolute()) {
            return Paths.get(xdg, "janex", "dependencies");
        }
        require(home != null, "Cannot locate dependency cache; set janex.dependencyCache");
        return Paths.get(home, ".cache", "janex", "dependencies");
    }

    /// Reads a strict boolean launch-policy property.
    private static boolean bool(String name) throws IOException {
        String value = System.getProperty(name, "false");
        require(value.equalsIgnoreCase("true") || value.equalsIgnoreCase("false"), "Invalid boolean property: " + name);
        return Boolean.parseBoolean(value);
    }

    /// Rejects a failed acquisition or policy constraint.
    private static void require(boolean valid, String message) throws IOException {
        if (!valid) {
            throw new IOException(message);
        }
    }
}

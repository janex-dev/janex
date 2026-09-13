# Dependency Cache

The Rust Host and standalone Java launcher share this internal cache representation. It is separate
from the Janex file format. Both return owned archive bytes and recheck cache integrity on every hit.

The default directory is `<JANEX_HOME>/cache/dependencies`. `JANEX_HOME` defaults to
`%USERPROFILE%/.janex` on Windows and `$HOME/.janex` elsewhere. An explicit `JANEX_HOME`
must be a nonempty absolute path; an invalid value is an error rather than a fallback.
The selected user home must also be absolute. Directory lookup does not create directories.
`--dependency-cache` (Rust CLI) and `janex.dependencyCache` (Java) override the dependency
cache directly and take precedence over `JANEX_HOME`. Local-only launches need no cache.

The cache contains `files/sha256/<first-two-digest-digits>/<remaining-digits>/<filename>`,
`metadata/maven/`, `metadata/urls/`, `locks/`, and `tmp/`. Files contain original archive bytes,
without a cache header. Digests are lowercase hexadecimal SHA-256. Filenames retain their
dependency semantics and must be safe single path components on every supported platform.

The lowercase hexadecimal SHA-256 of the following bytes identifies a request:

```text
original dependency URI in UTF-8 | 00 | resolved transport URL in UTF-8 | 00 | optional encoded ChecksumValue
```

Use the resolved URL before redirects. A checksum includes its algorithm byte. Metadata is stored
as `<key>.cbor` under `metadata/maven` for Maven PURLs or `metadata/urls` for HTTP(S) addresses.
Each record is a deterministic CBOR map, limited to 1 MiB:

```text
{0: 1, 1: uri, 2: resolved_url, 3: filename, 4: checksum_or_null, 5: sha256}
```

Fields 1–3 are text strings, field 4 is the encoded checksum byte string or null, and field 5 is
the 32-byte content digest. Readers compare all request fields before using the content reference.
Different sources can reference the same content path. Resolution remains exact and immutable
until explicitly refreshed; no HTTP freshness or conditional-request policy is currently applied.

On an online cache miss, a declared SHA-256 can locate shared content before downloading. Maven
requests with a secure declared checksum also try the conventional user `.m2/repository` layout.
Only fully verified bytes are imported; missing or mismatched candidates fall back to the network.
The Maven repository is never modified. Maven settings and alternative local-repository layouts
are not interpreted. Explicit refresh bypasses these candidates; offline mode requires a Janex record.

Verify both the stored SHA-256 and any declared dependency checksum before accepting the bytes.
Online writers hold `locks/<key>.lock`, write and sync temporary files under `tmp/`, and atomically
publish content before metadata. Valid existing content is reused; repairs replace files rather
than modifying them in place. Offline readers use shared read-only locks and never create directories
or lock files. Failed downloads leave existing records intact. Installed applications must retain
their dependencies independently of disposable cache paths.

Java uses `FileChannel` locks; Rust uses `fs4` locks. These interoperate on Windows. On Unix, the
underlying `fcntl` and `flock` locks may be independent, so simultaneous Java and Rust launches may
download the same entry twice. Atomic replacement and validation protect complete cache records
independently of lock interoperability. Filesystems must support atomic replacement.

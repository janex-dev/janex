# Dependency Cache

The Rust Host and standalone Java launcher share this internal cache representation. It is separate
from the Janex file format. Both return owned archive bytes and recheck cache integrity on every hit.

The default directory is `%LOCALAPPDATA%/Janex/Cache/dependencies` on Windows,
`$HOME/Library/Caches/janex/dependencies` on macOS, and `$XDG_CACHE_HOME/janex/dependencies` on other
systems. An absent or relative `XDG_CACHE_HOME` falls back to `$HOME/.cache`.

The lowercase hexadecimal SHA-256 of the following bytes identifies an entry:

```text
resolved transport URL in UTF-8 | 00 | optional encoded ChecksumValue
```

Use the original resolved URL before redirects. A checksum includes its algorithm byte. The entry
is `<key>.cache`; `<key>.lock` is a persistent lock file. Cache contents are:

```text
"JNXDEP01" (8 bytes) | SHA-256 of original JAR (32 bytes) | original JAR bytes
```

Verify both the stored SHA-256 and any declared dependency checksum before accepting the bytes.
Online writers hold an exclusive lock, write and sync a temporary file in the cache directory, and
atomically replace the entry only after verification. Offline readers use shared read-only locks
and never create directories or lock files. Failed downloads leave existing records intact.

Java uses `FileChannel` locks; Rust uses `fs4` locks. These interoperate on Windows. On Unix, the
underlying `fcntl` and `flock` locks may be independent, so simultaneous Java and Rust launches may
download the same entry twice. Atomic replacement and validation protect complete cache records
independently of lock interoperability. Filesystems must support atomic replacement.

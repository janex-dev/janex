// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Best-effort runtime metadata caching for recognizable native Java installations.

use crate::{Result, runtime::JavaRuntime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fmt::Write as _,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Maximum serialized entry or release-file size accepted by this cache.
const MAX_BYTES: u64 = 256 * 1024;

/// Environment overrides whose contents or referenced files can affect a probe.
const OVERRIDES: &[&str] = &[
    "JAVA_TOOL_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "_JAVA_OPTIONS",
    "IBM_JAVA_OPTIONS",
    "OPENJ9_JAVA_OPTIONS",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
];

/// Core files whose presence and identity form part of the installation fingerprint.
const CORE_FILES: &[&str] = &["lib/modules", "lib/rt.jar", "jre/lib/rt.jar"];

/// Native runtime and launcher configuration locations used by common JVM layouts.
const RUNTIME_FILES: &[&str] = &[
    "lib/jvm.cfg",
    "jre/lib/jvm.cfg",
    "bin/java.dll",
    "bin/jli.dll",
    "bin/server/jvm.dll",
    "bin/client/jvm.dll",
    "jre/bin/java.dll",
    "jre/bin/server/jvm.dll",
    "jre/bin/client/jvm.dll",
    "lib/libjava.so",
    "lib/libjli.so",
    "lib/server/libjvm.so",
    "lib/client/libjvm.so",
    "lib/libjava.dylib",
    "lib/libjli.dylib",
    "lib/server/libjvm.dylib",
];

/// File identity sufficient to notice ordinary replacement and in-place installation updates.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    /// Canonical target, including resolved links inside the installation.
    path: PathBuf,
    /// Exact physical byte length.
    length: u64,
    /// Last modification time at the filesystem's available precision.
    modified: SystemTime,
    /// Creation time when supported by the filesystem.
    created: Option<SystemTime>,
    /// Read-only state, including platforms without Unix mode bits.
    readonly: bool,
    /// Unix device, inode, and permissions when available.
    #[cfg(unix)]
    identity: (u64, u64, u32),
}

impl Stamp {
    /// Reads one existing regular file; failures prevent caching that installation.
    fn read(path: &Path) -> io::Result<Self> {
        let path = path.canonicalize()?;
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(io::Error::other("Java cache input is not a regular file"));
        }
        Ok(Self {
            path,
            length: metadata.len(),
            modified: metadata.modified()?,
            created: metadata.created().ok(),
            readonly: metadata.permissions().readonly(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (metadata.dev(), metadata.ino(), metadata.mode())
            },
        })
    }

    /// Distinguishes absence from unreadable files and dangling links.
    fn optional(path: &Path) -> io::Result<Option<Self>> {
        match fs::symlink_metadata(path) {
            Ok(_) => Self::read(path).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// Local installation state recorded before and after a successful probe.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Fingerprint {
    /// Launcher file attributes and canonical identity.
    executable: Stamp,
    /// Digest of the complete small release file, or absence for older installations.
    release: Option<[u8; 32]>,
    /// Optional core and native files in stable relative-name order.
    files: Vec<Option<Stamp>>,
}

impl Fingerprint {
    /// Recognizes native bin/java layouts with Java 8 or modular core libraries.
    fn read(executable: &Path) -> io::Result<Self> {
        let bin = executable
            .parent()
            .filter(|path| path.file_name() == Some(OsStr::new("bin")))
            .ok_or_else(|| io::Error::other("Unrecognized Java launcher layout"))?;
        if fs::symlink_metadata(bin.join("ikvm.properties")).is_ok() {
            return Err(io::Error::other("IKVM metadata caching is unsupported"));
        }
        let home = bin
            .parent()
            .ok_or_else(|| io::Error::other("Missing Java home"))?;
        let mut magic = [0; 4];
        fs::File::open(executable)?.read_exact(&mut magic)?;
        if !native_magic(magic) {
            return Err(io::Error::other(
                "Java launcher is not a recognized native executable",
            ));
        }
        let release = match fs::File::open(home.join("release")) {
            Ok(file) => Some(Sha256::digest(bounded(file)?).into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let mut files = CORE_FILES
            .iter()
            .map(|name| Stamp::optional(&home.join(name)))
            .collect::<io::Result<Vec<_>>>()?;
        if files.iter().all(Option::is_none) {
            return Err(io::Error::other("Unrecognized Java core library layout"));
        }
        for name in RUNTIME_FILES {
            files.push(Stamp::optional(&home.join(name))?);
        }
        // Java 8 on Unix places native libraries beneath architecture directories.
        #[cfg(unix)]
        for lib in [home.join("lib"), home.join("jre/lib")] {
            if lib.is_dir() {
                let mut directories = fs::read_dir(lib)?.collect::<io::Result<Vec<_>>>()?;
                directories.sort_by_key(|entry| entry.file_name());
                for entry in directories {
                    if entry.path().is_dir() {
                        for name in [
                            "jvm.cfg",
                            "libjava.so",
                            "server/libjvm.so",
                            "client/libjvm.so",
                        ] {
                            if let Some(stamp) = Stamp::optional(&entry.path().join(name))? {
                                files.push(Some(stamp));
                            }
                        }
                    }
                }
            }
        }
        Ok(Self {
            executable: Stamp::read(executable)?,
            release,
            files,
        })
    }
}

/// Recognizes PE, ELF, and both endian forms of Mach-O and universal Mach-O headers.
fn native_magic(bytes: [u8; 4]) -> bool {
    bytes.starts_with(b"MZ")
        || bytes == *b"\x7fELF"
        || matches!(
            u32::from_be_bytes(bytes),
            0xfeedface
                | 0xcefaedfe
                | 0xfeedfacf
                | 0xcffaedfe
                | 0xcafebabe
                | 0xbebafeca
                | 0xcafebabf
                | 0xbfbafeca
        )
}

/// Private serialization adapter; JavaRuntime itself has no public persistence contract.
#[derive(Serialize, Deserialize)]
#[serde(remote = "JavaRuntime")]
struct RuntimeData {
    /// Absolute executable selected by the caller.
    executable: PathBuf,
    /// Runtime-reported home.
    home: PathBuf,
    /// Java feature number.
    feature: u32,
    /// Exact reported version.
    version_text: String,
    /// Reported vendor name.
    vendor: String,
    /// Normalized architecture of the actual JVM.
    architecture: String,
    /// Optional runtime VM name.
    vm_name: Option<String>,
    /// Observable system modules and versions.
    modules: BTreeMap<String, Option<String>>,
}

/// One independently and atomically replaceable runtime entry.
#[derive(Serialize, Deserialize)]
struct Entry {
    /// Version of this private cache schema, independent of the Janex file format.
    version: u32,
    /// Installation state used to produce the runtime information.
    fingerprint: Fingerprint,
    /// Validated probe result.
    #[serde(with = "RuntimeData")]
    runtime: JavaRuntime,
}

/// Reads a bounded local cache object without trusting its recorded file length.
fn bounded(file: fs::File) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("Java cache input exceeds the byte limit"));
    }
    Ok(bytes)
}

/// Isolates entries by canonical path and launcher platform, including translated launchers.
fn entry_path(directory: &Path, executable: &Path) -> PathBuf {
    let mut digest = Sha256::new();
    digest.update(std::env::consts::OS);
    digest.update([0]);
    digest.update(std::env::consts::ARCH);
    digest.update([0]);
    digest.update(executable.as_os_str().as_encoded_bytes());
    let mut name = String::with_capacity(69);
    for byte in digest.finalize() {
        write!(name, "{byte:02x}").expect("writing to a string cannot fail");
    }
    name.push_str(".json");
    directory.join(name)
}

/// Loads a bounded, matching entry without allowing cached data to redirect execution.
fn load(path: &Path, fingerprint: &Fingerprint) -> Option<JavaRuntime> {
    let entry: Entry = serde_json::from_slice(&bounded(fs::File::open(path).ok()?).ok()?).ok()?;
    let runtime = entry.runtime;
    (entry.version == 1
        && entry.fingerprint == *fingerprint
        && runtime.executable == fingerprint.executable.path
        && runtime.home.is_absolute()
        && super::runtime::feature_version(&runtime.version_text).ok() == Some(runtime.feature)
        && !runtime.architecture.is_empty()
        && if runtime.feature >= 9 {
            runtime.modules.contains_key("java.base")
        } else {
            runtime.modules.is_empty()
        })
    .then_some(runtime)
}

/// Publishes an entry with a same-directory rename; failed writes do not damage existing entries.
fn save(path: &Path, entry: &Entry) -> io::Result<()> {
    let bytes = serde_json::to_vec(entry)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("Java cache entry exceeds the byte limit"));
    }
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing Java cache directory"))?;
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Returns whether probes run without option injection or dynamic-library overrides.
fn environment_allows_cache(mut value: impl FnMut(&str) -> Option<std::ffi::OsString>) -> bool {
    OVERRIDES
        .iter()
        .all(|name| value(name).is_none_or(|value| value.is_empty()))
}

/// Resolves the user cache without making its availability a launch requirement.
pub(crate) fn probe(executable: &Path) -> Result<JavaRuntime> {
    if !environment_allows_cache(|name| std::env::var_os(name)) {
        return JavaRuntime::probe(executable);
    }
    let Ok(home) = janex_platform::janex_home() else {
        return JavaRuntime::probe(executable);
    };
    probe_with(executable, &home.join("cache/java"), JavaRuntime::probe)
}

/// Caches only successful probes with a stable fingerprint across the probe interval.
fn probe_with(
    executable: &Path,
    directory: &Path,
    fresh: impl FnOnce(&Path) -> Result<JavaRuntime>,
) -> Result<JavaRuntime> {
    let executable = executable.canonicalize()?;
    let Ok(before) = Fingerprint::read(&executable) else {
        return fresh(&executable);
    };
    let path = entry_path(directory, &executable);
    if let Some(runtime) = load(&path, &before) {
        return Ok(runtime);
    }
    let runtime = fresh(&executable)?;
    if Fingerprint::read(&executable).ok().as_ref() == Some(&before) {
        let _ = save(
            &path,
            &Entry {
                version: 1,
                fingerprint: before,
                runtime: runtime.clone(),
            },
        );
    }
    Ok(runtime)
}

#[cfg(test)]
mod tests {
    //! Installation update, persistence, failure, and concurrency tests without environment mutation.
    use super::*;
    use std::cell::Cell;

    /// Creates a recognizable installation and an independent writable cache directory.
    fn installation() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("jdk");
        fs::create_dir_all(home.join("bin")).unwrap();
        fs::create_dir(home.join("lib")).unwrap();
        let executable = home
            .join("bin")
            .join(if cfg!(windows) { "java.exe" } else { "java" });
        fs::write(&executable, b"\x7fELFtest launcher").unwrap();
        fs::write(home.join("release"), b"JAVA_VERSION=25\n").unwrap();
        fs::write(home.join("lib/modules"), b"modules").unwrap();
        let cache = directory.path().join("cache");
        (directory, executable, cache)
    }

    /// Returns deliberately foreign-architecture information independent of the launcher architecture.
    fn runtime(executable: &Path) -> JavaRuntime {
        JavaRuntime {
            executable: executable.to_owned(),
            home: executable.parent().unwrap().parent().unwrap().to_owned(),
            feature: 25,
            version_text: "25".into(),
            vendor: "Example".into(),
            architecture: "aarch64".into(),
            vm_name: Some("OpenJDK 64-Bit Server VM".into()),
            modules: BTreeMap::from([
                ("java.base".into(), Some("25".into())),
                ("example.module".into(), None),
            ]),
        }
    }

    #[test]
    fn persisted_hit_skips_probe_and_preserves_actual_vm_architecture_and_modules() {
        let (_directory, executable, cache) = installation();
        let first = probe_with(&executable, &cache, |path| Ok(runtime(path))).unwrap();
        let second = probe_with(&executable, &cache, |_| panic!("cache hit started Java")).unwrap();
        assert_eq!(first.executable, second.executable);
        assert_eq!(first.home, second.home);
        assert_eq!(second.architecture, "aarch64");
        assert_eq!(first.modules, second.modules);
        assert_eq!(first.vm_name, second.vm_name);
    }

    #[test]
    fn release_content_invalidates_even_with_unchanged_length_and_timestamp() {
        let (_directory, executable, cache) = installation();
        let calls = Cell::new(0);
        let probe = |path: &Path| {
            calls.set(calls.get() + 1);
            Ok(runtime(path))
        };
        probe_with(&executable, &cache, probe).unwrap();
        let release = executable
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("release");
        let modified = fs::metadata(&release).unwrap().modified().unwrap();
        fs::write(&release, b"JAVA_VERSION=26\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&release)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        probe_with(&executable, &cache, probe).unwrap();
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn launcher_core_and_native_runtime_changes_invalidate_entries() {
        let (_directory, executable, cache) = installation();
        let home = executable.parent().unwrap().parent().unwrap();
        let calls = Cell::new(0);
        let probe = |path: &Path| {
            calls.set(calls.get() + 1);
            Ok(runtime(path))
        };
        probe_with(&executable, &cache, probe).unwrap();
        fs::write(&executable, b"\x7fELFnew launcher with different size").unwrap();
        probe_with(&executable, &cache, probe).unwrap();
        fs::write(home.join("lib/modules"), b"new modules").unwrap();
        probe_with(&executable, &cache, probe).unwrap();
        let native = home.join("lib/server/libjvm.so");
        fs::create_dir(native.parent().unwrap()).unwrap();
        fs::write(&native, b"VM").unwrap();
        probe_with(&executable, &cache, probe).unwrap();
        fs::write(&native, b"updated VM").unwrap();
        probe_with(&executable, &cache, probe).unwrap();
        assert_eq!(calls.get(), 5);
        fs::remove_file(&executable).unwrap();
        assert!(
            probe_with(&executable, &cache, |_| panic!(
                "missing launcher was probed"
            ))
            .is_err()
        );
    }

    #[test]
    fn legacy_java_8_core_without_release_is_cacheable() {
        let (_directory, executable, cache) = installation();
        let home = executable.parent().unwrap().parent().unwrap();
        fs::remove_file(home.join("release")).unwrap();
        fs::remove_file(home.join("lib/modules")).unwrap();
        fs::create_dir_all(home.join("jre/lib")).unwrap();
        fs::write(home.join("jre/lib/rt.jar"), b"legacy core").unwrap();
        probe_with(&executable, &cache, |path| {
            let mut value = runtime(path);
            value.feature = 8;
            value.version_text = "1.8.0_442".into();
            value.home.push("jre");
            value.modules.clear();
            Ok(value)
        })
        .unwrap();
        let cached = probe_with(&executable, &cache, |_| panic!("legacy cache missed")).unwrap();
        assert_eq!(cached.feature, 8);
        assert!(cached.modules.is_empty());
        assert!(cached.home.ends_with("jre"));
    }

    #[test]
    fn malformed_oversized_and_unknown_entries_are_replaced() {
        let (_directory, executable, cache) = installation();
        let canonical = executable.canonicalize().unwrap();
        let file = entry_path(&cache, &canonical);
        fs::create_dir_all(&cache).unwrap();
        for bad in [b"broken json".to_vec(), vec![b' '; MAX_BYTES as usize + 1]] {
            fs::write(&file, bad).unwrap();
            probe_with(&executable, &cache, |path| Ok(runtime(path))).unwrap();
            probe_with(&executable, &cache, |_| {
                panic!("replacement was not readable")
            })
            .unwrap();
        }
        let mut entry: Entry = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        entry.version = 99;
        fs::write(&file, serde_json::to_vec(&entry).unwrap()).unwrap();
        let called = Cell::new(false);
        probe_with(&executable, &cache, |path| {
            called.set(true);
            Ok(runtime(path))
        })
        .unwrap();
        assert!(called.get());
    }

    #[test]
    fn cached_executable_cannot_redirect_the_launch() {
        let (_directory, executable, cache) = installation();
        probe_with(&executable, &cache, |path| Ok(runtime(path))).unwrap();
        let file = entry_path(&cache, &executable.canonicalize().unwrap());
        let mut entry: Entry = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        entry.runtime.executable = cache.join("different-program");
        fs::write(&file, serde_json::to_vec(&entry).unwrap()).unwrap();
        let called = Cell::new(false);
        let value = probe_with(&executable, &cache, |path| {
            called.set(true);
            Ok(runtime(path))
        })
        .unwrap();
        assert!(called.get());
        assert_eq!(value.executable, executable.canonicalize().unwrap());
    }

    #[test]
    fn unavailable_cache_and_failed_probes_do_not_create_negative_entries() {
        let (directory, executable, cache) = installation();
        fs::write(&cache, b"not a directory").unwrap();
        let calls = Cell::new(0);
        for _ in 0..2 {
            probe_with(&executable, &cache, |path| {
                calls.set(calls.get() + 1);
                Ok(runtime(path))
            })
            .unwrap();
        }
        assert_eq!(calls.get(), 2);
        let other = directory.path().join("other-cache");
        assert!(
            probe_with(&executable, &other, |_| Err(crate::error::invalid(
                "probe failed"
            )))
            .is_err()
        );
        assert!(!other.exists());
        probe_with(&executable, &other, |path| Ok(runtime(path))).unwrap();
    }

    #[test]
    fn changes_during_a_probe_are_not_published() {
        let (_directory, executable, cache) = installation();
        probe_with(&executable, &cache, |path| {
            fs::write(
                path.parent().unwrap().parent().unwrap().join("release"),
                b"updated",
            )
            .unwrap();
            Ok(runtime(path))
        })
        .unwrap();
        assert!(!cache.exists());
    }

    #[test]
    fn wrappers_ikvm_and_unknown_layouts_always_probe() {
        let (_directory, executable, cache) = installation();
        fs::write(
            executable.with_file_name("ikvm.properties"),
            b"configuration",
        )
        .unwrap();
        let calls = Cell::new(0);
        let probe = |path: &Path| {
            calls.set(calls.get() + 1);
            Ok(runtime(path))
        };
        probe_with(&executable, &cache, probe).unwrap();
        probe_with(&executable, &cache, probe).unwrap();
        fs::remove_file(executable.with_file_name("ikvm.properties")).unwrap();
        fs::write(&executable, b"#!/bin/sh\nexec another-java\n").unwrap();
        probe_with(&executable, &cache, probe).unwrap();
        fs::write(&executable, b"\x7fELFnative").unwrap();
        fs::remove_file(
            executable
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("lib/modules"),
        )
        .unwrap();
        probe_with(&executable, &cache, probe).unwrap();
        assert_eq!(calls.get(), 4);
        assert!(!cache.exists());
    }

    #[test]
    fn option_and_loader_environment_overrides_bypass_cache() {
        assert!(environment_allows_cache(|_| None));
        assert!(environment_allows_cache(|_| Some("".into())));
        for name in OVERRIDES {
            assert!(!environment_allows_cache(
                |key| (key == *name).then(|| "override".into())
            ));
        }
    }

    #[test]
    fn concurrent_writers_publish_complete_entries() {
        let (_directory, executable, cache) = installation();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    probe_with(&executable, &cache, |path| Ok(runtime(path))).unwrap();
                });
            }
        });
        probe_with(&executable, &cache, |_| {
            panic!("concurrent entry was lost or malformed")
        })
        .unwrap();
        assert_eq!(fs::read_dir(cache).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn executable_symlink_retargeting_uses_the_new_installation() {
        let (directory, first, cache) = installation();
        let (_second_directory, second, _second_cache) = installation();
        let alias = directory.path().join("java-alias");
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        probe_with(&alias, &cache, |path| Ok(runtime(path))).unwrap();
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&second, &alias).unwrap();
        let actual = probe_with(&alias, &cache, |path| Ok(runtime(path))).unwrap();
        assert_eq!(actual.executable, second.canonicalize().unwrap());
    }
}

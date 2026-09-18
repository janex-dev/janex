// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Installation and verified replacement of the managed Janex executable.

mod archive;
mod release;

use crate::{Result, error::invalid};
use semver::Version;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

/// Maximum compressed archive or executable size.
const MAX_BYTES: u64 = 128 * 1024 * 1024;
/// Executable name in the distribution and managed bin directory.
const EXECUTABLE: &str = if cfg!(windows) { "janex.exe" } else { "janex" };

/// Source selection for self-update; absent fields select the latest stable GitHub release.
#[derive(Default)]
pub struct UpdateOptions {
    /// Exact release version, optionally prefixed with `v`; mutually exclusive with `archive`.
    pub version: Option<String>,
    /// Explicitly trusted local ZIP or tar.xz distribution, without network access.
    pub archive: Option<PathBuf>,
    /// Optional SHA-256 hex digest required to match the local archive.
    pub sha256: Option<String>,
}

/// Installs an independent copy into `root/bin` and returns its absolute path.
/// Existing binaries are replaced after staging; links at the destination are rejected.
/// Shell setup is the caller's responsibility. Reinstalling from this same path is a no-op.
pub fn install(root: &Path, source: &Path) -> Result<PathBuf> {
    let target = target(root)?;
    let _lock = lock(root)?;
    reject_link(&target)?;
    cleanup(&target)?;
    if target.is_file() && source.canonicalize()? == target.canonicalize()? {
        return Ok(target);
    }
    let stage = staging(&target)?;
    let candidate = stage.path().join(EXECUTABLE);
    fs::copy(source, &candidate)?;
    fs::OpenOptions::new()
        .write(true)
        .open(&candidate)?
        .sync_all()?;
    replace(&candidate, &target, stage)?;
    Ok(target)
}

/// Updates only the managed executable, retaining the old binary if acquisition or validation fails.
/// An exact version or local archive may downgrade; latest-release updates never downgrade.
/// Returns the new version, or `None` when the latest release is not newer.
/// Windows may retain an old executable in a staging directory until a later self operation.
pub fn update(root: &Path, current: &Path, options: &UpdateOptions) -> Result<Option<String>> {
    if options.archive.is_some() && options.version.is_some()
        || options.sha256.is_some() && options.archive.is_none()
    {
        return Err(invalid("invalid self-update source options"));
    }
    let target = target(root)?;
    reject_link(&target)?;
    if !target.is_file() || current.canonicalize()? != target.canonicalize()? {
        return Err(invalid(
            "run the managed Janex executable after `janex self install`",
        ));
    }
    let _lock = lock(root)?;
    cleanup(&target)?;
    let platform = release::platform(
        std::env::consts::OS,
        &janex_platform::native_architecture()?,
    )?;
    let (bytes, name, expected_version) = if let Some(path) = &options.archive {
        let bytes = read_bounded(path)?;
        if let Some(checksum) = &options.sha256 {
            release::checksum(checksum)?.verify(bytes.as_slice())?;
        }
        (
            bytes,
            path.file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_owned(),
            None,
        )
    } else {
        let release = release::resolve(options.version.as_deref(), &platform)?;
        if options.version.is_none()
            && release.version <= Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
        {
            return Ok(None);
        }
        let bytes = release.download()?;
        (bytes, release.name, Some(release.version))
    };
    let stage = staging(&target)?;
    let candidate = stage.path().join(EXECUTABLE);
    archive::extract(&bytes, &name, &candidate)?;
    let version = probe(&candidate)?;
    if expected_version
        .as_ref()
        .is_some_and(|expected| *expected != version)
    {
        return Err(invalid(
            "downloaded Janex version does not match its release",
        ));
    }
    replace(&candidate, &target, stage)?;
    Ok(Some(version.to_string()))
}

/// Returns the managed executable path without creating it.
fn target(root: &Path) -> Result<PathBuf> {
    if !root.is_absolute() {
        return Err(invalid("Janex home must be absolute"));
    }
    Ok(root.join("bin").join(EXECUTABLE))
}

/// Serializes installation and update publication across native processes.
fn lock(root: &Path) -> Result<fs::File> {
    fs::create_dir_all(root.join("state"))?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("state/self.lock"))?;
    fs4::FileExt::lock(&file)?;
    Ok(file)
}

/// Refuses symlink replacement so externally managed installations are not redirected.
fn reject_link(target: &Path) -> Result<()> {
    match target.symlink_metadata() {
        Ok(meta) if !meta.is_file() || meta.is_symlink() => {
            Err(invalid("managed Janex path must be a regular file"))
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Removes completed Windows backups without traversing directories or touching active images.
fn cleanup(target: &Path) -> Result<()> {
    let parent = target.parent().unwrap();
    if !parent.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".janex-update-")
            && entry.file_type()?.is_dir()
        {
            // Only remove the known backup file and an empty directory; never traverse a tree.
            let _ = fs::remove_file(entry.path().join("old.exe"));
            let _ = fs::remove_dir(entry.path());
        }
    }
    Ok(())
}

/// Creates private staging on the destination filesystem.
fn staging(target: &Path) -> Result<tempfile::TempDir> {
    let parent = target.parent().unwrap();
    fs::create_dir_all(parent)?;
    Ok(tempfile::Builder::new()
        .prefix(".janex-update-")
        .tempdir_in(parent)?)
}

/// Publishes a complete replacement without writing into the running executable's inode.
fn replace(candidate: &Path, target: &Path, stage: tempfile::TempDir) -> Result<()> {
    reject_link(target)?;
    #[cfg(not(windows))]
    {
        fs::rename(candidate, target)?;
        drop(stage);
    }
    #[cfg(windows)]
    {
        let backup = stage.path().join("old.exe");
        let existed = target.exists();
        if existed {
            fs::rename(target, &backup)?;
        }
        if let Err(error) = fs::rename(candidate, target) {
            if existed && let Err(restore) = fs::rename(&backup, target) {
                let _ = stage.keep();
                return Err(invalid(format!(
                    "replacement failed: {error}; restore {} manually: {restore}",
                    backup.display()
                )));
            }
            return Err(error.into());
        }
        if existed && fs::remove_file(&backup).is_err() {
            // Preserve the running image without attempting recursive cleanup.
            let _ = stage.keep();
        }
    }
    Ok(())
}

/// Checks that the staged program can run on this host and identifies itself as Janex.
fn probe(path: &Path) -> Result<Version> {
    let mut magic = [0; 4];
    fs::File::open(path)?.read_exact(&mut magic)?;
    let native = if cfg!(windows) {
        magic.starts_with(b"MZ")
    } else if cfg!(target_os = "macos") {
        matches!(
            magic,
            [0xcf, 0xfa, 0xed, 0xfe] | [0xca, 0xfe, 0xba, 0xbe] | [0xbe, 0xba, 0xfe, 0xca]
        )
    } else {
        magic == *b"\x7fELF"
    };
    if !native {
        return Err(invalid(
            "distribution executable is not native to this operating system",
        ));
    }
    let mut command = Command::new(path);
    command.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(invalid("updated Janex cannot run on this system"));
    }
    let version = std::str::from_utf8(&output.stdout)
        .ok()
        .and_then(|v| v.trim().strip_prefix("janex "))
        .ok_or_else(|| invalid("distribution does not contain a Janex executable"))?;
    Version::parse(version).map_err(|e| invalid(format!("invalid Janex version: {e}")))
}

/// Reads a local distribution without allowing an unbounded allocation.
fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("Janex distribution exceeds byte limit"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_failure_keeps_the_previous_executable() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join(EXECUTABLE);
        fs::write(&target, b"previous").unwrap();
        let stage = staging(&target).unwrap();
        let missing = stage.path().join(EXECUTABLE);
        assert!(replace(&missing, &target, stage).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"previous");
    }

    #[cfg(unix)]
    #[test]
    fn installation_does_not_replace_external_symlink_targets() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("external");
        fs::write(&original, b"external").unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        std::os::unix::fs::symlink(&original, root.path().join("bin/janex")).unwrap();
        assert!(install(root.path(), &original).is_err());
        assert_eq!(fs::read(original).unwrap(), b"external");
    }
}

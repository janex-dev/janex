// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Bounded extraction into a new, private SDK staging tree.

use super::{JavaRequest, java_name};
use crate::{Result, error::invalid};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
};

/// Checks one portable filename component, including Windows device aliases.
pub(super) fn safe_component(text: &str) -> Result<()> {
    let stem = text.split('.').next().unwrap_or("").to_ascii_uppercase();
    if text.is_empty()
        || text == "."
        || text == ".."
        || text.len() > 255
        || text.ends_with(['.', ' '])
        || text
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        || matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err(invalid("unsafe SDK archive filename"));
    }
    Ok(())
}

/// Converts archive slash-separated names into a confined relative path.
fn relative(text: &str) -> Result<PathBuf> {
    if text.starts_with('/') || text.contains('\\') || text.len() > 4096 {
        return Err(invalid("SDK archive path escapes its root"));
    }
    let mut result = PathBuf::new();
    for part in text.trim_end_matches('/').split('/') {
        if part == "." {
            continue;
        }
        safe_component(part)?;
        result.push(part);
    }
    if result.as_os_str().is_empty() {
        return Err(invalid("empty SDK archive path"));
    }
    Ok(result)
}

/// A deferred archive link; regular contents are written before any links exist.
struct Link {
    /// Destination relative to the extraction root.
    path: PathBuf,
    /// Confined target relative to the extraction root.
    target: PathBuf,
    /// Original relative symlink payload; absent for hard links.
    symbolic: Option<String>,
}

/// Extracts a ZIP or tar.gz SDK, enforcing aggregate size, path, type, and link constraints.
pub(super) fn extract(input: fs::File, format: &str, root: &Path, limit: u64) -> Result<()> {
    let mut remaining = limit;
    let mut links = Vec::new();
    let mut seen = BTreeSet::new();
    match format {
        "zip" => {
            let mut zip = zip::ZipArchive::new(input)
                .map_err(|e| invalid(format!("invalid SDK ZIP: {e}")))?;
            if zip.len() > 200_000 {
                return Err(invalid("SDK archive entry limit exceeded"));
            }
            for index in 0..zip.len() {
                let mut entry = zip
                    .by_index(index)
                    .map_err(|e| invalid(format!("invalid SDK ZIP entry: {e}")))?;
                let path = relative(entry.name())?;
                let mode = entry.unix_mode().unwrap_or(0o644);
                let kind = mode & 0o170000;
                if entry.is_dir() {
                    fs::create_dir_all(root.join(path))?;
                    continue;
                }
                if !seen.insert(path.clone()) {
                    return Err(invalid("duplicate SDK archive entry"));
                }
                if kind == 0o120000 {
                    if entry.size() > 4096 {
                        return Err(invalid("oversized SDK symlink"));
                    }
                    let mut target = String::new();
                    entry.read_to_string(&mut target)?;
                    links.push(link(path, &target, true)?);
                } else if kind == 0 || kind == 0o100000 {
                    write_file(&mut entry, root, &path, mode, &mut remaining)?;
                } else {
                    return Err(invalid("unsupported SDK ZIP entry type"));
                }
            }
        }
        "tar.gz" => {
            let gzip = flate2::read::MultiGzDecoder::new(input);
            let budget = limit
                .checked_add(256 * 1024 * 1024)
                .ok_or_else(|| invalid("invalid SDK expansion limit"))?;
            let mut archive = tar::Archive::new(gzip.take(budget));
            for (index, entry) in archive.entries()?.enumerate() {
                if index >= 200_000 {
                    return Err(invalid("SDK archive entry limit exceeded"));
                }
                let mut entry = entry?;
                let path_bytes = entry.path_bytes();
                let name = std::str::from_utf8(&path_bytes)
                    .map_err(|_| invalid("SDK archive path is not UTF-8"))?;
                if name == "." || name == "./" {
                    continue;
                }
                let path = relative(name)?;
                let kind = entry.header().entry_type();
                if kind.is_dir() {
                    fs::create_dir_all(root.join(path))?;
                    continue;
                }
                if !seen.insert(path.clone()) {
                    return Err(invalid("duplicate SDK archive entry"));
                }
                if kind.is_file() {
                    let mode = entry.header().mode()?;
                    write_file(&mut entry, root, &path, mode, &mut remaining)?;
                } else if kind.is_symlink() || kind.is_hard_link() {
                    let target = entry
                        .link_name_bytes()
                        .ok_or_else(|| invalid("SDK link has no target"))?;
                    let target = std::str::from_utf8(&target)
                        .map_err(|_| invalid("SDK link target is not UTF-8"))?;
                    links.push(link(path, target, kind.is_symlink())?);
                } else {
                    return Err(invalid("unsupported SDK tar entry type"));
                }
            }
            // Consume the remaining gzip stream to validate its trailer and bound padding.
            let mut input = archive.into_inner();
            std::io::copy(&mut input, &mut std::io::sink())?;
            if input.limit() == 0 {
                return Err(invalid("SDK archive stream expansion limit exceeded"));
            }
        }
        _ => return Err(invalid("unsupported SDK archive format")),
    }
    let canonical = root.canonicalize()?;
    while !links.is_empty() {
        let mut pending = Vec::new();
        let count = links.len();
        for link in links {
            let target = root.join(&link.target);
            if !target.exists() {
                pending.push(link);
                continue;
            }
            if !target.canonicalize()?.starts_with(&canonical) {
                return Err(invalid("SDK link target escapes its root"));
            }
            let destination = root.join(&link.path);
            fs::create_dir_all(destination.parent().unwrap())?;
            if !destination
                .parent()
                .unwrap()
                .canonicalize()?
                .starts_with(&canonical)
            {
                return Err(invalid("SDK link parent escapes its root"));
            }
            if let Some(symbolic) = link.symbolic {
                #[cfg(unix)]
                std::os::unix::fs::symlink(symbolic, &destination)?;
                #[cfg(windows)]
                {
                    let _ = symbolic;
                    return Err(invalid(
                        "symbolic links in Windows SDK archives are unsupported",
                    ));
                }
            } else {
                if !target.is_file() {
                    return Err(invalid("SDK hard link target is not a regular file"));
                }
                fs::hard_link(target, destination)?;
            }
        }
        if pending.len() == count {
            return Err(invalid("cyclic or missing SDK archive link target"));
        }
        links = pending;
    }
    Ok(())
}

/// Writes a new regular file without preserving ownership or special permission bits.
fn write_file(
    input: &mut impl Read,
    root: &Path,
    path: &Path,
    mode: u32,
    remaining: &mut u64,
) -> Result<()> {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap())?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let count = std::io::copy(&mut input.take(remaining.saturating_add(1)), &mut file)?;
    *remaining = remaining
        .checked_sub(count)
        .ok_or_else(|| invalid("SDK extracted byte limit exceeded"))?;
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644 | (mode & 0o111)))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

/// Resolves a link lexically before any archive link has been created.
fn link(path: PathBuf, target: &str, symbolic: bool) -> Result<Link> {
    if target.is_empty() || target.starts_with('/') || target.contains('\\') || target.len() > 4096
    {
        return Err(invalid("unsafe SDK archive link"));
    }
    let mut resolved = if symbolic {
        path.parent().unwrap().to_owned()
    } else {
        PathBuf::new()
    };
    for part in target.split('/') {
        match part {
            "." => {}
            ".." => {
                if !resolved.pop() {
                    return Err(invalid("SDK link escapes archive root"));
                }
            }
            _ => {
                safe_component(part)?;
                resolved.push(part);
            }
        }
    }
    Ok(Link {
        path,
        target: resolved,
        symbolic: symbolic.then(|| target.into()),
    })
}

/// Finds exactly one SDK root with a release file and the requested tools, without executing Java.
pub(super) fn find_home(root: &Path, request: &super::SdkRequest) -> Result<PathBuf> {
    let super::SdkRequest::Java(request) = request else {
        return super::tools::find_home(root, request);
    };
    let mut homes = Vec::new();
    let mut pending = vec![(root.to_owned(), 0)];
    while let Some((path, depth)) = pending.pop() {
        if path.join("release").is_file()
            && path.join("bin").join(java_name()).is_file()
            && (request.kind == "jre"
                || path
                    .join("bin")
                    .join(if cfg!(windows) { "javac.exe" } else { "javac" })
                    .is_file())
        {
            validate_release(&path, request)?;
            homes.push(path);
            continue;
        }
        if depth < 4 {
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    pending.push((entry.path(), depth + 1));
                }
            }
        }
    }
    if homes.len() != 1 {
        return Err(invalid(
            "SDK archive must contain exactly one matching Java home",
        ));
    }
    Ok(homes.remove(0))
}

/// Checks archive-reported release and architecture against the selected package.
fn validate_release(home: &Path, request: &JavaRequest) -> Result<()> {
    let mut text = String::new();
    fs::File::open(home.join("release"))?
        .take(65537)
        .read_to_string(&mut text)?;
    if text.len() > 65536 {
        return Err(invalid("SDK release metadata is too large"));
    }
    let property = |name: &str| -> Result<String> {
        let values: Vec<_> = text
            .lines()
            .filter_map(|line| line.split_once('='))
            .filter(|(key, _)| *key == name)
            .collect();
        if values.len() != 1 {
            return Err(invalid(format!("missing or duplicate SDK {name}")));
        }
        Ok(values[0].1.trim().trim_matches('"').to_owned())
    };
    let mut version = property("JAVA_VERSION")?;
    if let Some(update) = version.strip_prefix("1.8.0_") {
        version = format!("8.0.{update}");
    }
    let mut release_request = request.clone();
    release_request.version = request.version.split('+').next().unwrap().into();
    if !release_request.matches(&version) {
        return Err(invalid("SDK release version does not match catalog"));
    }
    let arch = property("OS_ARCH")?;
    if janex_platform::normalize_architecture(&arch) != request.architecture {
        return Err(invalid("SDK architecture does not match catalog"));
    }
    Ok(())
}

/// Rewinds a downloaded archive and computes its persistent identity after checking the expected digest.
pub(super) fn verify(
    file: &mut fs::File,
    expected: &janex_format::checksum::Checksum,
) -> Result<String> {
    use janex_format::checksum::{Algorithm, Checksum};
    file.rewind()?;
    if Checksum::compute(expected.algorithm(), &mut *file)? != *expected {
        return Err(invalid("SDK archive checksum mismatch"));
    }
    file.rewind()?;
    let digest = if expected.algorithm() == Algorithm::Sha256 {
        expected.clone()
    } else {
        Checksum::compute(Algorithm::Sha256, &mut *file)?
    };
    file.rewind()?;
    Ok(super::hex(digest.digest()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_cross_platform_paths_and_escaping_links() {
        for name in [
            "../a",
            "/a",
            "C:/a",
            "a\\b",
            "a/../b",
            "a/NUL.txt",
            "a/x:stream",
            "a/x.",
        ] {
            assert!(relative(name).is_err(), "{name}");
        }
        assert!(link("jdk/lib/x".into(), "../../../outside", true).is_err());
        assert_eq!(
            link("jdk/lib/x".into(), "../legal", true).unwrap().target,
            PathBuf::from("jdk/legal")
        );
    }

    #[test]
    fn zip_limits_and_traversal_fail_without_writing_outside_staging() {
        use zip::{ZipWriter, write::SimpleFileOptions};
        for (name, limit) in [("../outside", 100), ("jdk/file", 2)] {
            let temp = tempfile::tempdir().unwrap();
            let archive = temp.path().join("archive.zip");
            let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(b"contents").unwrap();
            zip.finish().unwrap();
            let root = temp.path().join("tree");
            fs::create_dir(&root).unwrap();
            assert!(extract(fs::File::open(archive).unwrap(), "zip", &root, limit).is_err());
            assert!(!temp.path().join("outside").exists());
        }
    }

    #[test]
    fn tar_checksum_and_safe_links_are_checked() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("archive.tar.gz");
        let gzip = flate2::write::GzEncoder::new(
            fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(gzip);
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "jdk/bin/java", b"abc".as_slice())
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Link);
        header.set_size(0);
        header.set_mode(0o755);
        tar.append_link(&mut header, "jdk/bin/alias", "jdk/bin/java")
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let root = temp.path().join("tree");
        fs::create_dir(&root).unwrap();
        extract(fs::File::open(&path).unwrap(), "tar.gz", &root, 1024).unwrap();
        assert_eq!(fs::read(root.join("jdk/bin/alias")).unwrap(), b"abc");
        let mut bytes = fs::read(&path).unwrap();
        let index = bytes.len() - 8;
        bytes[index] ^= 1;
        fs::write(&path, bytes).unwrap();
        let root = temp.path().join("corrupt");
        fs::create_dir(&root).unwrap();
        assert!(extract(fs::File::open(&path).unwrap(), "tar.gz", &root, 1024).is_err());
    }
}

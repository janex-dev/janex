// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Extracts only the CLI executable from a Janex distribution.

use super::{EXECUTABLE, MAX_BYTES};
use crate::{Result, error::invalid};
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};

/// Extracts the one root executable, ignoring other entries without materializing their paths.
pub(super) fn extract(bytes: &[u8], name: &str, destination: &Path) -> Result<()> {
    let mut found = false;
    if name.ends_with(".zip") {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| invalid(e.to_string()))?;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|e| invalid(e.to_string()))?;
            if entry.name() != EXECUTABLE {
                continue;
            }
            if found
                || !matches!(entry.unix_mode().unwrap_or(0) & 0o170000, 0 | 0o100000)
                || entry.is_dir()
            {
                return Err(invalid("distribution executable must be one regular file"));
            }
            write(&mut entry, destination)?;
            found = true;
        }
    } else if name.ends_with(".tar.xz") {
        let reader = lzma_rust2::XzReader::new(Cursor::new(bytes), true);
        let mut archive = tar::Archive::new(reader.take(MAX_BYTES * 4 + 1));
        for entry in archive.entries()? {
            let mut entry = entry?;
            if entry.path_bytes().as_ref() != EXECUTABLE.as_bytes() {
                continue;
            }
            if found || !entry.header().entry_type().is_file() {
                return Err(invalid("distribution executable must be one regular file"));
            }
            write(&mut entry, destination)?;
            found = true;
        }
        // Finish XZ validation, including its index and stream checksum.
        let mut reader = archive.into_inner();
        std::io::copy(&mut reader, &mut std::io::sink())?;
        if reader.limit() == 0 {
            return Err(invalid("distribution expansion limit exceeded"));
        }
    } else {
        return Err(invalid("expected a Janex .zip or .tar.xz distribution"));
    }
    if !found {
        return Err(invalid(format!("distribution has no root {EXECUTABLE}")));
    }
    Ok(())
}

/// Writes a bounded executable into private staging and flushes it before publication.
fn write(input: &mut impl Read, path: &Path) -> Result<()> {
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let length = std::io::copy(&mut input.take(MAX_BYTES + 1), &mut output)?;
    if length == 0 || length > MAX_BYTES {
        return Err(invalid("invalid executable size"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output.set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    output.sync_all()?;
    Ok(())
}

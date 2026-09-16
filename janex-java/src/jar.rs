// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Bounded JAR entry reading without extraction or Janex resource interpretation.

use crate::{Error, Limits, Result, error::invalid};
use std::{
    collections::BTreeSet,
    io::{Cursor, Read},
};

/// One decoded archive entry retaining its raw UTF-8 name and optional Unix mode.
#[derive(Debug)]
pub struct Entry {
    /// Exact archive name, including a directory's trailing slash; not a validated host path.
    pub name: String,
    /// Complete uncompressed bytes, checked against the stored size and CRC.
    pub content: Vec<u8>,
    /// Stored Unix mode, including filesystem type bits, when available.
    pub unix_mode: Option<u32>,
}

/// Reads complete JAR members, rejecting encryption and duplicate raw names.
///
/// `limits` bounds each entry's content, name length, and the number of entries.
/// `max_total_bytes` bounds both encoded input and aggregate decoded content.
/// No entries are extracted, and names, node types, and link targets are not interpreted.
/// A failure returns no partial entry collection. CRCs do not establish publisher trust.
pub fn read(bytes: &[u8], limits: Limits, max_total_bytes: u64) -> Result<Vec<Entry>> {
    if bytes.len() as u64 > max_total_bytes {
        return Err(invalid("import byte limit exceeded"));
    }
    let offset = checked_archive_offset(bytes, limits)?;
    let config = zip::read::Config {
        archive_offset: zip::read::ArchiveOffset::Known(offset),
    };
    let mut archive =
        zip::ZipArchive::with_config(config, Cursor::new(bytes)).map_err(zip_error)?;
    check_central_names(
        bytes,
        archive.central_directory_start(),
        archive.len(),
        limits,
    )?;
    let mut entries = Vec::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(zip_error)?;
        if file.encrypted() {
            return Err(Error::Unsupported(
                "encrypted JAR entries are unsupported".into(),
            ));
        }
        limits.bytes(file.name_raw().len() as u64)?;
        let name = std::str::from_utf8(file.name_raw())
            .map_err(|_| invalid("JAR entry name is not UTF-8"))?
            .to_owned();
        // Some Maven JARs store -1 as an unspecified Unix mode.
        let unix_mode = file.unix_mode().filter(|mode| *mode != 0xffff);
        let expected = file.size();
        limits.bytes(expected)?;
        total = total
            .checked_add(expected)
            .filter(|total| *total <= max_total_bytes)
            .ok_or_else(|| invalid("aggregate import byte limit exceeded"))?;
        let content = read_bounded(&mut file, expected)?;
        if content.len() as u64 != expected {
            return Err(invalid("JAR entry size mismatch"));
        }
        entries.push(Entry {
            name,
            content,
            unix_mode,
        });
    }
    Ok(entries)
}

/// Locates the archive and bounds its declared entry count before allocating a ZIP directory index.
fn checked_archive_offset(bytes: &[u8], limits: Limits) -> Result<u64> {
    let end = (bytes.len().saturating_sub(65557)..bytes.len().saturating_sub(21))
        .rev()
        .find(|&offset| {
            bytes.get(offset..offset + 4) == Some(b"PK\x05\x06")
                && offset + 22 + usize::from(u16_at(bytes, offset + 20)) == bytes.len()
        })
        .ok_or_else(|| invalid("missing terminal ZIP end record"))?;
    if u16_at(bytes, end + 4) != 0 || u16_at(bytes, end + 6) != 0 {
        return Err(invalid("multidisk JAR is unsupported"));
    }
    let count = u16_at(bytes, end + 10);
    if count != u16_at(bytes, end + 8) {
        return Err(invalid("inconsistent ZIP entry count"));
    }
    if count != u16::MAX {
        limits.elements(u64::from(count))?;
    }
    let locator = end
        .checked_sub(20)
        .filter(|&offset| bytes.get(offset..offset + 4) == Some(b"PK\x06\x07"));
    if let Some(locator) = locator {
        let mut found = None;
        for offset in (0..locator.saturating_sub(55)).rev() {
            if bytes.get(offset..offset + 4) == Some(b"PK\x06\x06")
                && u64_at(bytes, offset + 4).checked_add(12) == Some((locator - offset) as u64)
            {
                limits.elements(u64_at(bytes, offset + 32))?;
                let base = (offset as u64)
                    .checked_sub(u64_at(bytes, locator + 8))
                    .ok_or_else(|| invalid("ZIP64 locator offset exceeds its physical position"))?;
                if found.replace(base).is_some() {
                    return Err(invalid("ambiguous ZIP64 end record"));
                }
            }
        }
        return found.ok_or_else(|| invalid("missing ZIP64 end record"));
    } else if count == u16::MAX {
        return Err(invalid("ZIP64 entry count has no locator"));
    }
    let directory_size = u64::from(u32_at(bytes, end + 12));
    let directory_offset = u64::from(u32_at(bytes, end + 16));
    (end as u64)
        .checked_sub(directory_size)
        .and_then(|start| start.checked_sub(directory_offset))
        .ok_or_else(|| invalid("ZIP directory exceeds its physical end"))
}

/// Checks raw central-directory names before the ZIP library's duplicate-name replacement hides them.
fn check_central_names(bytes: &[u8], start: u64, expected: usize, limits: Limits) -> Result<()> {
    let mut position =
        usize::try_from(start).map_err(|_| invalid("ZIP directory offset overflow"))?;
    let mut names = BTreeSet::new();
    while bytes.get(position..position.saturating_add(4)) == Some(b"PK\x01\x02") {
        let header = bytes
            .get(position..position.saturating_add(46))
            .ok_or_else(|| invalid("truncated ZIP directory header"))?;
        let name_length = usize::from(u16_at(header, 28));
        let tail = name_length + usize::from(u16_at(header, 30)) + usize::from(u16_at(header, 32));
        let end = position
            .checked_add(46)
            .and_then(|position| position.checked_add(tail))
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid("ZIP directory entry outside input"))?;
        let name = &bytes[position + 46..position + 46 + name_length];
        if !names.insert(name) {
            return Err(invalid("duplicate JAR entry name"));
        }
        limits.elements(names.len() as u64)?;
        position = end;
    }
    if names.len() != expected {
        return Err(invalid("ZIP directory entry count mismatch"));
    }
    Ok(())
}

/// Reads a stream completely, with at most one byte of lookahead beyond the limit.
fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("import byte limit exceeded"));
    }
    Ok(bytes)
}

/// Maps archive errors while retaining underlying operating-system failures.
fn zip_error(error: zip::result::ZipError) -> Error {
    match error {
        zip::result::ZipError::Io(error) => Error::Io(error),
        other => invalid(format!("invalid or unsupported JAR: {other}")),
    }
}

/// Reads a ZIP integer from an already bounded header.
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("bounded ZIP field"),
    )
}
/// Reads a ZIP integer from an already bounded header.
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounded ZIP field"),
    )
}
/// Reads a ZIP64 integer from an already bounded header.
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("bounded ZIP64 field"),
    )
}

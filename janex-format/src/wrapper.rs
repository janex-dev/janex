// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! JAR-tail boundary discovery using ZIP directory offsets rather than local-header scans.

use crate::{ErrorKind, Result, binary::Limits, container::read_at, error::invalid};
use std::io::{Read, Seek};

/// A validated end record's directory coordinates.
struct Directory {
    /// Physical JAR start, also the candidate Janex end.
    jar_start: u64,
    /// Physical central-directory start.
    start: u64,
    /// Physical central-directory end, before the ZIP end records.
    end: u64,
    /// Number of central-directory entries.
    count: u64,
}

/// Returns JAR starts whose end records and referenced directory records are consistent.
pub(crate) fn candidate_ends(
    source: &mut (impl Read + Seek),
    size: u64,
    limits: Limits,
) -> Result<Vec<u64>> {
    let start = size.saturating_sub(65557);
    let tail = read_at(source, start, size - start, limits)?;
    let mut ends = Vec::new();
    for index in 0..tail.len().saturating_sub(21) {
        if tail[index..index + 4] != *b"PK\x05\x06" {
            continue;
        }
        let record = &tail[index..index + 22];
        if index + 22 + usize::from(u16_at(record, 20)) != tail.len() {
            continue;
        }
        if u16_at(record, 4) != 0 || u16_at(record, 6) != 0 {
            continue;
        }
        let eocd = start + index as u64;
        let directories = directories(source, eocd, record, limits)?;
        for directory in directories {
            match validate_directory(source, &directory, limits) {
                Ok(()) => ends.push(directory.jar_start),
                Err(error) if error.kind() == ErrorKind::Invalid => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(ends)
}

/// Derives directory coordinates from a normal EOCD and an optional ZIP64 record sequence.
fn directories(
    source: &mut (impl Read + Seek),
    eocd: u64,
    record: &[u8],
    limits: Limits,
) -> Result<Vec<Directory>> {
    let count = u64::from(u16_at(record, 10));
    let length = u64::from(u32_at(record, 12));
    let offset = u64::from(u32_at(record, 16));
    let locator = if eocd >= 20 {
        read_at(source, eocd - 20, 20, limits)?
    } else {
        Vec::new()
    };
    if !locator.starts_with(b"PK\x06\x07") {
        if count != u64::from(u16_at(record, 8))
            || count == 65535
            || length == u64::from(u32::MAX)
            || offset == u64::from(u32::MAX)
        {
            return Ok(Vec::new());
        }
        return Ok(coordinates(eocd, length, offset, count)
            .into_iter()
            .collect());
    }
    if u32_at(&locator, 4) != 0 || u32_at(&locator, 16) != 1 {
        return Ok(Vec::new());
    }
    let relative_record = u64_at(&locator, 8);
    let locator_start = eocd - 20;
    let mut result = Vec::new();
    for position in zip64_records(source, relative_record, locator_start, limits)? {
        let fixed = read_at(source, position, 56, limits)?;
        let count64 = u64_at(&fixed, 32);
        let length64 = u64_at(&fixed, 40);
        let offset64 = u64_at(&fixed, 48);
        if u32_at(&fixed, 16) != 0
            || u32_at(&fixed, 20) != 0
            || u64_at(&fixed, 24) != count64
            || (count != 65535 && count != count64)
            || (u16_at(record, 8) != 65535 && u64::from(u16_at(record, 8)) != count64)
            || (length != u64::from(u32::MAX) && length != length64)
            || (offset != u64::from(u32::MAX) && offset != offset64)
        {
            continue;
        }
        if let Some(directory) = coordinates(position, length64, offset64, count64)
            && directory.jar_start.checked_add(relative_record) == Some(position)
        {
            result.push(directory);
        }
    }
    Ok(result)
}

/// Computes checked physical coordinates for a relative ZIP central directory.
fn coordinates(end: u64, length: u64, offset: u64, count: u64) -> Option<Directory> {
    let start = end.checked_sub(length)?;
    let jar_start = start.checked_sub(offset)?;
    Some(Directory {
        jar_start,
        start,
        end,
        count,
    })
}

/// Scans bounded chunks for ZIP64 EOCDs whose declared lengths end exactly at the locator.
fn zip64_records(
    source: &mut (impl Read + Seek),
    lower: u64,
    locator: u64,
    limits: Limits,
) -> Result<Vec<u64>> {
    let mut positions = Vec::new();
    let mut start = lower;
    while start.checked_add(56).is_some_and(|end| end <= locator) {
        let length = (locator - start).min(65536);
        let bytes = read_at(source, start, length, limits)?;
        for index in 0..bytes.len().saturating_sub(3) {
            if bytes[index..index + 4] != *b"PK\x06\x06" {
                continue;
            }
            let position = start + index as u64;
            if position.checked_add(56).is_none_or(|end| end > locator) {
                continue;
            }
            let header = read_at(source, position, 12, limits)?;
            let size = u64_at(&header, 4);
            if size >= 44
                && position
                    .checked_add(12)
                    .and_then(|position| position.checked_add(size))
                    == Some(locator)
            {
                limits.elements(positions.len() as u64 + 1)?;
                positions.push(position);
            }
        }
        if length <= 3 {
            break;
        }
        start += length - 3;
    }
    Ok(positions)
}

/// Checks central-directory framing and local-header/data ranges inside a candidate JAR.
fn validate_directory(
    source: &mut (impl Read + Seek),
    directory: &Directory,
    limits: Limits,
) -> Result<()> {
    limits.elements(directory.count)?;
    let mut position = directory.start;
    for _ in 0..directory.count {
        if position
            .checked_add(46)
            .is_none_or(|end| end > directory.end)
        {
            return Err(invalid("truncated ZIP directory"));
        }
        let header = read_at(source, position, 46, limits)?;
        if !header.starts_with(b"PK\x01\x02") {
            return Err(invalid("incorrect ZIP directory signature"));
        }
        let name_length = usize::from(u16_at(&header, 28));
        let extra_length = usize::from(u16_at(&header, 30));
        let variable_length = name_length + extra_length + usize::from(u16_at(&header, 32));
        let next = position
            .checked_add(46 + variable_length as u64)
            .filter(|&end| end <= directory.end)
            .ok_or_else(|| invalid("ZIP directory entry exceeds directory"))?;
        let variable = read_at(source, position + 46, variable_length as u64, limits)?;
        let mut uncompressed = u64::from(u32_at(&header, 24));
        let mut compressed = u64::from(u32_at(&header, 20));
        let mut local = u64::from(u32_at(&header, 42));
        let mut disk = u64::from(u16_at(&header, 34));
        expand_zip64(
            &variable[name_length..name_length + extra_length],
            &mut [
                (&mut uncompressed, u64::from(u32::MAX), 8),
                (&mut compressed, u64::from(u32::MAX), 8),
                (&mut local, u64::from(u32::MAX), 8),
                (&mut disk, 65535, 4),
            ],
        )?;
        if disk != 0 {
            return Err(invalid("multidisk ZIP is not a JAR tail"));
        }
        let local = directory
            .jar_start
            .checked_add(local)
            .filter(|&local| local < directory.start)
            .ok_or_else(|| invalid("ZIP local header is outside the JAR data region"))?;
        validate_local(
            source,
            directory,
            local,
            &header,
            &variable[..name_length],
            compressed,
            uncompressed,
            limits,
        )?;
        position = next;
    }
    if position < directory.end {
        if directory.end - position < 6 {
            return Err(invalid("trailing ZIP directory bytes"));
        }
        let signature = read_at(source, position, 6, limits)?;
        if !signature.starts_with(b"PK\x05\x05")
            || position + 6 + u64::from(u16_at(&signature, 4)) != directory.end
        {
            return Err(invalid("invalid ZIP directory digital signature record"));
        }
    }
    Ok(())
}

/// Expands only placeholder fields from a ZIP64 extra field, in their prescribed order.
fn expand_zip64(extra: &[u8], fields: &mut [(&mut u64, u64, usize)]) -> Result<()> {
    let mut position = 0;
    let mut expanded = false;
    while position < extra.len() {
        if extra.len() - position < 4 {
            return Err(invalid("truncated ZIP extra field"));
        }
        let id = u16_at(extra, position);
        let size = usize::from(u16_at(extra, position + 2));
        let value = extra
            .get(position + 4..position + 4 + size)
            .ok_or_else(|| invalid("ZIP extra field exceeds header"))?;
        if id == 1 {
            if expanded {
                return Err(invalid("duplicate ZIP64 extra field"));
            }
            expanded = true;
            let mut offset = 0;
            for (field, placeholder, width) in fields.iter_mut() {
                if **field == *placeholder {
                    let bytes = value
                        .get(offset..offset + *width)
                        .ok_or_else(|| invalid("missing ZIP64 extra-field value"))?;
                    **field = bytes
                        .iter()
                        .rev()
                        .fold(0, |value, byte| (value << 8) | u64::from(*byte));
                    offset += *width;
                }
            }
        }
        position += 4 + size;
    }
    if !expanded
        && fields
            .iter()
            .any(|(field, placeholder, _)| **field == *placeholder)
    {
        return Err(invalid("missing ZIP64 extra field"));
    }
    Ok(())
}

/// Checks a local header against the central descriptor and bounds the compressed data.
#[allow(clippy::too_many_arguments)]
fn validate_local(
    source: &mut (impl Read + Seek),
    directory: &Directory,
    position: u64,
    central: &[u8],
    name: &[u8],
    compressed: u64,
    uncompressed: u64,
    limits: Limits,
) -> Result<()> {
    if position
        .checked_add(30)
        .is_none_or(|end| end > directory.start)
    {
        return Err(invalid("truncated ZIP local header"));
    }
    let header = read_at(source, position, 30, limits)?;
    if !header.starts_with(b"PK\x03\x04")
        || u16_at(&header, 6) != u16_at(central, 8)
        || u16_at(&header, 8) != u16_at(central, 10)
    {
        return Err(invalid("ZIP local and central headers disagree"));
    }
    let name_length = usize::from(u16_at(&header, 26));
    let variable_length = name_length + usize::from(u16_at(&header, 28));
    let data_start = position
        .checked_add(30 + variable_length as u64)
        .filter(|&end| end <= directory.start)
        .ok_or_else(|| invalid("ZIP local fields exceed data region"))?;
    let variable = read_at(source, position + 30, variable_length as u64, limits)?;
    if &variable[..name_length] != name {
        return Err(invalid("ZIP local and central filenames disagree"));
    }
    let data_end = data_start
        .checked_add(compressed)
        .filter(|&end| end <= directory.start)
        .ok_or_else(|| invalid("ZIP compressed data exceeds JAR data region"))?;
    if u16_at(&header, 6) & 8 == 0 {
        let mut local_compressed = u64::from(u32_at(&header, 18));
        let mut local_uncompressed = u64::from(u32_at(&header, 22));
        expand_zip64(
            &variable[name_length..],
            &mut [
                (&mut local_uncompressed, u64::from(u32::MAX), 8),
                (&mut local_compressed, u64::from(u32::MAX), 8),
            ],
        )?;
        if local_compressed != compressed
            || local_uncompressed != uncompressed
            || u32_at(&header, 14) != u32_at(central, 16)
        {
            return Err(invalid("ZIP local sizes or CRC disagree with directory"));
        }
    } else {
        let length = (directory.start - data_end).min(24);
        let descriptor = read_at(source, data_end, length, limits)?;
        let valid = [0, 4].into_iter().any(|skip| {
            if skip == 4 && !descriptor.starts_with(b"PK\x07\x08") {
                return false;
            }
            let Some(bytes) = descriptor.get(skip..) else {
                return false;
            };
            if bytes.len() < 12 || u32_at(bytes, 0) != u32_at(central, 16) {
                return false;
            }
            (u64::from(u32_at(bytes, 4)) == compressed
                && u64::from(u32_at(bytes, 8)) == uncompressed)
                || (bytes.len() >= 20
                    && u64_at(bytes, 4) == compressed
                    && u64_at(bytes, 12) == uncompressed)
        });
        if !valid {
            return Err(invalid("invalid ZIP data descriptor"));
        }
    }
    Ok(())
}

/// Reads a little-endian field from a previously bounded record.
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("bounded record"),
    )
}
/// Reads a little-endian field from a previously bounded record.
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounded record"),
    )
}
/// Reads a little-endian field from a previously bounded record.
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("bounded record"),
    )
}

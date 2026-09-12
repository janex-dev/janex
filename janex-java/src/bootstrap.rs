// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Private Java bootstrap resources and lossless program-argument transport.

use crate::{Limits, launch::EntryPoint};
use crate::{Result, error::invalid};
use std::{
    ffi::OsString,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};
use zip::{ZipWriter, write::SimpleFileOptions};

/// Binary name of the Java 8-compatible bootstrap class.
pub(crate) const MAIN_CLASS: &str = "org.janex.bootstrap.Bootstrap";

/// Writes a private bootstrap JAR containing the entry point and original program arguments.
///
/// Strings use counted UTF-16 code units so Windows arguments never pass through an ANSI
/// encoding. Non-Unicode Unix arguments require direct launching. The caller owns the
/// parent directory and must retain it until the application exits.
pub(crate) fn write(
    directory: &Path,
    entry: &EntryPoint,
    arguments: &[OsString],
    instance_main: bool,
    limits: Limits,
) -> Result<PathBuf> {
    if entry.main_class.as_deref() == Some(MAIN_CLASS) {
        return Err(invalid(
            "the bootstrap class cannot be an application entry point",
        ));
    }
    limits.elements(arguments.len() as u64)?;
    let mut data = Vec::new();
    string(
        &mut data,
        entry.main_module.as_deref().unwrap_or("").encode_utf16(),
        limits,
    )?;
    string(
        &mut data,
        entry.main_class.as_deref().unwrap_or("").encode_utf16(),
        limits,
    )?;
    data.push(u8::from(instance_main));
    data.extend_from_slice(&count(arguments.len())?.to_be_bytes());
    for argument in arguments {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            string(&mut data, argument.encode_wide(), limits)?;
        }
        #[cfg(not(windows))]
        {
            let argument = argument.to_str().ok_or_else(|| invalid(
                "bootstrap arguments must be Unicode; use direct launching for native byte arguments"
            ))?;
            string(&mut data, argument.encode_utf16(), limits)?;
        }
    }
    limits.bytes(data.len() as u64)?;
    let path = directory.join("bootstrap.jar");
    let mut jar = ZipWriter::new(File::create(&path)?);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    jar.start_file("org/janex/bootstrap/Bootstrap.class", options)
        .map_err(std::io::Error::other)?;
    jar.write_all(include_bytes!("../bootstrap/Bootstrap.class"))?;
    jar.start_file("org/janex/bootstrap/launch.bin", options)
        .map_err(std::io::Error::other)?;
    jar.write_all(&data)?;
    jar.finish().map_err(std::io::Error::other)?;
    Ok(path)
}

/// Appends a bounded string without normalizing its UTF-16 code units.
fn string(
    output: &mut Vec<u8>,
    units: impl Iterator<Item = u16> + Clone,
    limits: Limits,
) -> Result<()> {
    let length = count(units.clone().count())?;
    let size = output.len() as u64 + 4 + u64::from(length) * 2;
    limits.bytes(size)?;
    output.extend_from_slice(&length.to_be_bytes());
    for unit in units {
        output.extend_from_slice(&unit.to_be_bytes());
    }
    Ok(())
}

/// Converts a collection length to the nonnegative Java array-size range.
fn count(length: usize) -> Result<u32> {
    i32::try_from(length)
        .map(|length| length as u32)
        .map_err(|_| invalid("bootstrap data exceeds the Java array-size limit"))
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Private Java bootstrap resources and lossless program-argument transport.

use crate::{Limits, launch::EntryPoint};
use crate::{Result, error::invalid};
use std::{
    ffi::OsString,
    fs::File,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};
use zip::{ZipWriter, write::SimpleFileOptions};

/// Binary name of the Java 8-compatible bootstrap class.
pub(crate) const MAIN_CLASS: &str = "org.glavo.janex.bootstrap.Bootstrap";

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
    resources: Option<&[u8]>,
    jvm_options: &[String],
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
    let mut embedded = zip::ZipArchive::new(Cursor::new(include_bytes!(
        "../../janex-bootstrap/build/libs/janex-bootstrap.jar"
    )))
    .map_err(std::io::Error::other)?;
    for index in 0..embedded.len() {
        let mut entry = embedded.by_index(index).map_err(std::io::Error::other)?;
        if entry.is_dir() {
            continue;
        }
        if resources.is_none()
            && entry.name() != "org/glavo/janex/bootstrap/Bootstrap.class"
            && entry.name() != "META-INF/MANIFEST.MF"
            && !entry.name().starts_with("META-INF/LICENSE")
        {
            continue;
        }
        jar.start_file(entry.name(), options)
            .map_err(std::io::Error::other)?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        jar.write_all(&bytes)?;
    }
    jar.start_file("org/glavo/janex/bootstrap/launch.bin", options)
        .map_err(std::io::Error::other)?;
    jar.write_all(&data)?;
    if let Some(resources) = resources {
        let mut option_data = Vec::new();
        string(
            &mut option_data,
            entry.main_module.as_deref().unwrap_or("").encode_utf16(),
            limits,
        )?;
        string(
            &mut option_data,
            entry.main_class.as_deref().unwrap_or("").encode_utf16(),
            limits,
        )?;
        option_data.extend(count(jvm_options.len())?.to_be_bytes());
        for option in jvm_options {
            string(&mut option_data, option.encode_utf16(), limits)?;
        }
        limits.bytes(option_data.len() as u64)?;
        jar.start_file(
            "org/glavo/janex/bootstrap/options.bin",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .map_err(std::io::Error::other)?;
        jar.write_all(&option_data)?;
        limits.bytes(resources.len() as u64)?;
        jar.start_file("org/glavo/janex/bootstrap/resources.bin", options)
            .map_err(std::io::Error::other)?;
        jar.write_all(resources)?;
    }
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

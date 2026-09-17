// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Private Java bootstrap resources and lossless program-argument transport.

use crate::{Limits, launch::EntryPoint};
use crate::{Result, error::invalid};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};
use zip::{ZipWriter, write::SimpleFileOptions};

/// Binary name of the Java 8-compatible bootstrap class.
pub(crate) const MAIN_CLASS: &str = "org.glavo.janex.bootstrap.Bootstrap";

/// Encodes launch data separately from a content-addressed, reusable bootstrap JAR.
///
/// Strings use counted UTF-16 code units so Windows arguments never pass through an ANSI
/// encoding. Non-Unicode Unix arguments require direct launching. No per-launch file is written.
pub(crate) fn write(
    _directory: &Path,
    entry: &EntryPoint,
    arguments: &[OsString],
    instance_main: bool,
    limits: Limits,
    resources: Option<&[u8]>,
    jvm_options: &[String],
) -> Result<(PathBuf, String)> {
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
    let mut option_data = Vec::new();
    let path = cached_jar(resources.is_some())?;
    if let Some(resources) = resources {
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
        limits.bytes(resources.len() as u64)?;
    }
    let mut payload = b"JNX1".to_vec();
    for section in [&data[..], &option_data[..], resources.unwrap_or(&[])] {
        payload.extend(count(section.len())?.to_be_bytes());
        payload.extend(section);
    }
    limits.bytes(payload.len() as u64)?;
    Ok((
        path,
        format!(
            "-Djanex.launch={}",
            base64::engine::general_purpose::STANDARD.encode(payload)
        ),
    ))
}

/// Publishes fixed launcher bytes atomically and repairs missing or modified cache entries.
fn cached_jar(resources: bool) -> Result<PathBuf> {
    const EMBEDDED: &[u8] = include_bytes!("../../janex-bootstrap/build/libs/janex-bootstrap.jar");
    let digest: String = Sha256::digest(EMBEDDED)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let directory = janex_platform::janex_home()?
        .join("cache/bootstrap")
        .join(digest);
    fs::create_dir_all(&directory)?;
    let path = directory.join(if resources {
        "bootstrap.jar"
    } else {
        "entry.jar"
    });
    let bytes = if resources {
        EMBEDDED.to_vec()
    } else {
        let mut source =
            zip::ZipArchive::new(Cursor::new(EMBEDDED)).map_err(std::io::Error::other)?;
        let mut output = ZipWriter::new(Cursor::new(Vec::new()));
        for name in [
            "org/glavo/janex/bootstrap/Bootstrap.class",
            "org/glavo/janex/bootstrap/LaunchData.class",
        ] {
            let mut entry = source.by_name(name).map_err(std::io::Error::other)?;
            output
                .start_file(name, SimpleFileOptions::default())
                .map_err(std::io::Error::other)?;
            std::io::copy(&mut entry, &mut output)?;
        }
        output.finish().map_err(std::io::Error::other)?.into_inner()
    };
    if fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.write_all(&bytes)?;
        if let Err(error) = temporary.persist(&path)
            && fs::read(&path).ok().as_deref() != Some(bytes.as_slice())
        {
            return Err(error.error.into());
        }
    }
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

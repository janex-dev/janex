// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Native executable prefixes and private configuration shared by packer and launcher.
//!
//! Executing the wrapper authorizes its embedded launch policy. Embedded signer pins bind the
//! payload to that wrapper; they do not independently establish the wrapper's publisher identity.

use crate::{
    Result,
    authentication::{MATERIAL_LIMITS, OpenPgpCertificate, SignerCertificate},
    error::invalid,
    pack::PackSigner,
    run::{ExecutionPlan, LaunchMode, RunOptions, prepare_snapshot},
};
use janex_format::{binary::Limits, cbor::Value, container::Reader};
use janex_java::runtime::JavaOptions;
use std::{
    ffi::OsString,
    fs::File,
    io::{Cursor, Read, Seek},
    path::Path,
};

/// Private configuration marker immediately before the Janex magic.
const CONFIG_MARK: &[u8; 8] = b"JNXBOOT1";
/// Maximum complete executable prefix, including configuration.
const MAX_HEADER: u64 = 256 * 1024 * 1024;
/// Bounds for the private deterministic CBOR configuration.
const CONFIG_LIMITS: Limits = Limits {
    max_bytes: 4 * 1024 * 1024,
    max_elements: 65_536,
    max_depth: 32,
};

/// Runtime overrides independent of application arguments and embedded trust policy.
#[derive(Clone, Debug, Default)]
pub struct LaunchOverrides {
    /// Explicit executable or home; automatic selection otherwise prefers native runtimes.
    pub java: JavaOptions,
    /// Override entry-point invocation without changing signer pins.
    pub launch_mode: Option<LaunchMode>,
}

/// Builds a PE or ELF prefix with launch policy covered by the package's header checksum.
pub(crate) fn header(
    path: &Path,
    application: &str,
    mode: LaunchMode,
    signer: Option<&PackSigner>,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_HEADER + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_HEADER {
        return Err(invalid("native launcher exceeds the byte limit"));
    }
    validate_executable(&bytes)?;
    let (kind, key) = match signer {
        None => (0, Vec::new()),
        Some(PackSigner::Cms(signer)) => (3, signer.certificate().to_der()?),
        Some(PackSigner::OpenPgp(signer)) => (2, signer.certificate().as_bytes().to_vec()),
    };
    let config = Value::map([
        (Value::uint(0), Value::text(application)),
        (
            Value::uint(1),
            Value::uint(match mode {
                LaunchMode::Bootstrap => 0,
                LaunchMode::Direct => 1,
            }),
        ),
        (Value::uint(2), Value::uint(kind)),
        (Value::uint(3), Value::bytes(&key)),
    ])?;
    CONFIG_LIMITS.bytes(config.as_bytes().len() as u64)?;
    if bytes.len() as u64 + config.as_bytes().len() as u64 + 12 > MAX_HEADER {
        return Err(invalid("native launcher header exceeds the byte limit"));
    }
    bytes.extend_from_slice(config.as_bytes());
    bytes.extend_from_slice(&(config.as_bytes().len() as u32).to_le_bytes());
    bytes.extend_from_slice(CONFIG_MARK);
    Ok(bytes)
}

/// Rejects non-native inputs and PE certificates invalidated by subsequent wrapping.
fn validate_executable(bytes: &[u8]) -> Result<()> {
    if bytes.starts_with(b"\x7fELF") && bytes.len() >= 52 && matches!(bytes[4], 1 | 2) {
        return Ok(());
    }
    if bytes.starts_with(b"MZ") && bytes.len() >= 64 {
        let pe = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
        let header = bytes
            .get(pe..)
            .ok_or_else(|| invalid("invalid PE header offset"))?;
        if header.len() < 26 || !header.starts_with(b"PE\0\0") {
            return Err(invalid("invalid PE launcher"));
        }
        let optional_length = u16::from_le_bytes(header[20..22].try_into().unwrap()) as usize;
        let optional = header
            .get(24..24 + optional_length)
            .ok_or_else(|| invalid("truncated PE optional header"))?;
        let directory = match optional.get(..2) {
            Some([0x0b, 0x01]) => 96,
            Some([0x0b, 0x02]) => 112,
            _ => return Err(invalid("unsupported PE optional header")),
        };
        let count = optional
            .get(directory - 4..directory)
            .ok_or_else(|| invalid("truncated PE directories"))?;
        if u32::from_le_bytes(count.try_into().unwrap()) > 4 {
            let certificate = optional
                .get(directory + 32..directory + 40)
                .ok_or_else(|| invalid("truncated PE certificate directory"))?;
            if certificate.iter().any(|byte| *byte != 0) {
                return Err(invalid(
                    "native launcher must not contain an Authenticode certificate table",
                ));
            }
        }
        return Ok(());
    }
    Err(invalid("native launcher must be a PE or ELF executable"))
}

/// Reads one bounded executable snapshot and prepares its embedded application.
///
/// Configuration and authentication use the same bytes. Bootstrap reopens the executable and
/// checks its content identity; the caller must keep it unchanged until Java exits.
/// Arguments follow the package's preset arguments unchanged. The invocation channel is `open`.
/// Missing, malformed, or unsupported configuration is an error.
pub fn prepare(
    mut source: File,
    target: &Path,
    arguments: Vec<OsString>,
    overrides: LaunchOverrides,
) -> Result<ExecutionPlan> {
    source.rewind()?;
    let mut options = RunOptions::new(target);
    if !source.metadata()?.is_file() {
        return Err(invalid("native launcher must be a regular file"));
    }
    let mut bytes = Vec::new();
    (&mut source)
        .take(options.max_snapshot_bytes + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > options.max_snapshot_bytes {
        return Err(invalid("native executable exceeds the snapshot byte limit"));
    }
    let reader = Reader::open_auto(Cursor::new(bytes), options.limits)?;
    let start = usize::try_from(reader.range().start)
        .map_err(|_| invalid("native header exceeds address space"))?;
    let bytes = reader.into_inner().into_inner();
    if start < 12 || bytes.get(start - 8..start) != Some(CONFIG_MARK.as_slice()) {
        return Err(invalid("missing native launcher configuration"));
    }
    let length = u32::from_le_bytes(bytes[start - 12..start - 8].try_into().unwrap()) as usize;
    let offset = (start - 12)
        .checked_sub(length)
        .ok_or_else(|| invalid("invalid native launcher configuration length"))?;
    let config = Value::from_bytes(&bytes[offset..start - 12], CONFIG_LIMITS)?;
    if config.as_map()?.len() != 4 {
        return Err(invalid("invalid native launcher configuration fields"));
    }
    let application = config.required(0)?;
    if application.as_text()?.is_empty() {
        return Err(invalid("empty native launcher application"));
    }
    options.application = Some(application.as_text()?.to_owned());
    options.launch_mode = match config.required(1)?.as_u64()? {
        0 => LaunchMode::Bootstrap,
        1 => LaunchMode::Direct,
        _ => return Err(invalid("unknown native launcher invocation mode")),
    };
    let key = config.required(3)?;
    match config.required(2)?.as_u64()? {
        0 if key.as_byte_string()?.is_empty() => options.allow_unsigned = true,
        2 => {
            options.openpgp_trust = Some(OpenPgpCertificate::decode(
                key.as_byte_string()?,
                MATERIAL_LIMITS,
            )?)
        }
        3 => options.cms_trust.signers.push(SignerCertificate::decode(
            key.as_byte_string()?,
            MATERIAL_LIMITS,
        )?),
        _ => return Err(invalid("invalid native launcher trust policy")),
    }
    options.java = overrides.java;
    if let Some(mode) = overrides.launch_mode {
        options.launch_mode = mode;
    }
    options.arguments = arguments;
    options.invocation = "open".into();
    prepare_snapshot(&options, bytes)
}

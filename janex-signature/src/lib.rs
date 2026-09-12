// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Signature mechanisms and explicit caller-supplied authentication policy.

mod binary;
mod error;
pub use binary::Limits;
pub use error::{Error, ErrorKind, Result};

pub mod cms;
pub mod openpgp;

use crate::error::invalid;
use der::{AnyRef, Decode, Encode, Reader, SliceReader, Tagged};

/// Checks DER nesting and value counts before allocating decoded collections.
fn der_limits(bytes: &[u8], limits: Limits) -> Result<()> {
    limits.bytes(bytes.len() as u64)?;
    let mut count = 0;
    walk_der(bytes, limits, 0, &mut count)
}

/// Visits constructed DER values without interpreting primitive payloads as nested encodings.
fn walk_der(bytes: &[u8], limits: Limits, depth: usize, count: &mut u64) -> Result<()> {
    if depth > limits.max_depth {
        return Err(Error::new(ErrorKind::Limit, "DER nesting limit exceeded"));
    }
    let mut reader = SliceReader::new(bytes).map_err(der_error)?;
    while !reader.is_finished() {
        *count += 1;
        limits.elements(*count)?;
        let value = AnyRef::decode(&mut reader).map_err(der_error)?;
        if value.tag().is_constructed() {
            walk_der(value.value(), limits, depth + 1, count)?;
        }
    }
    Ok(())
}

/// Decodes a single canonical DER value after applying structural limits.
fn decode_der<T: for<'a> Decode<'a> + Encode>(bytes: &[u8], limits: Limits) -> Result<T> {
    der_limits(bytes, limits)?;
    let value = T::from_der(bytes).map_err(der_error)?;
    if value.to_der().map_err(der_error)? != bytes {
        return Err(invalid("noncanonical DER encoding"));
    }
    Ok(value)
}

/// Maps ASN.1 errors without including decoded key material in diagnostics.
fn der_error(error: der::Error) -> Error {
    invalid(format!("invalid DER: {error}"))
}

/// Reports an unsupported signature mechanism or parameter combination.
fn unsupported(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Unsupported, message)
}

/// Reports a cryptographic signature or signed digest mismatch.
fn verification(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Verification, message)
}

/// Reports rejection by caller-supplied identity, time, or key-usage policy.
fn trust(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Trust, message)
}

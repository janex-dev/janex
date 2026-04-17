// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use std::fmt::{Display, Formatter};

/// Errors produced while decoding, validating, or encoding Janex data.
#[derive(Debug)]
pub enum Error {
    /// The input ended before the expected number of bytes could be read.
    UnexpectedEndOfFile,
    /// A fixed magic number in the input did not match the expected value.
    InvalidMagicNumber { expected: u64, actual: u64 },
    /// A `vuint` encoding was malformed or exceeded the supported width.
    InvalidVUInt,
    /// UTF-8 decoding failed while parsing a Janex string.
    InvalidUtf8(std::string::FromUtf8Error),
    /// The input requested a format feature that this implementation does not support.
    UnsupportedFeature(&'static str),
    /// A class-file constant-pool tag was not recognized.
    UnknownConstantPoolInfo { tag: u8 },
    /// A Janex enum discriminant or type tag was not recognized.
    UnknownEnumValue { name: &'static str, value: u64 },
    /// A field or payload contained a semantically invalid value.
    InvalidValue(&'static str),
    /// A field or runtime input contained a semantically invalid value with detailed context.
    InvalidValueMessage(String),
    /// A checksum payload had the wrong byte length for its declared algorithm.
    InvalidChecksumLength { expected: u64, actual: u64 },
    /// The overall Janex file or section layout was invalid.
    InvalidSectionLayout(String),
    /// A cross-reference pointed to a missing or out-of-range target.
    InvalidReference(String),
    /// Metadata or resource verification failed.
    VerificationFailed(String),
    /// Parsing a CEL condition failed.
    ConditionParse(String),
    /// Executing a CEL condition failed.
    ConditionExecution(String),
    /// Compression or decompression failed.
    CompressionError(String),
    /// An underlying I/O operation failed.
    Io(std::io::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnexpectedEndOfFile => f.write_str("unexpected end of file"),
            Error::InvalidMagicNumber { expected, actual } => {
                write!(
                    f,
                    "invalid magic number: expected 0x{expected:016x}, got 0x{actual:016x}"
                )
            }
            Error::InvalidVUInt => f.write_str("invalid variable-length integer"),
            Error::InvalidUtf8(error) => write!(f, "invalid UTF-8 string: {error}"),
            Error::UnsupportedFeature(feature) => write!(f, "unsupported feature: {feature}"),
            Error::UnknownConstantPoolInfo { tag } => {
                write!(f, "unknown constant-pool entry tag: 0x{tag:02x}")
            }
            Error::UnknownEnumValue { name, value } => {
                write!(f, "unknown {name} value: 0x{value:x}")
            }
            Error::InvalidValue(message) => write!(f, "invalid value: {message}"),
            Error::InvalidValueMessage(message) => write!(f, "invalid value: {message}"),
            Error::InvalidChecksumLength { expected, actual } => write!(
                f,
                "invalid checksum length: expected {expected}, got {actual}"
            ),
            Error::InvalidSectionLayout(message) => write!(f, "invalid section layout: {message}"),
            Error::InvalidReference(message) => write!(f, "invalid reference: {message}"),
            Error::VerificationFailed(message) => write!(f, "verification failed: {message}"),
            Error::ConditionParse(message) => write!(f, "condition parse failed: {message}"),
            Error::ConditionExecution(message) => {
                write!(f, "condition execution failed: {message}")
            }
            Error::CompressionError(message) => write!(f, "compression error: {message}"),
            Error::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::InvalidUtf8(error) => Some(error),
            Error::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(value: std::string::FromUtf8Error) -> Self {
        Error::InvalidUtf8(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}

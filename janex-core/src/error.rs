// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use std::fmt::{Display, Formatter};

/// Errors produced while decoding, validating, or encoding Janex data.
#[derive(Debug)]
pub enum Error {
    UnexpectedEndOfFile,
    InvalidMagicNumber { expected: u64, actual: u64 },
    InvalidVUInt,
    InvalidUtf8(std::string::FromUtf8Error),
    UnsupportedFeature(&'static str),
    UnknownConstantPoolInfo { tag: u8 },
    UnknownEnumValue { name: &'static str, value: u64 },
    InvalidValue(&'static str),
    InvalidChecksumLength { expected: u64, actual: u64 },
    InvalidSectionLayout(String),
    InvalidReference(String),
    VerificationFailed(String),
    CompressionError(String),
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
            Error::InvalidChecksumLength { expected, actual } => write!(f, "invalid magic number: expected {expected}, got 0x{actual}"),
            Error::InvalidSectionLayout(message) => write!(f, "invalid section layout: {message}"),
            Error::InvalidReference(message) => write!(f, "invalid reference: {message}"),
            Error::VerificationFailed(message) => write!(f, "verification failed: {message}"),
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

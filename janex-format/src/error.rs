// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use std::{fmt, io};

/// A format operation's result.
pub type Result<T> = std::result::Result<T, Error>;

/// The category of a decoding, encoding, or verification failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The input is truncated or violates the format.
    Invalid,
    /// The operation requires an unsupported format feature.
    Unsupported,
    /// A configured resource limit would be exceeded.
    Limit,
    /// A digest or signature does not match its authenticated input.
    Verification,
    /// Reading or writing the underlying stream failed.
    Io,
}

/// A failure with a diagnostic context and an optional byte offset.
///
/// Offsets refer to the input passed to the failing operation unless its documentation
/// specifies an enclosing stream. An I/O failure retains its original source error.
#[derive(Debug)]
pub struct Error {
    /// The failure category.
    kind: ErrorKind,
    /// The byte offset, when available.
    offset: Option<u64>,
    /// The operation or violated constraint.
    context: String,
    /// The underlying stream failure.
    source: Option<io::Error>,
}

impl Error {
    /// Creates a format error without an underlying I/O error.
    pub fn new(kind: ErrorKind, context: impl Into<String>) -> Self {
        Self {
            kind,
            offset: None,
            context: context.into(),
            source: None,
        }
    }

    /// Associates the error with a byte offset in the operation's input.
    pub fn at(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Adds enclosing operation context while retaining the original cause and offset.
    pub fn context(mut self, context: impl fmt::Display) -> Self {
        self.context = format!("{context}: {}", self.context);
        self
    }

    /// Returns the failure category.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the recorded byte offset, if available.
    pub fn offset(&self) -> Option<u64> {
        self.offset
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.context)?;
        if let Some(offset) = self.offset {
            write!(f, " at byte {offset}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|source| source as _)
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Self {
            kind: ErrorKind::Io,
            offset: None,
            context: source.to_string(),
            source: Some(source),
        }
    }
}

/// Reports a violation of the current format.
pub(crate) fn invalid(context: impl Into<String>) -> Error {
    Error::new(ErrorKind::Invalid, context)
}

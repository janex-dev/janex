// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use std::io;

/// A packaging or launch service result.
pub type Result<T> = std::result::Result<T, Error>;

/// A service failure preserving format and operating-system causes.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An operating-system read, write, or process operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Input or output violates the Janex format or configured format limits.
    #[error(transparent)]
    Format(#[from] janex_format::Error),
    /// A Java runtime, manifest, or launch operation failed.
    #[error(transparent)]
    Java(#[from] janex_java::Error),
    /// A signature or certificate operation failed.
    #[error(transparent)]
    Signature(#[from] janex_signature::Error),
    /// Host policy rejects the publisher or authenticated coverage.
    #[error("{0}")]
    Trust(String),
    /// Input cannot be represented or does not meet the service's requirements.
    #[error("{0}")]
    InvalidInput(String),
    /// A valid input requires an unimplemented service capability.
    #[error("{0}")]
    Unsupported(String),
}

/// Creates a diagnostic for input that does not meet service requirements.
pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidInput(message.into())
}

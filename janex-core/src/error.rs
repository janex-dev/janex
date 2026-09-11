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

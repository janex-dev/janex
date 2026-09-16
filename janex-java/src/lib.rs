// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Java runtime discovery, JAR manifests, and native or bootstrap launch preparation.
//!
//! This crate does not read Janex containers or select publisher trust.

mod bootstrap;
mod error;
pub mod jar;
pub mod launch;
mod limits;
pub mod manifest;
pub mod modules;
pub mod runtime;
mod runtime_cache;

pub use error::{Error, Result};
pub use limits::Limits;

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Core Janex file-format types, binary codecs, and validation logic.

pub mod checksum;
pub mod classfile;
pub mod condition;
pub mod error;
pub mod io;
pub mod janex;
mod section;
pub use section::string_pool;

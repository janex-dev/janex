// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Reads, writes, and validates Janex 0.1 containers.
//!
//! Container decoding and checksum validation do not establish publisher trust or execute code.
//! Callers control input ownership, parsing limits, and authentication policy.
//!
//! # Writing and reading a blob
//!
//! ```
//! use std::io::Cursor;
//! use janex_format::{binary::Limits, blob::{BlobRef, BlobStore, PoolBuilder},
//!     cbor::Value, container::{BLOB_POOL, Reader, Writer}};
//!
//! let mut pool = PoolBuilder::new();
//! let index = pool.push(b"shared resource bytes", 3)?;
//! let pool = pool.finish(8, 3)?;
//! let mut writer = Writer::new(Vec::new())?;
//! writer.write_section(7, BLOB_POOL, &pool.bytes, Some(pool.type_info))?;
//! let bytes = writer.finish(Value::empty_map())?;
//!
//! let mut reader = Reader::open_auto(Cursor::new(bytes), Limits::default())?;
//! reader.verify_checksums()?;
//! let mut blobs = BlobStore::new(reader);
//! assert_eq!(blobs.resolve(BlobRef { pool: 7, index })?, b"shared resource bytes");
//! # Ok::<(), janex_format::Error>(())
//! ```
//!
//! This example detects corruption through checksums. Publisher authentication additionally
//! requires a trusted signature and secure checksum coverage of the complete container.

pub mod application;
pub mod binary;
pub mod blob;
pub mod cbor;
pub mod checksum;
pub mod classfile;
pub mod condition;
pub mod container;
pub mod content;
pub mod data_pool;
mod error;
pub mod localized;
pub mod purl;
pub mod resource;
pub mod strings;
pub mod version;
mod wrapper;

pub use error::{Error, ErrorKind, Result};

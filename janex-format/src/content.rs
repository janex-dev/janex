// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Inline or blob-backed content and lossless logical transforms.

use crate::{
    Result,
    binary::{self, Decoder},
    blob::{BlobRef, BlobStore},
    cbor::{self, Value},
    classfile,
    container::integer_keys,
    data_pool::DataPool,
    error::invalid,
};
use std::io::{Read, Seek};

/// The complete transformed byte sequence, inline or referenced by blob.
#[derive(Clone, Debug)]
pub enum Source {
    /// Bytes stored directly in the containing binary structure.
    Inline(Vec<u8>),
    /// All resolved bytes of one logical blob.
    Blob(BlobRef),
}

/// A logical transform, recorded in the order in which the encoder applied it.
#[derive(Clone, Debug)]
pub struct Transform {
    /// Required byte length after reversing this transform.
    pub input_size: u64,
    /// Transform identifier; CLASSFILE is 1.
    pub method: u8,
    /// Deterministic integer-keyed properties, including any unknown fields.
    pub properties: Value,
}

impl Transform {
    /// Validates the supported transform and any explicit data-pool reference.
    fn validate(&self) -> Result<()> {
        integer_keys(&self.properties)?;
        if self.method != 1 {
            return Err(invalid("unsupported content transform"));
        }
        if let Some(pool) = self.properties.get(0)? {
            BlobRef::from_value(&pool)?;
        }
        Ok(())
    }
}

/// Encoded content with zero or more reversible logical transforms.
#[derive(Clone, Debug)]
pub struct Content {
    /// Complete bytes after applying the transforms.
    pub source: Source,
    /// Transforms in encoding order, reversed from last to first when reading.
    pub transforms: Vec<Transform>,
}

impl Content {
    /// Creates inline content without transforms, including the canonical empty-content form.
    pub fn inline(bytes: Vec<u8>) -> Self {
        Self {
            source: Source::Inline(bytes),
            transforms: Vec::new(),
        }
    }

    /// Creates content naming a complete blob without transforms.
    pub fn blob(reference: BlobRef) -> Self {
        Self {
            source: Source::Blob(reference),
            transforms: Vec::new(),
        }
    }

    /// Reads one content descriptor, rejecting unknown sources and unsupported transforms.
    pub fn read(decoder: &mut Decoder<'_>) -> Result<Self> {
        let source = match decoder.u8()? {
            0 => Source::Inline(decoder.sized()?.to_vec()),
            1 => Source::Blob(BlobRef::read(decoder)?),
            _ => return Err(invalid("unknown content source")),
        };
        let count = decoder.count()?;
        let mut transforms = Vec::new();
        for _ in 0..count {
            let transform = Transform {
                input_size: decoder.vuint()?,
                method: decoder.u8()?,
                properties: cbor::read_sized(decoder)?,
            };
            decoder.limits().bytes(transform.input_size)?;
            transform.validate()?;
            transforms.push(transform);
        }
        Ok(Self { source, transforms })
    }

    /// Appends a validated descriptor; referenced blobs are not loaded or checked here.
    pub fn write(&self, bytes: &mut Vec<u8>) -> Result<()> {
        for transform in &self.transforms {
            transform.validate()?;
        }
        match &self.source {
            Source::Inline(value) => {
                bytes.push(0);
                binary::write_sized(&mut *bytes, value)?;
            }
            Source::Blob(reference) => {
                bytes.push(1);
                reference.write(bytes)?;
            }
        }
        binary::write_vuint(&mut *bytes, self.transforms.len() as u64)?;
        for transform in &self.transforms {
            binary::write_vuint(&mut *bytes, transform.input_size)?;
            bytes.push(transform.method);
            cbor::write_sized(&mut *bytes, &transform.properties)?;
        }
        Ok(())
    }

    /// Resolves directory-entry bytes, rejecting any logical transform.
    ///
    /// The caller must decode the declared number of directory entries and consume all bytes.
    pub fn resolve_entries<R: Read + Seek>(&self, blobs: &mut BlobStore<R>) -> Result<Vec<u8>> {
        if !self.transforms.is_empty() {
            return Err(invalid("directory entries cannot have content transforms"));
        }
        self.resolve_source(blobs)
    }

    /// Restores logical file bytes using the root pool unless a transform selects another pool.
    ///
    /// Every intermediate size is checked. An invalid explicit pool fails without fallback.
    /// Resource checksums are checked separately by the resource-tree reader.
    pub fn resolve_file<R: Read + Seek>(
        &self,
        blobs: &mut BlobStore<R>,
        default_pool: &DataPool,
    ) -> Result<Vec<u8>> {
        let limits = blobs.reader().limits();
        let mut bytes = self.resolve_source(blobs)?;
        for transform in self.transforms.iter().rev() {
            limits.bytes(transform.input_size)?;
            transform.validate()?;
            let explicit;
            let pool = if let Some(reference) = transform.properties.get(0)? {
                explicit =
                    DataPool::decode(&blobs.resolve(BlobRef::from_value(&reference)?)?, limits)?;
                &explicit
            } else {
                default_pool
            };
            bytes = classfile::restore(&bytes, pool, limits)?;
            if bytes.len() as u64 != transform.input_size {
                return Err(invalid("content transform size mismatch"));
            }
        }
        Ok(bytes)
    }

    /// Loads the complete encoded bytes within the reader's byte limit.
    fn resolve_source<R: Read + Seek>(&self, blobs: &mut BlobStore<R>) -> Result<Vec<u8>> {
        match &self.source {
            Source::Inline(bytes) => {
                blobs.reader().limits().bytes(bytes.len() as u64)?;
                Ok(bytes.clone())
            }
            Source::Blob(reference) => blobs.resolve(*reference),
        }
    }
}

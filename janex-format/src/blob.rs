// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Paged blob lookup, bounded decompression, and logical extent assembly.
//!
//! Table pages are loaded on demand and cached for the lifetime of a [`BlobStore`].
//! Section authentication is independent of this decoding layer.

use crate::{
    Error, ErrorKind, Result,
    binary::{self, Decoder, Limits},
    cbor::{self, Value},
    checksum::Checksum,
    container::{self, Reader, integer_keys},
    error::invalid,
};
use std::{
    collections::BTreeMap,
    io::{Read, Seek},
};

/// A logical blob identified by pool section ID and zero-based table index.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct BlobRef {
    /// The BlobPool section ID, not its position in the section table.
    pub pool: u64,
    /// The logical blob's table index.
    pub index: u64,
}

impl BlobRef {
    /// Reads two consecutive binary ULEB128 values.
    pub fn read(decoder: &mut Decoder<'_>) -> Result<Self> {
        Ok(Self {
            pool: decoder.vuint()?,
            index: decoder.vuint()?,
        })
    }
    /// Appends the binary representation to a byte vector.
    pub fn write(self, bytes: &mut Vec<u8>) -> Result<()> {
        binary::write_vuint(&mut *bytes, self.pool)?;
        binary::write_vuint(bytes, self.index)
    }
    /// Parses the two-element CBOR reference array.
    pub fn from_value(value: &Value) -> Result<Self> {
        let items = value.as_array()?;
        if items.len() != 2 {
            return Err(invalid("blob reference must have two elements"));
        }
        Ok(Self {
            pool: items[0].as_u64()?,
            index: items[1].as_u64()?,
        })
    }
    /// Encodes the native CBOR reference array.
    pub fn to_value(self) -> Value {
        Value::array([Value::uint(self.pool), Value::uint(self.index)])
    }
}

/// An independently reversible filter in encoding order.
#[derive(Clone, Debug)]
pub struct Filter {
    /// Byte count before this filter was applied by the encoder.
    pub input_size: u64,
    /// Filter ID; version 0.1 defines only ZSTD (1).
    pub method: u8,
    /// Complete filter properties, retaining unknown integer fields.
    pub properties: Value,
}

/// The stored length and filters describing an encoded byte range.
#[derive(Clone, Debug)]
pub struct Encoding {
    /// Encoded byte length.
    pub stored_size: u64,
    /// Filters in encoding order, reversed during decoding.
    pub filters: Vec<Filter>,
}

impl Encoding {
    /// Reads an encoding descriptor from a binary field.
    pub fn read(decoder: &mut Decoder<'_>) -> Result<Self> {
        let stored_size = decoder.vuint()?;
        let count = decoder.count()?;
        let mut filters = Vec::new();
        for _ in 0..count {
            let input_size = decoder.vuint()?;
            let method = decoder.u8()?;
            let properties = cbor::read_sized(decoder)?;
            integer_keys(&properties)?;
            if method != 1 {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!("blob filter {method}"),
                ));
            }
            if let Some(dictionary) = properties.get(0)? {
                BlobRef::from_value(&dictionary)?;
            }
            filters.push(Filter {
                input_size,
                method,
                properties,
            });
        }
        Ok(Self {
            stored_size,
            filters,
        })
    }

    /// Appends this descriptor's binary representation.
    pub fn write(&self, bytes: &mut Vec<u8>) -> Result<()> {
        binary::write_vuint(&mut *bytes, self.stored_size)?;
        binary::write_vuint(&mut *bytes, self.filters.len() as u64)?;
        for filter in &self.filters {
            binary::write_vuint(&mut *bytes, filter.input_size)?;
            bytes.push(filter.method);
            cbor::write_sized(&mut *bytes, &filter.properties)?;
        }
        Ok(())
    }

    /// Returns the binary descriptor wrapped in a CBOR byte string.
    pub fn to_value(&self) -> Result<Value> {
        let mut bytes = Vec::new();
        self.write(&mut bytes)?;
        Ok(Value::bytes(&bytes))
    }

    /// Reads the binary descriptor inside a CBOR byte string, rejecting trailing bytes.
    pub fn from_value(value: &Value, limits: Limits) -> Result<Self> {
        let mut decoder = Decoder::new(value.as_byte_string()?, limits)?;
        let encoding = Self::read(&mut decoder)?;
        decoder.finish()?;
        Ok(encoding)
    }

    /// Returns the logical decoded byte length.
    pub fn decoded_size(&self) -> u64 {
        self.filters
            .first()
            .map_or(self.stored_size, |filter| filter.input_size)
    }

    /// Returns whether any filter explicitly references an external dictionary.
    fn has_dictionary(&self) -> Result<bool> {
        for filter in &self.filters {
            if filter.properties.get(0)?.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// A decoded range of a Stored entry in the same pool.
#[derive(Clone, Copy, Debug)]
pub struct Extent {
    /// The source Stored entry's index.
    pub stored_blob_index: u64,
    /// The start within the source's decoded bytes.
    pub decoded_offset: u64,
    /// The nonzero number of decoded bytes to include.
    pub decoded_length: u64,
}

/// A logical blob-table entry.
#[derive(Clone, Debug)]
pub enum Entry {
    /// An independently stored encoded range.
    Stored {
        /// Offset after the pool's eight-byte magic.
        offset: u64,
        /// The encoded length and reversible filters.
        encoding: Encoding,
    },
    /// A concatenation of nonempty ranges from Stored entries in the same pool.
    Extents(Vec<Extent>),
    /// An unsupported entry retained only to preserve its position in the table.
    Unknown(u8),
}

/// One table page's metadata descriptor, without a claim of publisher authentication.
#[derive(Clone)]
struct Page {
    /// Offset relative to bytes after the pool magic.
    offset: u64,
    /// Length and self-contained filters.
    encoding: Encoding,
    /// Optional checksum of the decoded table bytes.
    checksum: Option<Checksum>,
}

/// Metadata and validated pages for one pool.
struct Pool {
    /// The number of logical table entries.
    count: u64,
    /// Base-two logarithm of the number of entries per page.
    shift: u8,
    /// Bytes available after the pool magic.
    size: u64,
    /// Physical page descriptors in logical page order.
    pages: Vec<Page>,
    /// Decoded pages keyed by their logical page number.
    cache: BTreeMap<usize, Vec<Entry>>,
    /// Validated nonempty page and Stored ranges, keyed by offset with exclusive ends.
    ranges: BTreeMap<u64, u64>,
}

/// A container reader with lazy blob-table page caches.
///
/// This object owns the reader and shares its immutable-input requirement. Blob contents
/// are decoded on request, not cached. [`Self::validate_pool`] checks every table page and
/// cross-entry range constraint without decoding all blob contents.
pub struct BlobStore<R> {
    /// The owned container reader.
    reader: Reader<R>,
    /// Metadata and page caches of pools that have been referenced.
    pools: BTreeMap<u64, Pool>,
}

impl<R: Read + Seek> BlobStore<R> {
    /// Creates an empty page cache around a container reader.
    pub fn new(reader: Reader<R>) -> Self {
        Self {
            reader,
            pools: BTreeMap::new(),
        }
    }
    /// Borrows the underlying reader for metadata inspection.
    pub fn reader(&self) -> &Reader<R> {
        &self.reader
    }
    /// Returns the underlying reader, dropping the page caches.
    pub fn into_reader(self) -> Reader<R> {
        self.reader
    }

    /// Returns a logical entry, loading only its table page if it is not cached.
    pub fn entry(&mut self, reference: BlobRef) -> Result<Entry> {
        self.open_pool(reference.pool)?;
        let pool = &self.pools[&reference.pool];
        if reference.index >= pool.count {
            return Err(invalid("blob index exceeds pool count"));
        }
        let page_index = (reference.index >> pool.shift) as usize;
        let entry_index = (reference.index & ((1 << pool.shift) - 1)) as usize;
        self.load_page(reference.pool, page_index)?;
        Ok(self.pools[&reference.pool].cache[&page_index][entry_index].clone())
    }

    /// Resolves one complete logical blob, enforcing the configured decoded-byte limit.
    ///
    /// Extents may load additional source pages. Dictionaries are decoded without further
    /// dictionary references. Unknown entries and unsupported filters cannot be resolved.
    pub fn resolve(&mut self, reference: BlobRef) -> Result<Vec<u8>> {
        self.resolve_inner(reference, false)
            .map_err(|error| error.context(format!("blob {}:{}", reference.pool, reference.index)))
    }

    /// Validates all table pages, stored-range disjointness, and extent source boundaries.
    ///
    /// Does not authenticate the containing section or decompress ordinary blob contents.
    pub fn validate_pool(&mut self, id: u64) -> Result<()> {
        self.open_pool(id)?;
        for index in 0..self.pools[&id].pages.len() {
            self.load_page(id, index)?;
        }
        let count = self.pools[&id].count;
        for index in 0..count {
            if let Entry::Extents(extents) = self.entry(BlobRef { pool: id, index })? {
                for extent in extents {
                    self.extent_encoding(id, extent)?;
                }
            }
        }
        Ok(())
    }

    /// Loads and validates the small page directory of a previously unused pool.
    fn open_pool(&mut self, id: u64) -> Result<()> {
        if self.pools.contains_key(&id) {
            return Ok(());
        }
        let info = self.reader.section(id)?;
        if info.kind() != container::BLOB_POOL {
            return Err(invalid("blob reference does not identify a BlobPool"));
        }
        let size = info.length() - 8;
        let value = info
            .type_info()?
            .ok_or_else(|| invalid("missing blob pool page directory"))?;
        let count = value.required(0)?.as_u64()?;
        let shift = value.required(1)?.as_u64()?;
        if !(8..=12).contains(&shift) {
            return Err(invalid("invalid blob table page shift"));
        }
        let values = value.required(2)?.as_array()?;
        let expected = if count == 0 {
            0
        } else {
            1 + ((count - 1) >> shift)
        };
        if values.len() as u64 != expected {
            return Err(invalid("incorrect blob table page count"));
        }
        self.reader.limits().elements(expected)?;
        let mut pages = Vec::new();
        let mut ranges = BTreeMap::new();
        for value in values {
            let fields = value.as_array()?;
            if !(2..=3).contains(&fields.len()) {
                return Err(invalid("invalid blob page descriptor"));
            }
            let offset = fields[0].as_u64()?;
            let encoding = Encoding::from_value(&fields[1], self.reader.limits())?;
            if encoding.has_dictionary()? {
                return Err(invalid(
                    "blob table pages must not use external dictionaries",
                ));
            }
            let checksum = fields
                .get(2)
                .map(|value| Checksum::decode(value.as_byte_string()?))
                .transpose()?;
            register_ranges(&mut ranges, vec![(offset, encoding.stored_size)], size)?;
            pages.push(Page {
                offset,
                encoding,
                checksum,
            });
        }
        self.pools.insert(
            id,
            Pool {
                count,
                shift: shift as u8,
                size,
                pages,
                cache: BTreeMap::new(),
                ranges,
            },
        );
        Ok(())
    }

    /// Loads, decodes, and checks one page before publishing it to the cache.
    fn load_page(&mut self, id: u64, index: usize) -> Result<()> {
        let pool = &self.pools[&id];
        if pool.cache.contains_key(&index) {
            return Ok(());
        }
        let page = pool.pages[index].clone();
        let count = (pool.count - ((index as u64) << pool.shift)).min(1 << pool.shift);
        let stored =
            self.reader
                .read_section_range(id, 8 + page.offset, page.encoding.stored_size)?;
        let bytes = self.decode(stored, &page.encoding, true)?;
        if let Some(checksum) = page.checksum {
            checksum.verify(&bytes[..])?;
        }
        let mut decoder = Decoder::new(&bytes, self.reader.limits())?;
        let mut entries = Vec::new();
        let mut ranges = Vec::new();
        for _ in 0..count {
            let kind = decoder.u8()?;
            let mut payload = Decoder::new(decoder.sized()?, self.reader.limits())?;
            let entry = match kind {
                0 => {
                    let offset = payload.vuint()?;
                    let encoding = Encoding::read(&mut payload)?;
                    ranges.push((offset, encoding.stored_size));
                    Entry::Stored { offset, encoding }
                }
                1 => {
                    let count = payload.count()?;
                    if count == 0 {
                        return Err(invalid("empty extent list"));
                    }
                    let mut extents = Vec::new();
                    for _ in 0..count {
                        let extent = Extent {
                            stored_blob_index: payload.vuint()?,
                            decoded_offset: payload.vuint()?,
                            decoded_length: payload.vuint()?,
                        };
                        if extent.decoded_length == 0 {
                            return Err(invalid("zero-length blob extent"));
                        }
                        extents.push(extent);
                    }
                    Entry::Extents(extents)
                }
                _ => {
                    payload.take(payload.remaining())?;
                    Entry::Unknown(kind)
                }
            };
            payload.finish()?;
            entries.push(entry);
        }
        decoder.finish()?;
        let pool = self.pools.get_mut(&id).expect("opened pool");
        register_ranges(&mut pool.ranges, ranges, pool.size)?;
        pool.cache.insert(index, entries);
        Ok(())
    }

    /// Validates an extent against a Stored descriptor without decoding its bytes.
    fn extent_encoding(&mut self, pool: u64, extent: Extent) -> Result<Encoding> {
        let entry = self.entry(BlobRef {
            pool,
            index: extent.stored_blob_index,
        })?;
        let Entry::Stored { encoding, .. } = entry else {
            return Err(invalid("extent source must be a Stored entry"));
        };
        if extent.decoded_length == 0
            || extent
                .decoded_offset
                .checked_add(extent.decoded_length)
                .is_none_or(|end| end > encoding.decoded_size())
        {
            return Err(invalid("extent exceeds decoded source"));
        }
        Ok(encoding)
    }

    /// Resolves a stored blob or concatenation, optionally forbidding dictionary references.
    fn resolve_inner(&mut self, reference: BlobRef, dictionary: bool) -> Result<Vec<u8>> {
        match self.entry(reference)? {
            Entry::Stored { offset, encoding } => {
                self.reader.limits().bytes(encoding.decoded_size())?;
                if dictionary && encoding.has_dictionary()? {
                    return Err(invalid(
                        "dictionary decoding must not reference another dictionary",
                    ));
                }
                let stored = self.reader.read_section_range(
                    reference.pool,
                    8 + offset,
                    encoding.stored_size,
                )?;
                self.decode(stored, &encoding, dictionary)
            }
            Entry::Extents(extents) => {
                let mut total = 0u64;
                for extent in &extents {
                    self.extent_encoding(reference.pool, *extent)?;
                    total = total
                        .checked_add(extent.decoded_length)
                        .ok_or_else(|| invalid("extent length sum overflow"))?;
                }
                let total = self.reader.limits().bytes(total)?;
                let mut output = Vec::with_capacity(total);
                for extent in extents {
                    let bytes = self.resolve_inner(
                        BlobRef {
                            pool: reference.pool,
                            index: extent.stored_blob_index,
                        },
                        dictionary,
                    )?;
                    let start = extent.decoded_offset as usize;
                    let end = start + extent.decoded_length as usize;
                    output.extend_from_slice(&bytes[start..end]);
                }
                Ok(output)
            }
            Entry::Unknown(kind) => Err(Error::new(
                ErrorKind::Unsupported,
                format!("blob entry type {kind}"),
            )),
        }
    }

    /// Reverses filters while checking every intermediate size and dictionary restriction.
    fn decode(
        &mut self,
        mut bytes: Vec<u8>,
        encoding: &Encoding,
        no_dictionary: bool,
    ) -> Result<Vec<u8>> {
        if bytes.len() as u64 != encoding.stored_size {
            return Err(invalid("encoded blob length mismatch"));
        }
        for filter in encoding.filters.iter().rev() {
            let expected = self.reader.limits().bytes(filter.input_size)?;
            if filter.method != 1 {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "unsupported blob filter",
                ));
            }
            let dictionary = match filter.properties.get(0)? {
                Some(value) => {
                    if no_dictionary {
                        return Err(invalid("external dictionary is not permitted here"));
                    }
                    self.resolve_inner(BlobRef::from_value(&value)?, true)?
                }
                None => Vec::new(),
            };
            bytes = decode_zstd(&bytes, &dictionary, expected, self.reader.limits())?;
        }
        Ok(bytes)
    }
}

/// Validates new ranges against one another and previously loaded ranges, then commits them.
fn register_ranges(
    existing: &mut BTreeMap<u64, u64>,
    mut ranges: Vec<(u64, u64)>,
    size: u64,
) -> Result<()> {
    ranges.sort_unstable();
    let mut previous_end = 0;
    let mut nonempty = Vec::new();
    for (offset, length) in ranges {
        let end = offset
            .checked_add(length)
            .filter(|&end| end <= size)
            .ok_or_else(|| invalid("blob range exceeds pool"))?;
        if length == 0 {
            continue;
        }
        if offset < previous_end
            || existing
                .range(..=offset)
                .next_back()
                .is_some_and(|(_, &end)| end > offset)
            || existing
                .range(offset..)
                .next()
                .is_some_and(|(&start, _)| start < end)
        {
            return Err(invalid("overlapping blob or table-page ranges"));
        }
        previous_end = end;
        nonempty.push((offset, end));
    }
    existing.extend(nonempty);
    Ok(())
}

/// Decodes complete Zstandard frames and skips skippable frames, enforcing exact output size.
pub fn decode_zstd(
    bytes: &[u8],
    dictionary: &[u8],
    expected: usize,
    limits: Limits,
) -> Result<Vec<u8>> {
    limits.bytes(bytes.len() as u64)?;
    limits.bytes(expected as u64)?;
    let mut output = Vec::new();
    let mut position = 0usize;
    let mut frames = 0;
    while position < bytes.len() {
        let remaining = &bytes[position..];
        let magic = remaining
            .get(..4)
            .ok_or_else(|| invalid("truncated Zstandard magic"))?;
        let magic = u32::from_le_bytes(magic.try_into().expect("fixed length"));
        if (0x184d2a50..=0x184d2a5f).contains(&magic) {
            let length = remaining
                .get(4..8)
                .ok_or_else(|| invalid("truncated skippable frame"))?;
            let length = u32::from_le_bytes(length.try_into().expect("fixed length")) as usize;
            position = position
                .checked_add(8)
                .and_then(|start| start.checked_add(length))
                .filter(|&end| end <= bytes.len())
                .ok_or_else(|| invalid("skippable frame exceeds input"))?;
            continue;
        }
        if magic != 0xfd2fb528 {
            return Err(invalid("invalid Zstandard frame magic"));
        }
        let size = zstd::zstd_safe::find_frame_compressed_size(remaining).map_err(|code| {
            invalid(format!(
                "invalid Zstandard frame: {}",
                zstd::zstd_safe::get_error_name(code)
            ))
        })?;
        let frame = remaining
            .get(..size)
            .ok_or_else(|| invalid("Zstandard frame exceeds input"))?;
        if let Some(id) = zstd::zstd_safe::get_dict_id_from_frame(frame)
            && Some(id) != zstd::zstd_safe::get_dict_id_from_dict(dictionary)
        {
            return Err(invalid("Zstandard dictionary ID mismatch"));
        }
        let mut decoder = zstd::stream::read::Decoder::with_dictionary(frame, dictionary)?;
        let window_log = (64 - limits.max_bytes.max(1).leading_zeros()).clamp(10, 31);
        decoder.window_log_max(window_log)?;
        decoder
            .take((expected - output.len()) as u64 + 1)
            .read_to_end(&mut output)?;
        if output.len() > expected {
            return Err(invalid("Zstandard output exceeds declared size"));
        }
        position += size;
        frames += 1;
    }
    if frames == 0 || output.len() != expected {
        return Err(invalid("Zstandard decoded size or frame count mismatch"));
    }
    Ok(output)
}

/// The encoded pool section and the type information to place in file metadata.
pub struct BuiltPool {
    /// Complete section bytes, including the BlobPool magic.
    pub bytes: Vec<u8>,
    /// CBOR blob count and page directory.
    pub type_info: Value,
}

/// A pool writer collecting stored bytes and logical entries before writing table pages.
///
/// Blob indices remain stable as entries are added. Stored data precedes the table pages.
/// Callers choose sharing by reusing references or adding Extents entries.
#[derive(Default)]
pub struct PoolBuilder {
    /// Encoded stored bytes after the pool magic.
    bytes: Vec<u8>,
    /// Logical table entries in index order.
    entries: Vec<Entry>,
}

impl PoolBuilder {
    /// Creates an empty pool builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of entries and the index assigned to the next added blob.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the pool contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Stores one blob, using Zstandard only when its bytes and descriptor are smaller.
    pub fn push(&mut self, bytes: &[u8], compression_level: i32) -> Result<u64> {
        let (stored, encoding) = encode_bytes(bytes, compression_level)?;
        let index = self.entries.len() as u64;
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(&stored);
        self.entries.push(Entry::Stored { offset, encoding });
        Ok(index)
    }

    /// Adds a nonempty concatenation of decoded ranges from existing Stored entries.
    pub fn push_extents(&mut self, extents: Vec<Extent>) -> Result<u64> {
        if extents.is_empty() {
            return Err(invalid("empty extent list"));
        }
        for extent in &extents {
            let source = usize::try_from(extent.stored_blob_index)
                .ok()
                .and_then(|index| self.entries.get(index));
            let Some(Entry::Stored { encoding, .. }) = source else {
                return Err(invalid("extent source must be an existing Stored entry"));
            };
            if extent.decoded_length == 0
                || extent
                    .decoded_offset
                    .checked_add(extent.decoded_length)
                    .is_none_or(|end| end > encoding.decoded_size())
            {
                return Err(invalid("extent exceeds source"));
            }
        }
        let index = self.entries.len() as u64;
        self.entries.push(Entry::Extents(extents));
        Ok(index)
    }

    /// Encodes table pages using the selected shift (8 through 12) and compression level.
    pub fn finish(mut self, page_entry_shift: u8, compression_level: i32) -> Result<BuiltPool> {
        if !(8..=12).contains(&page_entry_shift) {
            return Err(invalid("invalid page shift"));
        }
        let mut pages = Vec::new();
        for entries in self.entries.chunks(1 << page_entry_shift) {
            let mut page = Vec::new();
            for entry in entries {
                write_entry(entry, &mut page)?;
            }
            let checksum = Checksum::compute(crate::checksum::Algorithm::Sha256, &page[..])?;
            let (stored, encoding) = encode_bytes(&page, compression_level)?;
            pages.push(Value::array([
                Value::uint(self.bytes.len() as u64),
                encoding.to_value()?,
                Value::bytes(&checksum.encode()),
            ]));
            self.bytes.extend_from_slice(&stored);
        }
        let type_info = Value::map([
            (Value::uint(0), Value::uint(self.entries.len() as u64)),
            (Value::uint(1), Value::uint(u64::from(page_entry_shift))),
            (Value::uint(2), Value::array(pages)),
        ])?;
        let mut bytes = b"BLOBPOOL".to_vec();
        bytes.extend(self.bytes);
        Ok(BuiltPool { bytes, type_info })
    }
}

/// Encodes a value with Zstandard if it reduces the complete encoded representation.
fn encode_bytes(bytes: &[u8], level: i32) -> Result<(Vec<u8>, Encoding)> {
    let stored = zstd::stream::encode_all(bytes, level)?;
    let compressed = Encoding {
        stored_size: stored.len() as u64,
        filters: vec![Filter {
            input_size: bytes.len() as u64,
            method: 1,
            properties: Value::empty_map(),
        }],
    };
    let plain = Encoding {
        stored_size: bytes.len() as u64,
        filters: Vec::new(),
    };
    let mut compressed_descriptor = Vec::new();
    let mut plain_descriptor = Vec::new();
    compressed.write(&mut compressed_descriptor)?;
    plain.write(&mut plain_descriptor)?;
    if stored.len() + compressed_descriptor.len() < bytes.len() + plain_descriptor.len() {
        Ok((stored, compressed))
    } else {
        Ok((bytes.to_vec(), plain))
    }
}

/// Appends one known table entry with an exact tagged-payload boundary.
fn write_entry(entry: &Entry, output: &mut Vec<u8>) -> Result<()> {
    let mut payload = Vec::new();
    let kind = match entry {
        Entry::Stored { offset, encoding } => {
            binary::write_vuint(&mut payload, *offset)?;
            encoding.write(&mut payload)?;
            0
        }
        Entry::Extents(extents) => {
            binary::write_vuint(&mut payload, extents.len() as u64)?;
            for extent in extents {
                binary::write_vuint(&mut payload, extent.stored_blob_index)?;
                binary::write_vuint(&mut payload, extent.decoded_offset)?;
                binary::write_vuint(&mut payload, extent.decoded_length)?;
            }
            1
        }
        Entry::Unknown(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "cannot write an unknown entry without its payload",
            ));
        }
    };
    output.push(kind);
    binary::write_sized(output, &payload)
}

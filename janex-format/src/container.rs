// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Sectioned container framing and verification of exact stored bytes.
//!
//! Opening a container validates its framing and metadata but does not authenticate
//! its publisher. [`Reader::verify_checksums`] checks recorded content digests;
//! signature verification is a separate operation over [`Reader::verification_input`].

use crate::{
    Error, ErrorKind, Result,
    binary::{self, Decoder, Limits},
    cbor::{self, Value},
    checksum::{Algorithm, Checksum},
    error::invalid,
};
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom, Write},
};

/// The eight-byte Janex container magic.
pub const MAGIC: &[u8; 8] = b"JANEX\0\0\0";
/// The eight-byte metadata magic.
pub const METADATA_MAGIC: &[u8; 8] = b"METADATA";
/// The eight-byte footer marker.
pub const END_MARK: &[u8; 8] = b"JANEXEND";
/// The BlobPool section type and little-endian magic number.
pub const BLOB_POOL: u64 = u64::from_le_bytes(*b"BLOBPOOL");
/// The Application section type and little-endian magic number.
pub const APPLICATION: u64 = u64::from_le_bytes(*b"JANEXAPP");
/// The padding section type; its data has no required magic.
pub const PADDING: u64 = u64::from_le_bytes(*b"PADDING\0");

/// A section descriptor retaining unknown metadata fields.
#[derive(Clone, Debug)]
pub struct SectionInfo {
    /// The complete section-info map.
    value: Value,
    /// The section's opaque file-local identifier.
    id: u64,
    /// The section type identifier.
    kind: u64,
    /// The complete encoded section size, including its magic if required.
    length: u64,
    /// The optional recorded checksum.
    checksum: Option<Checksum>,
}

impl SectionInfo {
    /// Validates the common section-info schema without discarding extensions.
    pub fn from_value(value: Value) -> Result<Self> {
        integer_keys(&value)?;
        let kind = value.required(0)?.as_u64()?;
        let id = value.required(1)?.as_u64()?;
        let length = value.required(2)?.as_u64()?;
        let checksum = value
            .get(3)?
            .map(|value| Checksum::decode(value.as_byte_string()?))
            .transpose()?;
        if let Some(info) = value.get(4)? {
            integer_keys(&info)?;
        }
        if (kind == BLOB_POOL || kind == APPLICATION) && length < 8 {
            return Err(invalid("section is shorter than its magic"));
        }
        Ok(Self {
            value,
            id,
            kind,
            length,
            checksum,
        })
    }

    /// Returns the complete map, including unknown fields.
    pub fn value(&self) -> &Value {
        &self.value
    }
    /// Returns the file-local section identifier.
    pub fn id(&self) -> u64 {
        self.id
    }
    /// Returns the section type.
    pub fn kind(&self) -> u64 {
        self.kind
    }
    /// Returns the encoded byte length, including any magic.
    pub fn length(&self) -> u64 {
        self.length
    }
    /// Returns the recorded checksum, if any.
    pub fn checksum(&self) -> Option<&Checksum> {
        self.checksum.as_ref()
    }
    /// Returns a copy of the type-specific map, if present.
    pub fn type_info(&self) -> Result<Option<Value>> {
        self.value.get(4)
    }
}

/// A metadata verification payload, not a claim that verification has succeeded.
#[derive(Clone, Debug)]
pub enum Verification {
    /// No verification payload.
    None,
    /// A checksum of the exact metadata verification input.
    Checksum(Checksum),
    /// A binary detached OpenPGP signature, to be validated under the Janex profile.
    OpenPgp(Vec<u8>),
    /// A DER CMS ContentInfo value, to be validated under the Janex profile.
    Cms(Vec<u8>),
}

impl Verification {
    /// Parses the framing of a verification payload without establishing signature trust.
    fn decode(kind: u8, bytes: &[u8]) -> Result<Self> {
        match kind {
            0 if bytes.is_empty() => Ok(Self::None),
            0 => Err(invalid("None verification payload must be empty")),
            1 => Ok(Self::Checksum(Checksum::decode(bytes)?)),
            2 if !bytes.is_empty() => Ok(Self::OpenPgp(bytes.to_vec())),
            3 if !bytes.is_empty() => Ok(Self::Cms(bytes.to_vec())),
            2 | 3 => Err(invalid("signature payload must not be empty")),
            _ => Err(invalid(format!("unknown verification type {kind}"))),
        }
    }
}

/// The outcome of validating all recorded section and external-region checksums.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegrityReport {
    /// Number of verified section and external-region checksums, excluding metadata.
    pub checksums_verified: usize,
    /// Whether secure checksums cover every section and both external regions are constrained.
    ///
    /// This does not indicate signature validity or publisher trust.
    pub complete_secure_coverage: bool,
}

/// A seekable Janex container reader owning its underlying stream.
///
/// The caller must ensure input bytes remain unchanged for the reader's lifetime.
/// Methods reposition the stream and may leave it partially consumed after failure.
/// Returned metadata is owned by this reader and does not borrow the stream.
pub struct Reader<R> {
    /// The owned input stream.
    source: R,
    /// Limits for buffered values and collections.
    limits: Limits,
    /// Physical file size at open time.
    physical_size: u64,
    /// Physical start of the Janex magic.
    start: u64,
    /// Physical end of the Janex footer.
    end: u64,
    /// The complete file metadata map.
    metadata: Value,
    /// Sections in physical order, paired with physical offsets.
    sections: Vec<(SectionInfo, u64)>,
    /// The verification mechanism and payload.
    verification: Verification,
    /// Exact bytes from metadata magic through the verification type.
    verification_input: Vec<u8>,
}

impl<R: Read + Seek> Reader<R> {
    /// Discovers a standalone footer or a footer immediately before a JAR tail.
    ///
    /// ZIP offsets must be relative to the JAR start. Ordinary ZIP and variable-length
    /// ZIP64 end records are supported. Every candidate must pass container framing
    /// validation; missing or ambiguous boundaries are errors. ZIP64 discovery may
    /// scan the prefix before its locator to establish an unambiguous record boundary.
    pub fn open_auto(mut source: R, limits: Limits) -> Result<Self> {
        let size = source.seek(SeekFrom::End(0))?;
        let mut ends = crate::wrapper::candidate_ends(&mut source, size, limits)?;
        ends.push(size);
        ends.sort_unstable();
        ends.dedup();
        let mut matched = None;
        for end in ends {
            match Reader::open(&mut source, size - end, limits) {
                Ok(_) => {
                    if matched.replace(end).is_some() {
                        return Err(invalid("ambiguous Janex boundary"));
                    }
                }
                Err(error) if error.kind() == ErrorKind::Invalid => {}
                Err(error) => return Err(error),
            }
        }
        let end = matched.ok_or_else(|| invalid("no valid Janex boundary"))?;
        Self::open(source, size - end, limits)
    }

    /// Opens a container using an explicit external-tail length; standalone files use zero.
    ///
    /// Validates version, CBOR metadata, lengths, section identities, known section magic,
    /// and recorded external-region sizes. Checksums and signatures are not verified here.
    /// Unknown section bodies are left uninterpreted. Reads only framing and metadata.
    pub fn open(mut source: R, external_tail_length: u64, limits: Limits) -> Result<Self> {
        let physical_size = source.seek(SeekFrom::End(0))?;
        let end = physical_size
            .checked_sub(external_tail_length)
            .ok_or_else(|| invalid("external tail exceeds file size"))?;
        let footer_start = end
            .checked_sub(24)
            .ok_or_else(|| invalid("missing Janex footer"))?;
        let footer = read_at(&mut source, footer_start, 24, limits)?;
        if &footer[..8] != END_MARK {
            return Err(invalid("incorrect Janex end marker").at(footer_start));
        }
        let metadata_length = u64::from_le_bytes(footer[8..16].try_into().expect("fixed length"));
        let file_length = u64::from_le_bytes(footer[16..24].try_into().expect("fixed length"));
        let metadata_start = end
            .checked_sub(metadata_length)
            .ok_or_else(|| invalid("metadata length exceeds file"))?;
        let start = end
            .checked_sub(file_length)
            .ok_or_else(|| invalid("Janex length exceeds file"))?;
        if metadata_start
            < start
                .checked_add(8)
                .ok_or_else(|| invalid("Janex start overflow"))?
            || metadata_length < 24
        {
            return Err(invalid("metadata overlaps container magic or footer"));
        }
        if read_at(&mut source, start, 8, limits)? != MAGIC {
            return Err(invalid("incorrect Janex magic").at(start));
        }
        let bytes = read_at(&mut source, metadata_start, metadata_length - 24, limits)?;
        let mut decoder = Decoder::new(&bytes, limits)?;
        if decoder.take(8)? != METADATA_MAGIC {
            return Err(invalid("incorrect metadata magic").at(metadata_start));
        }
        let major = decoder.u32()?;
        let minor = decoder.u32()?;
        if (major, minor) != (0, 1) {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("Janex version {major}.{minor}"),
            )
            .at(metadata_start + 8));
        }
        let metadata = cbor::read_sized(&mut decoder)?;
        validate_metadata(&metadata)?;
        let kind = decoder.u8()?;
        let verification_input = bytes[..decoder.position()].to_vec();
        let verification = Verification::decode(kind, decoder.sized()?)?;
        decoder.finish()?;

        let table = metadata.required(0)?.as_array()?;
        limits.elements(table.len() as u64)?;
        let mut sections = Vec::new();
        let mut ids = BTreeSet::new();
        let mut offset = start + 8;
        for value in table {
            let info = SectionInfo::from_value(value)?;
            if !ids.insert(info.id) {
                return Err(invalid("duplicate section ID"));
            }
            let next = offset
                .checked_add(info.length)
                .filter(|&next| next <= metadata_start)
                .ok_or_else(|| invalid("section exceeds container data").at(offset))?;
            if info.kind == BLOB_POOL || info.kind == APPLICATION {
                let magic = read_at(&mut source, offset, 8, limits)?;
                if magic != info.kind.to_le_bytes() {
                    return Err(invalid("section magic does not match its type").at(offset));
                }
            }
            sections.push((info, offset));
            offset = next;
        }
        if offset != metadata_start {
            return Err(invalid("section lengths do not fill container data").at(offset));
        }
        for (key, length) in [(1, start), (2, external_tail_length)] {
            if let Some(region) = metadata.get(key)?
                && region.required(0)?.as_u64()? != length
            {
                return Err(invalid("external-region size mismatch"));
            }
        }
        Ok(Self {
            source,
            limits,
            physical_size,
            start,
            end,
            metadata,
            sections,
            verification,
            verification_input,
        })
    }

    /// Returns the complete file-metadata map, including unknown fields.
    pub fn metadata(&self) -> &Value {
        &self.metadata
    }
    /// Iterates over section descriptors in physical order.
    pub fn sections(&self) -> impl ExactSizeIterator<Item = &SectionInfo> {
        self.sections.iter().map(|(info, _)| info)
    }
    /// Returns a section by its opaque ID, reporting a missing reference as invalid input.
    pub fn section(&self, id: u64) -> Result<&SectionInfo> {
        self.sections
            .iter()
            .find(|(info, _)| info.id == id)
            .map(|(info, _)| info)
            .ok_or_else(|| invalid(format!("missing section {id}")))
    }
    /// Returns a section's absolute file range, including its magic when present.
    ///
    /// This reads no section bytes and does not establish checksum or signature validity.
    pub fn section_range(&self, id: u64) -> Result<std::ops::Range<u64>> {
        let (info, offset) = self
            .sections
            .iter()
            .find(|(info, _)| info.id == id)
            .ok_or_else(|| invalid(format!("missing section {id}")))?;
        Ok(*offset..*offset + info.length)
    }
    /// Returns the verification payload without implying it has been authenticated.
    pub fn verification(&self) -> &Verification {
        &self.verification
    }
    /// Returns the exact input to checksum or signature verification.
    pub fn verification_input(&self) -> &[u8] {
        &self.verification_input
    }
    /// Returns the limits used by this reader.
    pub fn limits(&self) -> Limits {
        self.limits
    }
    /// Returns the physical byte range occupied by Janex, excluding external regions.
    pub fn range(&self) -> std::ops::Range<u64> {
        self.start..self.end
    }

    /// Reads an entire section into memory, subject to the byte limit.
    ///
    /// This operation alone does not verify its checksum.
    pub fn read_section(&mut self, id: u64) -> Result<Vec<u8>> {
        let length = self.section(id)?.length;
        self.read_section_range(id, 0, length)
    }

    /// Reads a section-relative byte range, rejecting arithmetic overflow and out-of-range access.
    ///
    /// The returned bytes are not independently authenticated by this operation.
    pub fn read_section_range(&mut self, id: u64, offset: u64, length: u64) -> Result<Vec<u8>> {
        let (info, base) = self
            .sections
            .iter()
            .find(|(info, _)| info.id == id)
            .ok_or_else(|| invalid(format!("missing section {id}")))?;
        if offset
            .checked_add(length)
            .is_none_or(|end| end > info.length)
        {
            return Err(invalid("range exceeds section"));
        }
        read_at(&mut self.source, base + offset, length, self.limits)
    }

    /// Verifies the metadata checksum, when present, and every recorded content checksum.
    ///
    /// Streams section and external-region data without applying the buffered byte limit.
    /// Signed metadata still requires a separate signature and trust check. No verification
    /// result is cached; a caller with an immutable snapshot may retain the returned report.
    pub fn verify_checksums(&mut self) -> Result<IntegrityReport> {
        if let Verification::Checksum(checksum) = &self.verification {
            checksum.verify(&self.verification_input[..])?;
        }
        let mut report = IntegrityReport {
            checksums_verified: 0,
            complete_secure_coverage: true,
        };
        for (info, offset) in &self.sections {
            match &info.checksum {
                Some(checksum) => {
                    verify_range(&mut self.source, *offset, info.length, checksum)
                        .map_err(|error| error.context(format!("section {}", info.id)))?;
                    report.checksums_verified += 1;
                    report.complete_secure_coverage &= checksum.algorithm().is_secure();
                }
                None => report.complete_secure_coverage = false,
            }
        }
        for (key, offset, length) in [
            (1, 0, self.start),
            (2, self.end, self.physical_size - self.end),
        ] {
            match self.metadata.get(key)? {
                Some(region) => match region.get(1)? {
                    Some(value) => {
                        let checksum = Checksum::decode(value.as_byte_string()?)?;
                        verify_range(&mut self.source, offset, length, &checksum)?;
                        report.checksums_verified += 1;
                        report.complete_secure_coverage &=
                            length == 0 || checksum.algorithm().is_secure();
                    }
                    None => report.complete_secure_coverage &= length == 0,
                },
                None => report.complete_secure_coverage = false,
            }
        }
        Ok(report)
    }

    /// Returns the owned input stream at its current, unspecified position.
    pub fn into_inner(self) -> R {
        self.source
    }

    /// Borrows the underlying source without changing its cursor or verifying its contents.
    pub fn get_ref(&self) -> &R {
        &self.source
    }
}

/// A sequential writer for one Janex container, beginning at the output's current position.
///
/// Failed writes may leave partial output. Discard that output instead of retrying an
/// operation. The caller owns any external header or tail and supplies their metadata.
pub struct Writer<W> {
    /// The owned output stream.
    output: W,
    /// Encoded section descriptors in physical order.
    sections: Vec<Value>,
    /// IDs already assigned to written sections.
    ids: BTreeSet<u64>,
    /// Bytes written since the Janex magic, including that magic.
    length: u64,
}

impl<W: Write> Writer<W> {
    /// Writes the container magic at the current output position.
    pub fn new(mut output: W) -> Result<Self> {
        output.write_all(MAGIC)?;
        Ok(Self {
            output,
            sections: Vec::new(),
            ids: BTreeSet::new(),
            length: 8,
        })
    }

    /// Writes one complete section and records its SHA-256 checksum.
    ///
    /// `bytes` includes any section magic. IDs must be unique. `type_info` is a
    /// type-specific integer-keyed map; this operation checks its framing only.
    pub fn write_section(
        &mut self,
        id: u64,
        kind: u64,
        bytes: &[u8],
        type_info: Option<Value>,
    ) -> Result<()> {
        if self.ids.contains(&id) {
            return Err(invalid("duplicate section ID"));
        }
        if (kind == BLOB_POOL || kind == APPLICATION) && !bytes.starts_with(&kind.to_le_bytes()) {
            return Err(invalid("section magic does not match its type"));
        }
        let checksum = Checksum::compute(Algorithm::Sha256, bytes)?;
        let mut fields = vec![
            (Value::uint(0), Value::uint(kind)),
            (Value::uint(1), Value::uint(id)),
            (Value::uint(2), Value::uint(bytes.len() as u64)),
            (Value::uint(3), Value::bytes(&checksum.encode())),
        ];
        if let Some(type_info) = type_info {
            integer_keys(&type_info)?;
            fields.push((Value::uint(4), type_info));
        }
        let value = Value::map(fields)?;
        let next = self
            .length
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| invalid("container length overflow"))?;
        self.output.write_all(bytes)?;
        self.length = next;
        self.ids.insert(id);
        self.sections.push(value);
        Ok(())
    }

    /// Finishes the file using a SHA-256 metadata checksum and returns the output.
    ///
    /// `metadata` must be a file-metadata map without key zero; the section table is
    /// supplied by this writer. The operation does not flush or close the output.
    pub fn finish(self, metadata: Value) -> Result<W> {
        self.finish_with(metadata, 1, |input| {
            Ok(Checksum::compute(Algorithm::Sha256, input)?.encode())
        })
    }

    /// Finishes using a payload produced over the exact verification input.
    ///
    /// The callback receives metadata bytes ending with `verification_type` and must
    /// return the complete payload for that type. This method validates framing only;
    /// the caller is responsible for signature profile and cryptographic correctness.
    /// Types are 0 (None), 1 (Checksum), 2 (OpenPGP), and 3 (CMS).
    /// Callback errors are propagated unchanged; format and I/O failures convert through `Error`.
    /// No footer is written if payload generation fails. The output is not flushed.
    pub fn finish_with<E: From<Error>>(
        mut self,
        metadata: Value,
        verification_type: u8,
        payload: impl FnOnce(&[u8]) -> std::result::Result<Vec<u8>, E>,
    ) -> std::result::Result<W, E> {
        let mut fields = metadata.as_map()?;
        if metadata.get(0)?.is_some() {
            return Err(invalid("writer supplies the section table").into());
        }
        fields.push((Value::uint(0), Value::array(self.sections)));
        let metadata = Value::map(fields)?;
        validate_metadata(&metadata)?;
        let mut bytes = METADATA_MAGIC.to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        cbor::write_sized(&mut bytes, &metadata)?;
        bytes.push(verification_type);
        let payload = payload(&bytes)?;
        let verification = Verification::decode(verification_type, &payload)?;
        if let Verification::Checksum(checksum) = verification {
            checksum.verify(&bytes[..])?;
        }
        binary::write_sized(&mut bytes, &payload)?;
        let metadata_length = (bytes.len() as u64)
            .checked_add(24)
            .ok_or_else(|| invalid("metadata length overflow"))?;
        let file_length = self
            .length
            .checked_add(metadata_length)
            .ok_or_else(|| invalid("container length overflow"))?;
        self.output.write_all(&bytes).map_err(Error::from)?;
        self.output.write_all(END_MARK).map_err(Error::from)?;
        self.output
            .write_all(&metadata_length.to_le_bytes())
            .map_err(Error::from)?;
        self.output
            .write_all(&file_length.to_le_bytes())
            .map_err(Error::from)?;
        Ok(self.output)
    }
}

/// Checks that a value is a map whose keys are all unsigned integers.
pub(crate) fn integer_keys(value: &Value) -> Result<()> {
    for (key, _) in value.as_map()? {
        key.as_u64()?;
    }
    Ok(())
}

/// Validates file metadata independently of its physical location.
fn validate_metadata(value: &Value) -> Result<()> {
    for (key, _) in value.as_map()? {
        if key.as_u64().is_err() && key.as_text()?.is_empty() {
            return Err(invalid("empty metadata attribute key"));
        }
    }
    value.required(0)?.as_array()?;
    for key in [1, 2] {
        if let Some(region) = value.get(key)? {
            integer_keys(&region)?;
            region.required(0)?.as_u64()?;
            if let Some(checksum) = region.get(1)? {
                Checksum::decode(checksum.as_byte_string()?)?;
            }
        }
    }
    for key in [3, 4] {
        if let Some(text) = value.get(key)?
            && text.as_text()?.is_empty()
        {
            return Err(invalid("empty package name or version"));
        }
    }
    Ok(())
}

/// Reads an exact range while limiting its allocation.
pub(crate) fn read_at(
    source: &mut (impl Read + Seek),
    offset: u64,
    length: u64,
    limits: Limits,
) -> Result<Vec<u8>> {
    let size = limits.bytes(length)?;
    source.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; size];
    source
        .read_exact(&mut bytes)
        .map_err(|error| Error::from(error).at(offset))?;
    Ok(bytes)
}

/// Verifies an exact physical byte range without accepting early EOF.
fn verify_range(
    source: &mut (impl Read + Seek),
    offset: u64,
    length: u64,
    checksum: &Checksum,
) -> Result<()> {
    source.seek(SeekFrom::Start(offset))?;
    let mut range = source.take(length);
    checksum.verify(&mut range)?;
    if range.limit() != 0 {
        return Err(invalid("truncated checksum range").at(offset));
    }
    Ok(())
}

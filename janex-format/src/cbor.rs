// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Deterministic CBOR values retaining their exact encoded bytes.
//!
//! Validation checks encoding, not application schemas. Typed accessors report type
//! mismatches; callers must also check the schema at each use site.

use crate::{
    Error, ErrorKind, Result,
    binary::{self, Limits},
    error::invalid,
};
use std::io::Write;

/// Exactly one core-deterministic CBOR item, stored in its original representation.
///
/// Equality compares encoded bytes. Map constructors sort keys by those bytes and
/// reject duplicates. Cloning copies the encoded value without interpreting extensions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Value {
    /// The complete deterministic encoding.
    bytes: Vec<u8>,
}

impl Value {
    /// Validates and copies one CBOR item, rejecting trailing bytes.
    pub fn from_bytes(bytes: &[u8], limits: Limits) -> Result<Self> {
        limits.bytes(bytes.len() as u64)?;
        let mut cursor = binary::Decoder::new(bytes, limits)?;
        scan(&mut cursor, bytes, 0)?;
        cursor.finish()?;
        Ok(Self {
            bytes: bytes.to_vec(),
        })
    }

    /// Returns the complete deterministic encoding, including its CBOR header.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Encodes an unsigned integer in its shortest form.
    pub fn uint(value: u64) -> Self {
        let mut encoder = minicbor::Encoder::new(Vec::new());
        encoder.u64(value).expect("Vec encoding cannot fail");
        Self {
            bytes: encoder.into_writer(),
        }
    }

    /// Encodes a signed 128-bit integer, using a minimal bignum only outside CBOR's basic range.
    pub fn integer(value: i128) -> Self {
        let negative = value < 0;
        let magnitude = if negative {
            (-1 - value) as u128
        } else {
            value as u128
        };
        let mut encoder = minicbor::Encoder::new(Vec::new());
        if let Ok(magnitude) = u64::try_from(magnitude) {
            encoder.u64(magnitude).expect("Vec encoding cannot fail");
            let mut bytes = encoder.into_writer();
            if negative {
                bytes[0] |= 0x20;
            }
            Self { bytes }
        } else {
            encoder
                .tag(minicbor::data::Tag::new(if negative { 3 } else { 2 }))
                .expect("Vec encoding cannot fail");
            let bytes = magnitude.to_be_bytes();
            let first = bytes
                .iter()
                .position(|byte| *byte != 0)
                .expect("nonzero bignum");
            encoder
                .bytes(&bytes[first..])
                .expect("Vec encoding cannot fail");
            Self {
                bytes: encoder.into_writer(),
            }
        }
    }

    /// Encodes UTF-8 text.
    pub fn text(value: &str) -> Self {
        let mut encoder = minicbor::Encoder::new(Vec::new());
        encoder.str(value).expect("Vec encoding cannot fail");
        Self {
            bytes: encoder.into_writer(),
        }
    }

    /// Encodes a byte string without interpreting its contents.
    pub fn bytes(value: &[u8]) -> Self {
        let mut encoder = minicbor::Encoder::new(Vec::new());
        encoder.bytes(value).expect("Vec encoding cannot fail");
        Self {
            bytes: encoder.into_writer(),
        }
    }

    /// Encodes a boolean.
    pub fn boolean(value: bool) -> Self {
        Self {
            bytes: vec![if value { 0xf5 } else { 0xf4 }],
        }
    }

    /// Encodes the CBOR null value.
    pub fn null() -> Self {
        Self { bytes: vec![0xf6] }
    }

    /// Encodes a definite-length array in element order.
    pub fn array(values: impl IntoIterator<Item = Self>) -> Self {
        let values: Vec<_> = values.into_iter().collect();
        let mut encoder = minicbor::Encoder::new(Vec::new());
        encoder
            .array(values.len() as u64)
            .expect("Vec encoding cannot fail");
        let mut bytes = encoder.into_writer();
        for value in values {
            bytes.extend(value.bytes);
        }
        Self { bytes }
    }

    /// Encodes a map in core-deterministic key order, rejecting duplicate encoded keys.
    pub fn map(entries: impl IntoIterator<Item = (Self, Self)>) -> Result<Self> {
        let mut entries: Vec<_> = entries.into_iter().collect();
        entries.sort_by(|a, b| a.0.bytes.cmp(&b.0.bytes));
        if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(invalid("duplicate CBOR map key"));
        }
        let mut encoder = minicbor::Encoder::new(Vec::new());
        encoder
            .map(entries.len() as u64)
            .expect("Vec encoding cannot fail");
        let mut bytes = encoder.into_writer();
        for (key, value) in entries {
            bytes.extend(key.bytes);
            bytes.extend(value.bytes);
        }
        Ok(Self { bytes })
    }

    /// Returns an empty map.
    pub fn empty_map() -> Self {
        Self { bytes: vec![0xa0] }
    }

    /// Returns whether this is an empty map.
    pub fn is_empty_map(&self) -> bool {
        self.bytes == [0xa0]
    }

    /// Returns whether this is null.
    pub fn is_null(&self) -> bool {
        self.bytes == [0xf6]
    }

    /// Returns an unsigned integer, or a type error.
    pub fn as_u64(&self) -> Result<u64> {
        minicbor::Decoder::new(&self.bytes)
            .u64()
            .map_err(decode_error)
    }

    /// Returns a signed 128-bit integer, rejecting nonminimal or out-of-range bignums.
    ///
    /// Tags 2 and 3 must contain a minimal 9-to-16-byte magnitude. Smaller magnitudes
    /// must use a basic CBOR integer; unrelated tags are type errors.
    pub fn as_i128(&self) -> Result<i128> {
        let mut decoder = minicbor::Decoder::new(&self.bytes);
        if self.bytes[0] >> 5 != 6 {
            return decoder.int().map(i128::from).map_err(decode_error);
        }
        let tag = decoder.tag().map_err(decode_error)?.as_u64();
        if !matches!(tag, 2 | 3) {
            return Err(invalid("expected integer or bignum"));
        }
        let bytes = decoder.bytes().map_err(decode_error)?;
        if !(9..=16).contains(&bytes.len()) || bytes[0] == 0 {
            return Err(invalid("nonminimal or oversized bignum magnitude"));
        }
        let mut magnitude = 0u128;
        for byte in bytes {
            magnitude = (magnitude << 8) | u128::from(*byte);
        }
        let magnitude = i128::try_from(magnitude)
            .map_err(|_| invalid("integer exceeds signed 128-bit range"))?;
        Ok(if tag == 3 { -1 - magnitude } else { magnitude })
    }

    /// Borrows a text string, or reports a type error.
    pub fn as_text(&self) -> Result<&str> {
        minicbor::Decoder::new(&self.bytes)
            .str()
            .map_err(decode_error)
    }

    /// Borrows the contents of a byte string, excluding its header.
    pub fn as_byte_string(&self) -> Result<&[u8]> {
        minicbor::Decoder::new(&self.bytes)
            .bytes()
            .map_err(decode_error)
    }

    /// Returns a boolean, or a type error.
    pub fn as_bool(&self) -> Result<bool> {
        minicbor::Decoder::new(&self.bytes)
            .bool()
            .map_err(decode_error)
    }

    /// Copies the direct elements of an array without recursively decoding them.
    pub fn as_array(&self) -> Result<Vec<Self>> {
        let mut decoder = minicbor::Decoder::new(&self.bytes);
        let count = decoder
            .array()
            .map_err(decode_error)?
            .ok_or_else(|| invalid("indefinite array"))?;
        let mut values = Vec::new();
        for _ in 0..count {
            values.push(self.next_value(&mut decoder)?);
        }
        Ok(values)
    }

    /// Copies the direct entries of a map, preserving deterministic order.
    pub fn as_map(&self) -> Result<Vec<(Self, Self)>> {
        let mut decoder = minicbor::Decoder::new(&self.bytes);
        let count = decoder
            .map()
            .map_err(decode_error)?
            .ok_or_else(|| invalid("indefinite map"))?;
        let mut values = Vec::new();
        for _ in 0..count {
            values.push((
                self.next_value(&mut decoder)?,
                self.next_value(&mut decoder)?,
            ));
        }
        Ok(values)
    }

    /// Looks up an integer key, reporting a type error if this value is not a map.
    pub fn get(&self, key: u64) -> Result<Option<Self>> {
        let mut decoder = minicbor::Decoder::new(&self.bytes);
        let count = decoder
            .map()
            .map_err(decode_error)?
            .ok_or_else(|| invalid("indefinite map"))?;
        for _ in 0..count {
            let start = decoder.position();
            let matches = match decoder.u64() {
                Ok(value) => value == key,
                Err(_) => {
                    decoder.set_position(start);
                    decoder.skip().map_err(decode_error)?;
                    false
                }
            };
            if matches {
                return self.next_value(&mut decoder).map(Some);
            }
            decoder.skip().map_err(decode_error)?;
        }
        Ok(None)
    }

    /// Looks up a text key, reporting a type error if this value is not a map.
    pub fn get_text(&self, key: &str) -> Result<Option<Self>> {
        let mut decoder = minicbor::Decoder::new(&self.bytes);
        let count = decoder
            .map()
            .map_err(decode_error)?
            .ok_or_else(|| invalid("indefinite map"))?;
        for _ in 0..count {
            let start = decoder.position();
            let matches = match decoder.str() {
                Ok(value) => value == key,
                Err(_) => {
                    decoder.set_position(start);
                    decoder.skip().map_err(decode_error)?;
                    false
                }
            };
            if matches {
                return self.next_value(&mut decoder).map(Some);
            }
            decoder.skip().map_err(decode_error)?;
        }
        Ok(None)
    }

    /// Returns a required integer-keyed field or reports its absence.
    pub fn required(&self, key: u64) -> Result<Self> {
        self.get(key)?
            .ok_or_else(|| invalid(format!("missing CBOR field {key}")))
    }

    /// Copies one already-validated child item and advances the CBOR decoder.
    fn next_value(&self, decoder: &mut minicbor::Decoder<'_>) -> Result<Self> {
        let start = decoder.position();
        decoder.skip().map_err(decode_error)?;
        Ok(Self {
            bytes: self.bytes[start..decoder.position()].to_vec(),
        })
    }
}

/// Reads a `Sized<CborValue>`, including Janex's zero-length empty-map representation.
pub fn read_sized(decoder: &mut binary::Decoder<'_>) -> Result<Value> {
    let limits = decoder.limits();
    let bytes = decoder.sized()?;
    if bytes.is_empty() {
        return Ok(Value::empty_map());
    }
    let value = Value::from_bytes(bytes, limits)?;
    if value.is_empty_map() {
        return Err(invalid("empty sized CBOR map must use zero length"));
    }
    Ok(value)
}

/// Writes `Sized<CborValue>`, using zero length for an empty map.
///
/// An I/O failure may leave a prefix in the destination.
pub fn write_sized(writer: impl Write, value: &Value) -> Result<()> {
    binary::write_sized(
        writer,
        if value.is_empty_map() {
            &[]
        } else {
            value.as_bytes()
        },
    )
}

/// Converts a CBOR type or framing error to a format diagnostic.
fn decode_error(error: minicbor::decode::Error) -> Error {
    let position = error.position();
    let error = invalid(error.to_string());
    match position {
        Some(position) => error.at(position as u64),
        None => error,
    }
}

/// Reads a preferred-width CBOR argument after its initial byte.
fn argument(cursor: &mut binary::Decoder<'_>, additional: u8) -> Result<u64> {
    let (length, minimum) = match additional {
        0..=23 => return Ok(u64::from(additional)),
        24 => (1, 24),
        25 => (2, 256),
        26 => (4, 65536),
        27 => (8, 1u64 << 32),
        _ => {
            return Err(
                invalid("reserved or indefinite CBOR argument").at((cursor.position() - 1) as u64)
            );
        }
    };
    let bytes = cursor.take(length)?;
    let value = bytes
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
    if value < minimum {
        return Err(invalid("nonminimal CBOR argument").at((cursor.position() - length - 1) as u64));
    }
    Ok(value)
}

/// Checks one item's exact representation and recursively checks its children.
fn scan(cursor: &mut binary::Decoder<'_>, input: &[u8], depth: usize) -> Result<()> {
    let start = cursor.position();
    if depth > cursor.limits().max_depth {
        return Err(Error::new(ErrorKind::Limit, "CBOR nesting limit exceeded").at(start as u64));
    }
    let first = cursor.u8()?;
    let major = first >> 5;
    let additional = first & 31;
    if major == 7 {
        return scan_simple(cursor, additional, start);
    }
    let value = argument(cursor, additional)?;
    match major {
        0 | 1 => {}
        2 | 3 => {
            let length = cursor.limits().bytes(value)?;
            let bytes = cursor.take(length)?;
            if major == 3 && std::str::from_utf8(bytes).is_err() {
                return Err(invalid("invalid CBOR UTF-8 text").at(start as u64));
            }
        }
        4 => {
            let count = cursor.limits().elements(value)?;
            for _ in 0..count {
                scan(cursor, input, depth + 1)?;
            }
        }
        5 => {
            let count = cursor.limits().elements(value)?;
            let mut previous: Option<&[u8]> = None;
            for _ in 0..count {
                let key_start = cursor.position();
                scan(cursor, input, depth + 1)?;
                let key = &input[key_start..cursor.position()];
                if previous.is_some_and(|previous| previous >= key) {
                    return Err(invalid("unordered or duplicate CBOR map key").at(key_start as u64));
                }
                previous = Some(key);
                scan(cursor, input, depth + 1)?;
            }
        }
        6 => scan(cursor, input, depth + 1)?,
        _ => unreachable!("three-bit major type"),
    }
    Ok(())
}

/// Validates simple values and preferred floating-point widths.
fn scan_simple(cursor: &mut binary::Decoder<'_>, additional: u8, start: usize) -> Result<()> {
    let valid = match additional {
        0..=23 => true,
        24 => cursor.u8()? >= 32,
        25 => {
            let bits = u16::from_be_bytes(cursor.take(2)?.try_into().expect("fixed length"));
            !half::f16::from_bits(bits).is_nan() || bits == 0x7e00
        }
        26 => {
            let value = f32::from_be_bytes(cursor.take(4)?.try_into().expect("fixed length"));
            !value.is_nan() && half::f16::from_f32(value).to_f32().to_bits() != value.to_bits()
        }
        27 => {
            let value = f64::from_be_bytes(cursor.take(8)?.try_into().expect("fixed length"));
            !value.is_nan() && ((value as f32) as f64).to_bits() != value.to_bits()
        }
        _ => false,
    };
    if !valid {
        return Err(invalid("non-deterministic or invalid CBOR simple value").at(start as u64));
    }
    Ok(())
}

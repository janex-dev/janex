// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::error::Error;

/// A byte reader for Janex binary structures.
pub trait DataReader {
    /// Returns the number of unread bytes remaining in the source.
    fn remaining(&self) -> usize;

    /// Reads exactly `N` bytes from the source.
    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], Error>;

    /// Reads `size` bytes from the source into an owned buffer.
    fn read_u8_array(&mut self, size: usize) -> Result<Box<[u8]>, Error>;

    /// Reads `size` little-endian `u16` values into an owned buffer.
    fn read_u16_array_le(&mut self, size: usize) -> Result<Box<[u16]>, Error> {
        Ok(self
            .read_u8_array(size * 2)?
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect())
    }

    /// Reads `size` big-endian `u16` values into an owned buffer.
    fn read_u16_array_be(&mut self, size: usize) -> Result<Box<[u16]>, Error> {
        Ok(self
            .read_u8_array(size * 2)?
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect())
    }

    /// Reads a single byte.
    fn read_u8(&mut self) -> Result<u8, Error> {
        let [b] = self.read_array()?;
        Ok(b)
    }

    /// Reads a little-endian `u16`.
    fn read_u16_le(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.read_array()?))
    }

    /// Reads a big-endian `u16`.
    fn read_u16_be(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.read_array()?))
    }

    /// Reads a little-endian `u32`.
    fn read_u32_le(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.read_array()?))
    }

    /// Reads a big-endian `u32`.
    fn read_u32_be(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.read_array()?))
    }

    /// Reads a little-endian `u64`.
    fn read_u64_le(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.read_array()?))
    }

    /// Reads a big-endian `u64`.
    fn read_u64_be(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.read_array()?))
    }

    /// Reads a little-endian `i64`.
    fn read_i64_le(&mut self) -> Result<i64, Error> {
        Ok(self.read_u64_le()? as i64)
    }

    /// Reads a big-endian `i64`.
    fn read_i64_be(&mut self) -> Result<i64, Error> {
        Ok(self.read_u64_be()? as i64)
    }

    /// Reads a variable-length unsigned integer encoded in 7-bit groups.
    fn read_vuint(&mut self) -> Result<u64, Error> {
        let first = self.read_u8()?;
        if first < 0x80 {
            return Ok(first as u64);
        }

        let mut result = (first & 0x7f) as u64;
        for i in 1..10 {
            let byte = self.read_u8()?;
            let low_bits = byte & 0x7f;

            if i == 9 && low_bits > 1 {
                return Err(Error::InvalidVUInt);
            }

            result |= (low_bits as u64) << (7 * i);
            if byte == low_bits {
                return Ok(result);
            }
        }

        Err(Error::InvalidVUInt)
    }

    /// Reads a length-prefixed byte slice into an owned buffer.
    fn read_bytes(&mut self) -> Result<Box<[u8]>, Error> {
        let len = self.read_vuint()? as usize;
        self.read_u8_array(len)
    }

    /// Reads a length-prefixed UTF-8 string.
    fn read_string(&mut self) -> Result<String, Error> {
        let bytes = self.read_bytes()?;
        Ok(String::from_utf8(bytes.into_vec())?)
    }
}

/// A `DataReader` backed by an immutable byte slice.
pub struct ArrayDataReader<'a> {
    bytes: &'a [u8],
}

impl<'a> ArrayDataReader<'a> {
    /// Creates a new reader over the given slice.
    pub fn new(bytes: &'a [u8]) -> ArrayDataReader<'a> {
        ArrayDataReader { bytes }
    }
}

impl DataReader for ArrayDataReader<'_> {
    fn remaining(&self) -> usize {
        self.bytes.len()
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        if self.bytes.len() >= N {
            let (head, tail) = self.bytes.split_at(N);
            let arr: [u8; N] = head.try_into().map_err(|_| Error::UnexpectedEndOfFile)?;
            self.bytes = tail;
            Ok(arr)
        } else {
            Err(Error::UnexpectedEndOfFile)
        }
    }

    fn read_u8_array(&mut self, size: usize) -> Result<Box<[u8]>, Error> {
        if self.bytes.len() >= size {
            let (head, tail) = self.bytes.split_at(size);
            self.bytes = tail;
            Ok(head.into())
        } else {
            Err(Error::UnexpectedEndOfFile)
        }
    }
}

/// A byte writer for Janex binary structures.
pub trait DataWriter {
    /// Appends raw bytes to the output.
    fn write_all(&mut self, bytes: &[u8]);

    /// Writes a single byte.
    fn write_u8(&mut self, value: u8) {
        self.write_all(&[value]);
    }

    /// Writes a little-endian `u16`.
    fn write_u16_le(&mut self, value: u16) {
        self.write_all(&value.to_le_bytes());
    }

    /// Writes a big-endian `u16`.
    fn write_u16_be(&mut self, value: u16) {
        self.write_all(&value.to_be_bytes());
    }

    /// Writes a little-endian `u32`.
    fn write_u32_le(&mut self, value: u32) {
        self.write_all(&value.to_le_bytes());
    }

    /// Writes a big-endian `u32`.
    fn write_u32_be(&mut self, value: u32) {
        self.write_all(&value.to_be_bytes());
    }

    /// Writes a little-endian `u64`.
    fn write_u64_le(&mut self, value: u64) {
        self.write_all(&value.to_le_bytes());
    }

    /// Writes a big-endian `u64`.
    fn write_u64_be(&mut self, value: u64) {
        self.write_all(&value.to_be_bytes());
    }

    /// Writes a little-endian `i64`.
    fn write_i64_le(&mut self, value: i64) {
        self.write_u64_le(value as u64);
    }

    /// Writes a big-endian `i64`.
    fn write_i64_be(&mut self, value: i64) {
        self.write_u64_be(value as u64);
    }

    /// Writes a variable-length unsigned integer using 7-bit groups.
    fn write_vuint(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.write_u8(((value as u8) & 0x7f) | 0x80);
            value >>= 7;
        }
        self.write_u8(value as u8);
    }

    /// Writes a length-prefixed byte slice.
    fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_vuint(bytes.len() as u64);
        self.write_all(bytes);
    }

    /// Writes a length-prefixed UTF-8 string.
    fn write_string(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }
}

/// A `DataWriter` that appends encoded bytes into a `Vec<u8>`.
pub struct VecDataWriter {
    bytes: Vec<u8>,
}

impl VecDataWriter {
    /// Creates an empty writer.
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Creates an empty writer with preallocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    /// Returns the encoded bytes accumulated so far.
    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Default for VecDataWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl DataWriter for VecDataWriter {
    fn write_all(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
}

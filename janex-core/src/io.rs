// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::byteorder::ByteOrder;
use crate::error::Error;
use std::marker::PhantomData;

/// A reader for reading data.
pub trait DataReader<BO: ByteOrder> {
    fn remaining(&self) -> usize;

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], Error>;

    fn read_u8_array(&mut self, size: usize) -> Result<Box<[u8]>, Error>;

    fn read_u16_array(&mut self, size: usize) -> Result<Box<[u16]>, Error> {
        Ok(self
            .read_u8_array(size * 2)?
            .chunks_exact(2)
            .map(BO::read_u16)
            .collect())
    }

    fn read_u8(&mut self) -> Result<u8, Error> {
        let [b] = self.read_array()?;
        Ok(b)
    }

    fn read_u16(&mut self) -> Result<u16, Error> {
        let bytes = self.read_array::<2>()?;
        Ok(BO::u16_from_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32, Error> {
        let bytes = self.read_array::<4>()?;
        Ok(BO::u32_from_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64, Error> {
        let bytes = self.read_array::<8>()?;
        Ok(BO::u64_from_bytes(bytes))
    }

    fn read_i64(&mut self) -> Result<i64, Error> {
        Ok(self.read_u64()? as i64)
    }

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

    fn read_bytes(&mut self) -> Result<Box<[u8]>, Error> {
        let len = self.read_vuint()? as usize;
        self.read_u8_array(len)
    }

    fn read_string(&mut self) -> Result<String, Error> {
        let bytes = self.read_bytes()?;
        Ok(String::from_utf8(bytes.into_vec())?)
    }
}

/// A implementation of [`DataReader`] that reads from a slice of bytes.
pub struct ArrayDataReader<'a> {
    bytes: &'a [u8],
}

impl<'a> ArrayDataReader<'a> {
    pub fn new(bytes: &'a [u8]) -> ArrayDataReader<'a> {
        ArrayDataReader { bytes }
    }
}

impl<BO: ByteOrder> DataReader<BO> for ArrayDataReader<'_> {
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

pub trait DataWriter<BO: ByteOrder> {
    fn write_all(&mut self, bytes: &[u8]);

    fn write_u8(&mut self, value: u8) {
        self.write_all(&[value]);
    }

    fn write_u16(&mut self, value: u16) {
        let bytes = if BO::u16_from_bytes([1, 0]) == 0x0100 {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        self.write_all(&bytes);
    }

    fn write_u32(&mut self, value: u32) {
        let bytes = if BO::u16_from_bytes([1, 0]) == 0x0100 {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        self.write_all(&bytes);
    }

    fn write_u64(&mut self, value: u64) {
        let bytes = if BO::u16_from_bytes([1, 0]) == 0x0100 {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        self.write_all(&bytes);
    }

    fn write_i64(&mut self, value: i64) {
        self.write_u64(value as u64);
    }

    fn write_vuint(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.write_u8(((value as u8) & 0x7f) | 0x80);
            value >>= 7;
        }
        self.write_u8(value as u8);
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_vuint(bytes.len() as u64);
        self.write_all(bytes);
    }

    fn write_string(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }
}

pub struct VecDataWriter<BO: ByteOrder> {
    bytes: Vec<u8>,
    byte_order: PhantomData<BO>,
}

impl<BO: ByteOrder> VecDataWriter<BO> {
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            byte_order: PhantomData,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
            byte_order: PhantomData,
        }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl<BO: ByteOrder> Default for VecDataWriter<BO> {
    fn default() -> Self {
        Self::new()
    }
}

impl<BO: ByteOrder> DataWriter<BO> for VecDataWriter<BO> {
    fn write_all(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
}

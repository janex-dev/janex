// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::byteorder::ByteOrder;
use crate::error::Error;

/// A reader for reading data.
pub trait DataReader<BO: ByteOrder> {
    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], Error>;

    fn read_u8_array(&mut self, size: usize) -> Result<Box<[u8]>, Error>;

    fn read_u16_array(&mut self, size: usize) -> Result<Box<[u16]>, Error> {
        Ok(self
            .read_u8_array(size * 2)?
            .chunks_exact(2)
            .map(|chunk| BO::read_u16(chunk))
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

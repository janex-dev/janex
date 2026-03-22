// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::classfile::Error;

/// A reader for reading big-endian data.
pub struct DataReader<'a> {
    bytes: &'a [u8],
}

impl<'a> DataReader<'a> {
    pub fn new(bytes: &'a [u8]) -> DataReader<'a> {
        DataReader { bytes }
    }

    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        if self.bytes.len() >= N {
            let (head, tail) = self.bytes.split_at(N);
            let arr: [u8; N] = head.try_into().map_err(|_| Error::UnexpectedEndOfFile)?;
            self.bytes = tail;
            Ok(arr)
        } else {
            Err(Error::UnexpectedEndOfFile)
        }
    }

    pub fn read_u8(&mut self) -> Result<u8, Error> {
        let [b] = self.read_array()?;
        Ok(b)
    }

    pub fn read_u16(&mut self) -> Result<u16, Error> {
        let bytes = self.read_array::<2>()?;
        Ok(u16::from_be_bytes(bytes))
    }

    pub fn read_u32(&mut self) -> Result<u32, Error> {
        let bytes = self.read_array::<4>()?;
        Ok(u32::from_be_bytes(bytes))
    }

    pub fn read_u8_array(&mut self, size: usize) -> Result<Box<[u8]>, Error> {
        if self.bytes.len() >= size {
            let (head, tail) = self.bytes.split_at(size);
            self.bytes = tail;
            Ok(head.into())
        } else {
            Err(Error::UnexpectedEndOfFile)
        }
    }

    pub fn read_u16_array(&mut self, size: usize) -> Result<Box<[u16]>, Error> {
        let bytes_count = size * 2;
        if self.bytes.len() >= bytes_count {
            let (head, tail) = self.bytes.split_at(bytes_count);
            self.bytes = tail;
            Ok(head
                .chunks_exact(2)
                .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
                .collect())
        } else {
            Err(Error::UnexpectedEndOfFile)
        }
    }
}

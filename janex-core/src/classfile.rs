// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

pub struct ClassFile {}

impl ClassFile {
    pub fn parse(bytes: &[u8]) -> Result<ClassFile, Error> {


        panic!("Not implemented yet")
    }
}

pub enum Error {
    UnexpectedEndOfFile,
}

struct ClassFileParser<'a> {
    bytes: &'a [u8],
}

impl<'a> ClassFileParser<'a> {
    fn new(bytes: &'a [u8]) -> ClassFileParser<'a> {
        ClassFileParser { bytes }
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

    fn read_u8(&mut self) -> Result<u8, Error> {
        let [b] = self.read_array()?;
        Ok(b)
    }

    fn read_u16(&mut self) -> Result<u16, Error> {
        let bytes = self.read_array::<2>()?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32, Error> {
        let bytes = self.read_array::<4>()?;
        Ok(u32::from_be_bytes(bytes))
    }
}

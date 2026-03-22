// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

/// See [`BigEndian`] and [`LittleEndian`].
pub trait ByteOrder {
    /// Reads a single byte from the buffer.
    ///
    /// # Panics
    ///
    /// Panics if the buffer is empty.
    fn read_u8(buf: &[u8]) -> u8 {
        buf[0]
    }

    /// Reads a 16-bit unsigned integer from the buffer.
    fn read_u16(buf: &[u8]) -> u16 {
        Self::u16_from_bytes([buf[0], buf[1]])
    }

    /// Reads a 32-bit unsigned integer from the buffer.
    fn read_u32(buf: &[u8]) -> u32 {
        Self::u32_from_bytes([buf[0], buf[1], buf[2], buf[3]])
    }

    /// Converts a byte array of length 1 to an u8.
    fn u8_from_bytes(bytes: [u8; 1]) -> u8 {
        bytes[0]
    }

    /// Converts a byte array of length 2 to an u16.
    fn u16_from_bytes(bytes: [u8; 2]) -> u16;

    /// Converts a byte array of length 4 to an u32.
    fn u32_from_bytes(bytes: [u8; 4]) -> u32;
}

pub struct BigEndian;
pub struct LittleEndian;

impl ByteOrder for BigEndian {
    fn u16_from_bytes(bytes: [u8; 2]) -> u16 {
        u16::from_be_bytes(bytes)
    }

    fn u32_from_bytes(bytes: [u8; 4]) -> u32 {
        u32::from_be_bytes(bytes)
    }
}

impl ByteOrder for LittleEndian {
    fn u16_from_bytes(bytes: [u8; 2]) -> u16 {
        u16::from_le_bytes(bytes)
    }

    fn u32_from_bytes(bytes: [u8; 4]) -> u32 {
        u32::from_le_bytes(bytes)
    }
}

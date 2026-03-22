// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

pub enum Error {
    UnexpectedEndOfFile,
    InvalidMagicNumber(u32),
    UnknownConstantPoolInfo { tag: u8 },
}
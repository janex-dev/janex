// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

use crate::io::{DataWriter, VecDataWriter};

pub(crate) fn parse(bytes: &[u8]) -> Box<[u8]> {
    bytes.into()
}

pub(crate) fn encode(writer: &mut VecDataWriter, bytes: &[u8]) {
    writer.write_all(bytes);
}

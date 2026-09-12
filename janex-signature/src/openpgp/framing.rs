// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Bounded packet framing before rPGP allocates signature subpackets.

use crate::{
    Error, ErrorKind, Result,
    binary::{Decoder, Limits},
    error::invalid,
};
use pgp::{
    armor::{BlockType, Dearmor, DearmorOptions},
    packet::PacketHeader,
    types::PacketLength,
};
use std::io::Read;
use zeroize::Zeroizing;

/// Reads one packet with a definite length, or an old-format final indeterminate packet.
pub(super) fn packet<'a>(bytes: &mut &'a [u8]) -> Result<(PacketHeader, &'a [u8])> {
    let header = PacketHeader::try_from_reader(&mut *bytes)
        .map_err(|_| invalid("invalid OpenPGP packet header"))?;
    let length = match header.packet_length() {
        PacketLength::Fixed(length) => usize::try_from(length)
            .map_err(|_| invalid("OpenPGP packet length does not fit usize"))?,
        PacketLength::Indeterminate => bytes.len(),
        PacketLength::Partial(_) => {
            return Err(invalid(
                "partial lengths are not valid for signature or key packets",
            ));
        }
    };
    if length > bytes.len() {
        return Err(invalid("truncated OpenPGP packet"));
    }
    let (body, rest) = bytes.split_at(length);
    *bytes = rest;
    Ok((header, body))
}

/// Returns the end of the original hashed signature fields after bounding nested subpackets.
pub(super) fn signature(body: &[u8], limits: Limits) -> Result<usize> {
    let mut count = 0;
    signature_at(body, limits, 0, &mut count)
}

/// Bounds both subpacket areas, including embedded signature bodies, before library decoding.
fn signature_at(body: &[u8], limits: Limits, depth: usize, count: &mut u64) -> Result<usize> {
    if depth > limits.max_depth {
        return Err(Error::new(
            ErrorKind::Limit,
            "OpenPGP signature nesting limit exceeded",
        ));
    }
    let mut reader = Decoder::new(body, limits)?;
    let width = match reader.u8()? {
        4 => 2,
        6 => 4,
        _ => return Err(invalid("OpenPGP signature version must be 4 or 6")),
    };
    reader.take(3)?;
    let length = number(&mut reader, width)?;
    subpackets(reader.take(length)?, limits, depth, count)?;
    let hashed_end = reader.position();
    let length = number(&mut reader, width)?;
    subpackets(reader.take(length)?, limits, depth, count)?;
    Ok(hashed_end)
}

/// Scans subpacket boundaries without allocating collections or interpreting unknown payloads.
fn subpackets(bytes: &[u8], limits: Limits, depth: usize, count: &mut u64) -> Result<()> {
    let mut reader = Decoder::new(bytes, limits)?;
    while reader.remaining() != 0 {
        *count += 1;
        limits.elements(*count)?;
        let subpacket = subpacket(&mut reader)?;
        if subpacket[0] & 0x7f == 32 {
            signature_at(&subpacket[1..], limits, depth + 1, count)?;
        }
    }
    Ok(())
}

/// Reads one complete subpacket, including its type octet but excluding its length.
fn subpacket<'a>(reader: &mut Decoder<'a>) -> Result<&'a [u8]> {
    let length = match reader.u8()? {
        first @ 0..=191 => usize::from(first),
        first @ 192..=254 => ((usize::from(first) - 192) << 8) + usize::from(reader.u8()?) + 192,
        255 => number(reader, 4)?,
    };
    if length == 0 {
        return Err(invalid("empty OpenPGP signature subpacket"));
    }
    reader.take(length)
}

/// Reads a bounded big-endian length, independently of Janex's own integer byte order.
fn number(reader: &mut Decoder<'_>, width: usize) -> Result<usize> {
    let mut number = 0_u64;
    for byte in reader.take(width)? {
        number = (number << 8) | u64::from(*byte);
    }
    reader.limits().bytes(number)
}

/// Borrows embedded signature bodies from both original subpacket areas.
pub(super) fn embedded(body: &[u8], limits: Limits) -> Result<Vec<&[u8]>> {
    let mut reader = Decoder::new(body, limits)?;
    let width = match reader.u8()? {
        4 => 2,
        6 => 4,
        _ => return Err(invalid("unsupported embedded signature version")),
    };
    reader.take(3)?;
    let mut result = Vec::new();
    for _ in 0..2 {
        let length = number(&mut reader, width)?;
        let mut area = Decoder::new(reader.take(length)?, limits)?;
        while area.remaining() != 0 {
            let bytes = subpacket(&mut area)?;
            if bytes.first().is_some_and(|tag| tag & 0x7f == 32) {
                result.push(&bytes[1..]);
                limits.elements(result.len() as u64)?;
            }
        }
    }
    Ok(result)
}

/// Decodes one typed armor block, rejecting non-whitespace prefixes and suffixes.
pub(super) fn armor(bytes: &[u8], typ: BlockType, limits: Limits) -> Result<Zeroizing<Vec<u8>>> {
    for (index, line) in bytes.split(|byte| *byte == b'\n').skip(1).enumerate() {
        if line.trim_ascii().is_empty() {
            break;
        }
        limits.elements(index as u64 + 1)?;
    }
    let mut reader = Dearmor::with_options(
        bytes,
        DearmorOptions::new().set_limit(limits.bytes(limits.max_bytes)?),
    );
    reader
        .read_header()
        .map_err(|_| invalid("invalid OpenPGP armor header"))?;
    if reader.typ != Some(typ) {
        return Err(invalid("unexpected OpenPGP armor block type"));
    }
    let mut decoded = Zeroizing::new(Vec::new());
    // Base64 cannot decode to more bytes than its complete bounded ASCII input.
    reader
        .by_ref()
        .take(bytes.len() as u64 + 1)
        .read_to_end(&mut decoded)
        .map_err(|_| invalid("invalid OpenPGP armor"))?;
    limits.bytes(decoded.len() as u64)?;
    if decoded.len() > bytes.len() {
        return Err(invalid("OpenPGP armor exceeds its encoded size"));
    }
    let (_, _, _, mut remaining) = reader.into_parts();
    let mut buffer = [0; 1024];
    loop {
        let count = remaining
            .read(&mut buffer)
            .map_err(|_| invalid("invalid OpenPGP armor suffix"))?;
        if count == 0 {
            break;
        }
        if !buffer[..count].iter().all(u8::is_ascii_whitespace) {
            return Err(invalid("trailing data after OpenPGP armor"));
        }
    }
    Ok(decoded)
}

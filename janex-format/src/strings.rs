// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! UTF-8 resource-name representations backed by opaque data pools.

use crate::{
    Result,
    binary::{Decoder, write_vuint},
    data_pool::DataPool,
    error::invalid,
};

/// Resolves a data-pool entry as UTF-8 for a resource path or name.
pub fn text(pool: &DataPool, index: u64) -> Result<&str> {
    std::str::from_utf8(pool.get(index)?).map_err(|_| invalid("resource name is not valid UTF-8"))
}

/// Reads and resolves a nonempty name in any of the three permitted representations.
pub fn read_nonempty(pool: &DataPool, decoder: &mut Decoder<'_>) -> Result<String> {
    let index = decoder.vuint()?;
    let value = if index != 0 {
        let text = text(pool, index)?;
        decoder.limits().bytes(text.len() as u64)?;
        text.to_owned()
    } else {
        let inline = decoder.string()?;
        if !inline.is_empty() {
            inline.to_owned()
        } else {
            let count = decoder.count()?;
            if count < 2 {
                return Err(invalid(
                    "string concatenation requires at least two entries",
                ));
            }
            let mut value = String::new();
            for _ in 0..count {
                let item = text(pool, decoder.vuint()?)?;
                let length = value
                    .len()
                    .checked_add(item.len())
                    .ok_or_else(|| invalid("concatenated string length overflow"))?;
                decoder.limits().bytes(length as u64)?;
                value.push_str(item);
            }
            value
        }
    };
    decoder.limits().bytes(value.len() as u64)?;
    if value.is_empty() {
        return Err(invalid("resolved name must not be empty"));
    }
    Ok(value)
}

/// Writes a pooled name, sharing an existing class basename when possible.
///
/// An existing full-string index is preferred. Otherwise an existing basename and
/// the `.class` suffix may be concatenated; other names are interned as full strings.
pub fn write_nonempty(pool: &mut DataPool, value: &str, bytes: &mut Vec<u8>) -> Result<()> {
    if value.is_empty() {
        return Err(invalid("name must not be empty"));
    }
    if let Some(index) = pool.find(value) {
        return write_vuint(bytes, index);
    }
    if let Some(base) = value.strip_suffix(".class").filter(|base| !base.is_empty())
        && let Some(index) = pool.find(base)
    {
        let suffix = pool.intern(".class");
        bytes.extend_from_slice(&[0, 0, 2]);
        write_vuint(&mut *bytes, index)?;
        return write_vuint(bytes, suffix);
    }
    write_vuint(bytes, pool.intern(value))
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Atomic metadata publication and shared registry CBOR encoding.

use crate::{Result, error::invalid};
use janex_format::cbor::Value;
use std::{fs, io::Write, path::Path};

/// Atomically replaces a metadata file after syncing its complete contents.
pub(crate) fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| invalid("metadata path has no parent"))?;
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Converts the registry's JSON-compatible schema to deterministic CBOR with text keys.
pub(crate) fn to_cbor(value: serde_json::Value) -> Result<Value> {
    use serde_json::Value as Json;
    Ok(match value {
        Json::Null => Value::null(),
        Json::Bool(v) => Value::boolean(v),
        Json::String(v) => Value::text(&v),
        Json::Number(v) => Value::uint(
            v.as_u64()
                .ok_or_else(|| invalid("invalid registry number"))?,
        ),
        Json::Array(v) => Value::array(v.into_iter().map(to_cbor).collect::<Result<Vec<_>>>()?),
        Json::Object(v) => Value::map(
            v.into_iter()
                .map(|(k, v)| Ok((Value::text(&k), to_cbor(v)?)))
                .collect::<Result<Vec<_>>>()?,
        )?,
    })
}

/// Decodes only the CBOR types used by the internal registry schema.
pub(crate) fn from_cbor(value: &Value) -> Result<serde_json::Value> {
    use serde_json::Value as Json;
    Ok(match value.as_bytes()[0] >> 5 {
        0 => Json::from(value.as_u64()?),
        3 => Json::from(value.as_text()?),
        4 => Json::Array(
            value
                .as_array()?
                .iter()
                .map(from_cbor)
                .collect::<Result<_>>()?,
        ),
        5 => {
            let mut map = serde_json::Map::new();
            for (key, value) in value.as_map()? {
                if map
                    .insert(key.as_text()?.into(), from_cbor(&value)?)
                    .is_some()
                {
                    return Err(invalid("duplicate registry key"));
                }
            }
            Json::Object(map)
        }
        _ if value.is_null() => Json::Null,
        _ => Json::Bool(value.as_bool()?),
    })
}

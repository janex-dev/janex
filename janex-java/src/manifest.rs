// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! JAR manifest attributes, byte-level continuations, and entry sections.

use crate::Limits;
use crate::{Result, error::invalid};
use std::collections::BTreeMap;

/// Parsed JAR manifest attributes with case-insensitive header names.
///
/// Attribute values retain whitespace and are decoded after joining continuation bytes.
/// Duplicate attributes use the last value, as in the JDK; repeated entry sections merge.
/// The original manifest bytes remain the caller's resource content.
#[derive(Clone, Debug, Default)]
pub struct Manifest {
    /// Main attributes indexed by lowercase ASCII header name.
    main: BTreeMap<String, String>,
    /// Per-entry attributes indexed by exact Name value and lowercase header name.
    entries: BTreeMap<String, BTreeMap<String, String>>,
}

impl Manifest {
    /// Parses newline-terminated manifest sections with CRLF, LF, or CR line endings.
    ///
    /// Orphan continuations, malformed headers, invalid UTF-8 values, and incomplete
    /// final lines are errors. Empty input represents an empty manifest.
    pub fn parse(bytes: &[u8], limits: Limits) -> Result<Self> {
        limits.bytes(bytes.len() as u64)?;
        let mut result = Self::default();
        let mut section = Vec::<(String, Vec<u8>)>::new();
        let mut main = true;
        let mut position = 0;
        while position < bytes.len() {
            let end = bytes[position..]
                .iter()
                .position(|byte| matches!(byte, b'\r' | b'\n'))
                .map(|end| position + end)
                .ok_or_else(|| invalid("manifest ends without a newline"))?;
            let line = &bytes[position..end];
            position = end + 1;
            if bytes[end] == b'\r' && bytes.get(position) == Some(&b'\n') {
                position += 1;
            }
            if line.contains(&0) {
                return Err(invalid("NUL in manifest header"));
            }
            if line.is_empty() {
                result.add_section(&mut section, main)?;
                main = false;
                continue;
            }
            if let Some(continuation) = line.strip_prefix(b" ") {
                let (_, value) = section
                    .last_mut()
                    .ok_or_else(|| invalid("manifest continuation has no header"))?;
                limits.bytes(value.len() as u64 + continuation.len() as u64)?;
                value.extend_from_slice(continuation);
            } else {
                let colon = line
                    .iter()
                    .position(|byte| *byte == b':')
                    .ok_or_else(|| invalid("manifest header has no colon"))?;
                let name = &line[..colon];
                if name.is_empty()
                    || name.len() > 70
                    || !name[0].is_ascii_alphanumeric()
                    || !name
                        .iter()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                    || line.get(colon + 1) != Some(&b' ')
                {
                    return Err(invalid("invalid manifest attribute header"));
                }
                limits.elements(section.len() as u64 + 1)?;
                section.push((
                    String::from_utf8(name.to_ascii_lowercase()).expect("ASCII header"),
                    line[colon + 2..].into(),
                ));
            }
        }
        if !section.is_empty() {
            result.add_section(&mut section, main)?;
        }
        limits.elements(result.entries.len() as u64)?;
        Ok(result)
    }

    /// Returns a main attribute using ASCII case-insensitive header lookup.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.main
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Returns a named entry's attribute, without applying main-section inheritance.
    pub fn entry_attribute(&self, entry: &str, name: &str) -> Option<&str> {
        self.entries
            .get(entry)?
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Returns whether the main Multi-Release attribute equals `true`, ignoring ASCII case.
    pub fn multi_release(&self) -> bool {
        self.get("Multi-Release")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    }

    /// Encodes a runtime manifest without implicit dependency paths or stale JAR signature fields.
    ///
    /// Non-signature attributes and entry sections are retained. The original manifest is
    /// unchanged. Output uses deterministic header order, CRLF, and byte-level continuation
    /// lines; absent Manifest-Version defaults to 1.0.
    pub fn for_runtime(&self) -> Vec<u8> {
        self.for_runtime_with_module_name(None)
    }

    /// Encodes a runtime manifest with an optional Automatic-Module-Name override.
    ///
    /// The override must be a valid Java module name. Other attributes follow `for_runtime`.
    pub fn for_runtime_with_module_name(&self, module_name: Option<&str>) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_header(
            &mut bytes,
            "Manifest-Version",
            self.get("Manifest-Version").unwrap_or("1.0"),
        );
        if let Some(name) = module_name {
            write_header(&mut bytes, "Automatic-Module-Name", name);
        }
        for (name, value) in &self.main {
            if !matches!(name.as_str(), "manifest-version" | "class-path")
                && !signature_header(name)
                && !(name == "automatic-module-name" && module_name.is_some())
            {
                write_header(&mut bytes, name, value);
            }
        }
        bytes.extend_from_slice(b"\r\n");
        for (entry, attributes) in &self.entries {
            if attributes.keys().any(|key| !signature_header(key)) {
                write_header(&mut bytes, "Name", entry);
                for (name, value) in attributes {
                    if !signature_header(name) {
                        write_header(&mut bytes, name, value);
                    }
                }
                bytes.extend_from_slice(b"\r\n");
            }
        }
        bytes
    }

    /// Merges a completed section, interpreting Name only at the start of entry sections.
    fn add_section(&mut self, section: &mut Vec<(String, Vec<u8>)>, main: bool) -> Result<()> {
        if section.is_empty() {
            return Ok(());
        }
        let mut values = section.drain(..).map(|(key, value)| {
            String::from_utf8(value)
                .map(|value| (key, value))
                .map_err(|_| invalid("manifest value is not UTF-8"))
        });
        let target = if main {
            &mut self.main
        } else {
            let (key, name) = values.next().expect("nonempty section")?;
            if key != "name" || name.is_empty() {
                return Err(invalid("manifest entry section must start with Name"));
            }
            self.entries.entry(name).or_default()
        };
        for value in values {
            let (key, value) = value?;
            if key == "name" {
                return Err(invalid("misplaced manifest Name attribute"));
            }
            target.insert(key, value);
        }
        Ok(())
    }
}

/// Identifies signature-only manifest attributes using lowercase ASCII header names.
fn signature_header(name: &str) -> bool {
    matches!(name, "signature-version" | "magic")
        || name.ends_with("-digest")
        || name.contains("-digest-")
}

/// Writes a folded UTF-8 header using at most 72 bytes per line, excluding CRLF.
fn write_header(bytes: &mut Vec<u8>, name: &str, value: &str) {
    let header = format!("{name}: {value}");
    let mut remaining = header.as_bytes();
    let mut capacity = 72;
    while !remaining.is_empty() {
        let length = remaining.len().min(capacity);
        bytes.extend_from_slice(&remaining[..length]);
        bytes.extend_from_slice(b"\r\n");
        remaining = &remaining[length..];
        if !remaining.is_empty() {
            bytes.push(b' ');
            capacity = 71;
        }
    }
}

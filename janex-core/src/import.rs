// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Directory and JAR import into ordered resource layers.

use crate::{Error, Result, error::invalid, manifest::Manifest};
use janex_format::{
    binary::Limits,
    blob::BlobRef,
    cbor::Value,
    condition::Condition,
    content::Content,
    resource::{Directory, DirectoryEntry, Layer, ResourceRoot},
    strings::StringPool,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read},
    path::{Component, Path},
};

/// Bounds for one imported directory or JAR.
#[derive(Clone, Copy, Debug)]
pub struct ImportOptions {
    /// Per-file byte limits, collection limits, and maximum filesystem directory depth.
    pub limits: Limits,
    /// Maximum input-archive size and aggregate uncompressed resource bytes buffered per input.
    pub max_total_bytes: u64,
}

impl Default for ImportOptions {
    /// Limits individual files to format defaults and aggregate content to 512 MiB.
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            max_total_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Imported resource layers and Java metadata before assigning blob references.
#[derive(Debug)]
pub struct ImportedRoot {
    /// Preserved source JAR filename, or `resources.jar` for directory input.
    pub jar_name: String,
    /// Parsed base manifest when present; its original bytes also remain a resource.
    pub manifest: Option<Manifest>,
    /// Unconditional base followed by increasing Java-version layers.
    pub layers: Vec<Layer>,
}

impl ImportedRoot {
    /// Creates a resource root whose string pool will be stored at the supplied reference.
    pub fn into_resource_root(self, string_pool: BlobRef) -> Result<ResourceRoot> {
        Ok(ResourceRoot {
            string_pool,
            strings: StringPool::new(),
            metadata: Value::map([(
                Value::text("janex.java.jar_name"),
                Value::text(&self.jar_name),
            )])?,
            layers: self.layers,
        })
    }
}

/// One imported path before grouping entries into directories and layers.
enum Item {
    /// Explicit directory metadata.
    Directory(Value),
    /// Regular-file bytes and metadata.
    File(Vec<u8>, Value),
    /// Relative symbolic-link target and metadata.
    Link(String, Value),
}

/// Imports one directory or JAR without extracting archive paths or following symbolic links.
///
/// Source contents must remain unchanged during import. Unsupported filesystem node types,
/// duplicate archive entries, malformed manifests, and unrepresentable paths are errors.
/// Regular-file bytes and POSIX permission bits are retained; timestamps are omitted.
/// All content is buffered within `options`; no dependencies are fetched.
pub fn import_path(path: &Path, options: ImportOptions) -> Result<ImportedRoot> {
    let metadata = fs::symlink_metadata(path)?;
    let (jar_name, items) = if metadata.is_dir() {
        let mut items = BTreeMap::new();
        items.insert(String::new(), Item::Directory(permissions(&metadata)?));
        let mut total = 0;
        read_directory(path, "", &mut items, &mut total, options, 0)?;
        ("resources.jar".into(), items)
    } else if metadata.is_file() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("JAR filename is not UTF-8"))?;
        if !name.ends_with(".jar") || name.contains(['/', '\\', '\0']) {
            return Err(invalid("input JAR must have a representable .jar filename"));
        }
        let limit = options.max_total_bytes;
        let bytes = read_bounded(fs::File::open(path)?, limit)?;
        (name.into(), read_jar(&bytes, options)?)
    } else {
        return Err(invalid("input must be a regular JAR file or a directory"));
    };
    build_layers(jar_name, items, options)
}

/// Reads a directory tree, retaining symbolic links as links instead of traversing their targets.
fn read_directory(
    path: &Path,
    relative: &str,
    items: &mut BTreeMap<String, Item>,
    total: &mut u64,
    options: ImportOptions,
    depth: usize,
) -> Result<()> {
    if depth > options.limits.max_depth {
        return Err(invalid("input directory nesting limit exceeded"));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("resource name is not UTF-8"))?;
        let resource = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        validate_path(&resource, options.limits)?;
        options.limits.elements(items.len() as u64 + 1)?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let item = if metadata.file_type().is_symlink() {
            Item::Link(
                link_target(&fs::read_link(entry.path())?)?,
                Value::empty_map(),
            )
        } else if metadata.is_dir() {
            let item = Item::Directory(permissions(&metadata)?);
            items.insert(resource.clone(), item);
            read_directory(&entry.path(), &resource, items, total, options, depth + 1)?;
            continue;
        } else if metadata.is_file() {
            options.limits.bytes(metadata.len())?;
            let limit = options
                .limits
                .max_bytes
                .min(options.max_total_bytes.saturating_sub(*total));
            let bytes = read_bounded(fs::File::open(entry.path())?, limit)?;
            add_size(total, bytes.len() as u64, options)?;
            Item::File(bytes, permissions(&metadata)?)
        } else {
            return Err(invalid(format!("unsupported filesystem node: {resource}")));
        };
        items.insert(resource, item);
    }
    Ok(())
}

/// Reads validated JAR members and checks CRCs through complete entry reads.
fn read_jar(bytes: &[u8], options: ImportOptions) -> Result<BTreeMap<String, Item>> {
    let offset = checked_archive_offset(bytes, options.limits)?;
    let config = zip::read::Config {
        archive_offset: zip::read::ArchiveOffset::Known(offset),
    };
    let mut archive =
        zip::ZipArchive::with_config(config, Cursor::new(bytes)).map_err(zip_error)?;
    check_central_names(
        bytes,
        archive.central_directory_start(),
        archive.len(),
        options.limits,
    )?;
    let mut items = BTreeMap::new();
    let mut total = 0;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(zip_error)?;
        if file.encrypted() {
            return Err(Error::Unsupported(
                "encrypted JAR entries are unsupported".into(),
            ));
        }
        let raw_name = std::str::from_utf8(file.name_raw())
            .map_err(|_| invalid("JAR entry name is not UTF-8"))?;
        let directory = raw_name.ends_with('/');
        let name = raw_name.strip_suffix('/').unwrap_or(raw_name).to_owned();
        validate_path(&name, options.limits)?;
        let mode = file.unix_mode();
        let kind = mode.map(|mode| mode & 0o170000).unwrap_or(0);
        if !matches!(kind, 0 | 0o100000 | 0o040000 | 0o120000) {
            return Err(invalid("unsupported JAR filesystem node"));
        }
        if (directory && kind != 0 && kind != 0o040000) || (!directory && kind == 0o040000) {
            return Err(invalid("JAR directory name disagrees with file mode"));
        }
        options.limits.bytes(file.size())?;
        let expected = file.size();
        let remaining = options.max_total_bytes.saturating_sub(total);
        if expected > remaining {
            return Err(invalid("aggregate import byte limit exceeded"));
        }
        let content = read_bounded(&mut file, expected)?;
        if content.len() as u64 != expected {
            return Err(invalid("JAR entry size mismatch"));
        }
        add_size(&mut total, content.len() as u64, options)?;
        let metadata = if kind == 0o120000 {
            Value::empty_map()
        } else {
            mode_metadata(mode)?
        };
        let item = if directory {
            if !content.is_empty() {
                return Err(invalid("JAR directory has nonempty content"));
            }
            Item::Directory(metadata)
        } else if kind == 0o120000 {
            let target = String::from_utf8(content)
                .map_err(|_| invalid("JAR symbolic-link target is not UTF-8"))?;
            validate_target(&target)?;
            Item::Link(target, metadata)
        } else {
            Item::File(content, metadata)
        };
        if items.insert(name, item).is_some() {
            return Err(invalid("conflicting JAR resource paths"));
        }
    }
    Ok(items)
}

/// Splits valid Multi-Release directories into increasing version layers.
fn build_layers(
    jar_name: String,
    items: BTreeMap<String, Item>,
    options: ImportOptions,
) -> Result<ImportedRoot> {
    let manifest = match items.get("META-INF/MANIFEST.MF") {
        Some(Item::File(bytes, _)) => Some(Manifest::parse(bytes, options.limits)?),
        Some(_) => return Err(invalid("JAR manifest must be a regular file")),
        None => None,
    };
    let multi_release = manifest.as_ref().is_some_and(Manifest::multi_release);
    let mut versions: BTreeMap<u32, BTreeMap<String, Item>> =
        BTreeMap::from([(0, BTreeMap::new())]);
    for (name, item) in items {
        let mut version = 0;
        let mut logical = name.as_str();
        if multi_release && let Some(tail) = name.strip_prefix("META-INF/versions/") {
            let (number, suffix) = tail.split_once('/').unwrap_or((tail, ""));
            if !number.starts_with('0')
                && number.bytes().all(|byte| byte.is_ascii_digit())
                && let Ok(parsed) = number.parse::<u32>()
                && (9..=i32::MAX as u32).contains(&parsed)
            {
                if suffix == "META-INF" || suffix.starts_with("META-INF/") {
                    return Err(invalid(
                        "Multi-Release JAR cannot version META-INF resources",
                    ));
                }
                if suffix.is_empty() && !matches!(item, Item::Directory(_)) {
                    return Err(invalid("Multi-Release version root is not a directory"));
                }
                version = parsed;
                logical = suffix;
            }
        }
        if versions
            .entry(version)
            .or_default()
            .insert(logical.into(), item)
            .is_some()
        {
            return Err(invalid("duplicate resource after Multi-Release mapping"));
        }
    }
    options.limits.elements(versions.len() as u64)?;
    let mut layers = Vec::new();
    for (version, items) in versions {
        let condition = if version == 0 {
            Condition::unconditional()
        } else {
            Condition::from_value(Value::map([(
                Value::uint(5),
                Value::map([
                    (Value::uint(0), Value::text("janex.java")),
                    (
                        Value::uint(1),
                        Value::map([(
                            Value::uint(0),
                            Value::text(&format!("vers:jep322/>={version}")),
                        )])?,
                    ),
                ])?,
            )])?)?
        };
        let mut directories: BTreeMap<String, Directory> = BTreeMap::new();
        for (path, item) in items {
            match item {
                Item::Directory(metadata) => {
                    directories
                        .entry(path.clone())
                        .or_insert_with(|| Directory {
                            path,
                            metadata: Value::empty_map(),
                            entries: Vec::new(),
                        })
                        .metadata = metadata;
                }
                Item::File(bytes, metadata) => {
                    let (parent, name) = parent_name(&path);
                    directories
                        .entry(parent.into())
                        .or_insert_with(|| Directory {
                            path: parent.into(),
                            metadata: Value::empty_map(),
                            entries: Vec::new(),
                        })
                        .entries
                        .push(DirectoryEntry::File {
                            name: name.into(),
                            content: Content::inline(bytes),
                            metadata,
                        });
                }
                Item::Link(target, metadata) => {
                    let (parent, name) = parent_name(&path);
                    directories
                        .entry(parent.into())
                        .or_insert_with(|| Directory {
                            path: parent.into(),
                            metadata: Value::empty_map(),
                            entries: Vec::new(),
                        })
                        .entries
                        .push(DirectoryEntry::SymbolicLink {
                            name: name.into(),
                            target,
                            metadata,
                        });
                }
            }
        }
        for directory in directories.values_mut() {
            directory.entries.sort_by(|a, b| a.name().cmp(b.name()));
        }
        layers.push(Layer {
            condition,
            directories: directories.into_values().collect(),
        });
    }
    // The format validator also catches implicit-parent conflicts across matching version layers.
    let resource = ResourceRoot {
        string_pool: BlobRef { pool: 0, index: 0 },
        strings: StringPool::new(),
        metadata: Value::empty_map(),
        layers,
    };
    let context = janex_format::condition::Context {
        os: String::new(),
        arch: String::new(),
        invocation: None,
        runtime: Some(janex_format::condition::RuntimeContext {
            runtime_type: "janex.java".into(),
            java_version: Some(janex_format::version::JavaVersion::parse(
                &i32::MAX.to_string(),
            )?),
            vendor: String::new(),
        }),
    };
    resource.merge(&context, options.limits)?;
    Ok(ImportedRoot {
        jar_name,
        manifest,
        layers: resource.layers,
    })
}

/// Locates the archive and bounds its declared entry count before allocating a ZIP directory index.
fn checked_archive_offset(bytes: &[u8], limits: Limits) -> Result<u64> {
    let end = (bytes.len().saturating_sub(65557)..bytes.len().saturating_sub(21))
        .rev()
        .find(|&offset| {
            bytes.get(offset..offset + 4) == Some(b"PK\x05\x06")
                && offset + 22 + usize::from(u16_at(bytes, offset + 20)) == bytes.len()
        })
        .ok_or_else(|| invalid("missing terminal ZIP end record"))?;
    if u16_at(bytes, end + 4) != 0 || u16_at(bytes, end + 6) != 0 {
        return Err(invalid("multidisk JAR is unsupported"));
    }
    let count = u16_at(bytes, end + 10);
    if count != u16_at(bytes, end + 8) {
        return Err(invalid("inconsistent ZIP entry count"));
    }
    if count != u16::MAX {
        limits.elements(u64::from(count))?;
    }
    let locator = end
        .checked_sub(20)
        .filter(|&offset| bytes.get(offset..offset + 4) == Some(b"PK\x06\x07"));
    if let Some(locator) = locator {
        let mut found = None;
        for offset in (0..locator.saturating_sub(55)).rev() {
            if bytes.get(offset..offset + 4) == Some(b"PK\x06\x06")
                && u64_at(bytes, offset + 4).checked_add(12) == Some((locator - offset) as u64)
            {
                limits.elements(u64_at(bytes, offset + 32))?;
                let base = (offset as u64)
                    .checked_sub(u64_at(bytes, locator + 8))
                    .ok_or_else(|| invalid("ZIP64 locator offset exceeds its physical position"))?;
                if found.replace(base).is_some() {
                    return Err(invalid("ambiguous ZIP64 end record"));
                }
            }
        }
        return found.ok_or_else(|| invalid("missing ZIP64 end record"));
    } else if count == u16::MAX {
        return Err(invalid("ZIP64 entry count has no locator"));
    }
    let directory_size = u64::from(u32_at(bytes, end + 12));
    let directory_offset = u64::from(u32_at(bytes, end + 16));
    (end as u64)
        .checked_sub(directory_size)
        .and_then(|start| start.checked_sub(directory_offset))
        .ok_or_else(|| invalid("ZIP directory exceeds its physical end"))
}

/// Checks raw central-directory names before the ZIP library's duplicate-name replacement hides them.
fn check_central_names(bytes: &[u8], start: u64, expected: usize, limits: Limits) -> Result<()> {
    let mut position =
        usize::try_from(start).map_err(|_| invalid("ZIP directory offset overflow"))?;
    let mut names = BTreeSet::new();
    while bytes.get(position..position.saturating_add(4)) == Some(b"PK\x01\x02") {
        let header = bytes
            .get(position..position.saturating_add(46))
            .ok_or_else(|| invalid("truncated ZIP directory header"))?;
        let name_length = usize::from(u16_at(header, 28));
        let tail = name_length + usize::from(u16_at(header, 30)) + usize::from(u16_at(header, 32));
        let end = position
            .checked_add(46)
            .and_then(|position| position.checked_add(tail))
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid("ZIP directory entry outside input"))?;
        let name = &bytes[position + 46..position + 46 + name_length];
        if !names.insert(name) {
            return Err(invalid("duplicate JAR entry name"));
        }
        limits.elements(names.len() as u64)?;
        position = end;
    }
    if names.len() != expected {
        return Err(invalid("ZIP directory entry count mismatch"));
    }
    Ok(())
}

/// Reads a stream completely, with at most one byte of lookahead beyond the limit.
fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("import byte limit exceeded"));
    }
    Ok(bytes)
}

/// Updates the aggregate uncompressed size with checked arithmetic.
fn add_size(total: &mut u64, length: u64, options: ImportOptions) -> Result<()> {
    *total = total
        .checked_add(length)
        .filter(|total| *total <= options.max_total_bytes)
        .ok_or_else(|| invalid("aggregate import byte limit exceeded"))?;
    Ok(())
}

/// Checks an imported canonical resource path without applying host path normalization.
fn validate_path(path: &str, limits: Limits) -> Result<()> {
    limits.bytes(path.len() as u64)?;
    if path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(invalid("unrepresentable resource path"));
    }
    Ok(())
}

/// Converts a native relative link target while preserving its navigation components.
fn link_target(path: &Path) -> Result<String> {
    if path
        .components()
        .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return Err(invalid("absolute symbolic-link target cannot be imported"));
    }
    let target = path
        .to_str()
        .ok_or_else(|| invalid("link target is not UTF-8"))?;
    #[cfg(windows)]
    let target = target.replace('\\', "/");
    #[cfg(not(windows))]
    let target = target.to_owned();
    validate_target(&target)?;
    Ok(target)
}

/// Checks a symbolic-link target before resource-tree resolution.
fn validate_target(target: &str) -> Result<()> {
    if target.contains('\0') || target.split('/').any(str::is_empty) {
        return Err(invalid("invalid relative symbolic-link target"));
    }
    Ok(())
}

/// Splits an imported non-directory path into its parent and direct name.
fn parent_name(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// Retains native POSIX permission bits when the host exposes them.
fn permissions(metadata: &fs::Metadata) -> Result<Value> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        mode_metadata(Some(metadata.permissions().mode()))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Ok(Value::empty_map())
    }
}

/// Encodes POSIX permission bits without including filesystem type bits.
fn mode_metadata(mode: Option<u32>) -> Result<Value> {
    Ok(match mode {
        Some(mode) => Value::map([(Value::uint(5), Value::uint(u64::from(mode & 0o7777)))])?,
        None => Value::empty_map(),
    })
}

/// Maps archive errors while retaining underlying operating-system failures.
fn zip_error(error: zip::result::ZipError) -> Error {
    match error {
        zip::result::ZipError::Io(error) => Error::Io(error),
        other => invalid(format!("invalid or unsupported JAR: {other}")),
    }
}

/// Reads a ZIP integer from an already bounded header.
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("bounded ZIP field"),
    )
}
/// Reads a ZIP integer from an already bounded header.
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounded ZIP field"),
    )
}
/// Reads a ZIP64 integer from an already bounded header.
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("bounded ZIP64 field"),
    )
}

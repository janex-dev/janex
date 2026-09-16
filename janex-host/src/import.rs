// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Directory and JAR import into ordered resource layers.

use crate::adapters::java_limits;
use crate::{Result, error::invalid};
use janex_format::{
    binary::Limits,
    blob::BlobRef,
    cbor::Value,
    condition::Condition,
    content::Content,
    data_pool::DataPool,
    resource::{Directory, DirectoryEntry, Layer, ResourceRoot},
};
use janex_java::manifest::Manifest;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
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
    /// Creates a resource root whose data pool will be stored at the supplied reference.
    pub fn into_resource_root(self, data_pool: BlobRef) -> Result<ResourceRoot> {
        Ok(ResourceRoot {
            data_pool,
            data: DataPool::new(),
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

/// Imports owned JAR bytes without extracting paths or consulting the filesystem.
///
/// `jar_name` is the original filename, used for automatic-module naming. Limits, resource
/// validation, metadata retention, and multi-release processing match [`import_path`].
pub fn import_jar(bytes: &[u8], jar_name: &str, options: ImportOptions) -> Result<ImportedRoot> {
    if !jar_name.ends_with(".jar") || jar_name.contains(['/', '\\', '\0']) {
        return Err(invalid("input JAR must have a representable .jar filename"));
    }
    if bytes.len() as u64 > options.max_total_bytes {
        return Err(invalid("input archive byte limit exceeded"));
    }
    build_layers(jar_name.into(), read_jar(bytes, options)?, options)
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
    let mut items = BTreeMap::new();
    for entry in janex_java::jar::read(bytes, java_limits(options.limits), options.max_total_bytes)?
    {
        let raw_name = &entry.name;
        let directory = raw_name.ends_with('/');
        let name = raw_name.strip_suffix('/').unwrap_or(raw_name).to_owned();
        validate_path(&name, options.limits)?;
        let mode = entry.unix_mode;
        let kind = mode.map(|mode| mode & 0o170000).unwrap_or(0);
        if !matches!(kind, 0 | 0o100000 | 0o040000 | 0o120000) {
            return Err(invalid("unsupported JAR filesystem node"));
        }
        if (directory && kind != 0 && kind != 0o040000) || (!directory && kind == 0o040000) {
            return Err(invalid("JAR directory name disagrees with file mode"));
        }
        let content = entry.content;
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
        Some(Item::File(bytes, _)) => Some(Manifest::parse(bytes, java_limits(options.limits))?),
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
        data_pool: BlobRef { pool: 0, index: 0 },
        data: DataPool::new(),
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

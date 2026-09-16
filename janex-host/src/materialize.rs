// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Materialization of merged resource trees as temporary Java path entries.

use crate::adapters::java_limits;
use crate::{Result, error::invalid};
use janex_format::{
    binary::Limits,
    blob::BlobStore,
    cbor::Value,
    condition::Context,
    resource::{Node, ResourceRoot, ResourceTree},
};
use janex_java::manifest::Manifest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, Write},
    path::{Component, Path, PathBuf},
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

/// A generated JAR and the amount of logical data written to it.
#[derive(Debug)]
pub struct MaterializedRoot {
    /// Absolute path to the generated JAR, preserving the root's JAR filename.
    pub path: PathBuf,
    /// Uncompressed bytes actually written, after runtime-manifest rewriting.
    pub logical_bytes: u64,
}

/// Writes one merged root as a new JAR inside an existing caller-owned directory.
///
/// Links expand into files or directory subtrees. Cycles, missing targets, root escapes,
/// invalid native JAR filenames, and outputs exceeding `max_bytes` fail. Entry names are
/// written directly into the archive and never interpreted as native filesystem paths.
///
/// Runtime manifests omit Class-Path and signature attributes; top-level META-INF signature
/// files are omitted. Original resource bytes remain unchanged in the source container.
/// On failure a partially written JAR may remain; the caller owns directory cleanup.
pub fn materialize<R: Read + Seek>(
    root: &ResourceRoot,
    context: &Context,
    blobs: &mut BlobStore<R>,
    directory: &Path,
    max_bytes: u64,
) -> Result<MaterializedRoot> {
    let limits = blobs.reader().limits();
    let tree = root.merge(context, limits)?;
    let name = root.jar_name()?;
    materialize_tree(&tree, name, blobs, directory, max_bytes)
}

/// Writes an already merged tree, preserving the caller's validation and selection work.
pub(crate) fn materialize_tree<R: Read + Seek>(
    tree: &ResourceTree<'_>,
    name: &str,
    blobs: &mut BlobStore<R>,
    directory: &Path,
    max_bytes: u64,
) -> Result<MaterializedRoot> {
    let limits = blobs.reader().limits();
    validate_filename(name)?;
    let path = directory.join(name);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let mut writer = ZipWriter::new(file);
    let mut children = BTreeMap::<&str, Vec<&str>>::new();
    for (path, _) in tree.entries().filter(|(path, _)| !path.is_empty()) {
        let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
        children.entry(parent).or_default().push(path);
    }
    let mut state = Materializer {
        tree,
        children,
        active: BTreeSet::new(),
        count: 0,
        bytes: 0,
        max_bytes,
        limits,
    };
    state.write_directory("", "", &mut writer, blobs, 0)?;
    writer.finish().map_err(zip_error)?;
    Ok(MaterializedRoot {
        path: path.canonicalize()?,
        logical_bytes: state.bytes,
    })
}

/// Tracks one expansion traversal and its output limits.
struct Materializer<'a, 'root> {
    /// Merged source nodes and default data pool.
    tree: &'a ResourceTree<'root>,
    /// Canonical child paths grouped by canonical parent path.
    children: BTreeMap<&'a str, Vec<&'a str>>,
    /// Canonical directories on the active expansion stack.
    active: BTreeSet<String>,
    /// Number of expanded archive entries.
    count: u64,
    /// Total written logical file bytes.
    bytes: u64,
    /// Maximum aggregate expanded file bytes.
    max_bytes: u64,
    /// Limits on expansion depth, path length, and entry count.
    limits: Limits,
}

impl Materializer<'_, '_> {
    /// Writes a directory subtree, rejecting cycles along the canonical source ancestry.
    fn write_directory<R: Read + Seek>(
        &mut self,
        source: &str,
        output: &str,
        writer: &mut ZipWriter<fs::File>,
        blobs: &mut BlobStore<R>,
        depth: usize,
    ) -> Result<()> {
        if depth > self.limits.max_depth {
            return Err(invalid("resource directory expansion limit exceeded"));
        }
        if !self.active.insert(source.into()) {
            return Err(invalid("symbolic-link directory cycle"));
        }
        let children = self.children.get(source).cloned().unwrap_or_default();
        for path in children {
            let name = path.rsplit('/').next().expect("nonempty child path");
            let destination = if output.is_empty() {
                name.into()
            } else {
                format!("{output}/{name}")
            };
            self.limits.bytes(destination.len() as u64)?;
            let (canonical, node) = self.tree.resolve(path)?;
            if matches!(node, Node::File { .. }) && jar_signature(&destination) {
                continue;
            }
            self.count += 1;
            self.limits.elements(self.count)?;
            match node {
                Node::Directory(metadata) => {
                    writer
                        .add_directory(format!("{destination}/"), zip_options(*metadata)?)
                        .map_err(zip_error)?;
                    self.write_directory(canonical, &destination, writer, blobs, depth + 1)?;
                }
                Node::File { metadata, .. } => {
                    let mut bytes = self.tree.read_file(canonical, blobs)?;
                    if destination.eq_ignore_ascii_case("META-INF/MANIFEST.MF") {
                        bytes = Manifest::parse(&bytes, java_limits(self.limits))?.for_runtime();
                    }
                    self.bytes = self
                        .bytes
                        .checked_add(bytes.len() as u64)
                        .filter(|size| *size <= self.max_bytes)
                        .ok_or_else(|| invalid("materialized resource byte limit exceeded"))?;
                    writer
                        .start_file(destination, zip_options(Some(metadata))?)
                        .map_err(zip_error)?;
                    writer.write_all(&bytes)?;
                }
                Node::SymbolicLink { .. } => {
                    unreachable!("resolve follows every final symbolic link")
                }
            }
        }
        self.active.remove(source);
        Ok(())
    }
}

/// Selects stable ZIP metadata and normal POSIX access permissions when specified.
fn zip_options(metadata: Option<&Value>) -> Result<SimpleFileOptions> {
    let mut options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    if let Some(metadata) = metadata
        && let Some(mode) = metadata.get(5)?
    {
        options = options.unix_permissions(mode.as_u64()? as u32);
    }
    Ok(options)
}

/// Identifies standard JAR signature files directly inside META-INF.
pub(crate) fn jar_signature(path: &str) -> bool {
    let Some((parent, name)) = path.rsplit_once('/') else {
        return false;
    };
    if !parent.eq_ignore_ascii_case("META-INF") {
        return false;
    }
    let name = name.to_ascii_uppercase();
    name.starts_with("SIG-")
        || [".SF", ".RSA", ".DSA", ".EC"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

/// Ensures a resource JAR filename cannot change the native destination directory.
fn validate_filename(name: &str) -> Result<()> {
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(invalid(
            "JAR filename is not a native single-component path",
        ));
    }
    #[cfg(windows)]
    {
        let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|number| {
                    matches!(
                        number,
                        "1" | "2"
                            | "3"
                            | "4"
                            | "5"
                            | "6"
                            | "7"
                            | "8"
                            | "9"
                            | "\u{00b9}"
                            | "\u{00b2}"
                            | "\u{00b3}"
                    )
                })
            });
        if device || name.contains([':', '<', '>', '"', '|', '?', '*']) {
            return Err(invalid("JAR filename is invalid on Windows"));
        }
    }
    Ok(())
}

/// Maps archive-write errors while retaining underlying I/O failures.
fn zip_error(error: zip::result::ZipError) -> crate::Error {
    match error {
        zip::result::ZipError::Io(error) => error.into(),
        other => invalid(format!("cannot write runtime JAR: {other}")),
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Resource-root encoding, conditional layer merging, and in-tree symbolic links.

use crate::{
    Error, ErrorKind, Result,
    binary::{Decoder, Limits, write_vuint},
    blob::{BlobRef, BlobStore},
    cbor::{self, Value},
    checksum::Checksum,
    condition::{Condition, Context, nonempty},
    container::integer_keys,
    content::Content,
    data_pool::{DataPool, DataPoolBuilder},
    error::invalid,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{Read, Seek},
    ops::Deref,
};

/// A named file, symbolic link, or removal in one layer directory.
#[derive(Clone, Debug)]
pub enum DirectoryEntry {
    /// A regular file whose checksum, if present, covers restored logical bytes.
    File {
        /// Nonempty single-component name.
        name: String,
        /// Inline or blob-backed logical file content.
        content: Content,
        /// Integer-keyed resource metadata, retaining extensions.
        metadata: Value,
    },
    /// A link resolved relative to its containing directory in the merged tree.
    SymbolicLink {
        /// Nonempty single-component name.
        name: String,
        /// Relative target with nonempty components; `.` and `..` are permitted.
        target: String,
        /// Resource metadata without checksums or POSIX permissions.
        metadata: Value,
    },
    /// Removes an earlier file or link; it never removes a directory.
    Tombstone {
        /// Single-component name to remove.
        name: String,
    },
}

impl DirectoryEntry {
    /// Returns the direct entry name for every variant.
    pub fn name(&self) -> &str {
        match self {
            Self::File { name, .. }
            | Self::SymbolicLink { name, .. }
            | Self::Tombstone { name } => name,
        }
    }

    /// Reads one entry and resolves names using the root pool.
    fn read(decoder: &mut Decoder<'_>, strings: &DataPool) -> Result<Self> {
        let tag = decoder.u32()?;
        let name = crate::strings::read_nonempty(strings, decoder)?;
        match tag {
            0x00534552 => Ok(Self::File {
                name,
                content: Content::read(decoder)?,
                metadata: cbor::read_sized(decoder)?,
            }),
            0x4c4d5953 => Ok(Self::SymbolicLink {
                name,
                target: crate::strings::read_nonempty(strings, decoder)?,
                metadata: cbor::read_sized(decoder)?,
            }),
            0x424d4f54 => Ok(Self::Tombstone { name }),
            _ => Err(invalid("unknown resource entry type")),
        }
    }

    /// Writes one entry, interning names into the root pool as needed.
    fn write(&self, bytes: &mut Vec<u8>, strings: &mut DataPoolBuilder) -> Result<()> {
        let tag: u32 = match self {
            Self::File { .. } => 0x00534552,
            Self::SymbolicLink { .. } => 0x4c4d5953,
            Self::Tombstone { .. } => 0x424d4f54,
        };
        bytes.extend_from_slice(&tag.to_le_bytes());
        crate::strings::write_nonempty(strings, self.name(), bytes)?;
        match self {
            Self::File {
                content, metadata, ..
            } => {
                content.write(bytes)?;
                cbor::write_sized(bytes, metadata)?;
            }
            Self::SymbolicLink {
                target, metadata, ..
            } => {
                crate::strings::write_nonempty(strings, target, bytes)?;
                cbor::write_sized(bytes, metadata)?;
            }
            Self::Tombstone { .. } => {}
        }
        Ok(())
    }
}

/// An explicit directory record; missing parent directories are implicit.
#[derive(Clone, Debug)]
pub struct Directory {
    /// Root-relative slash-separated path, or the empty string for the root.
    pub path: String,
    /// Metadata replacing earlier metadata at this directory path.
    pub metadata: Value,
    /// Direct entries, unique and sorted by resolved UTF-8 name bytes.
    pub entries: Vec<DirectoryEntry>,
}

/// One conditional layer, applied after all earlier matching layers.
#[derive(Clone, Debug)]
pub struct Layer {
    /// Evaluated using the caller's context; an empty map matches unconditionally.
    pub condition: Condition,
    /// Unique explicit directories sorted by UTF-8 path bytes.
    pub directories: Vec<Directory>,
}

/// A layered resource root with its resolved default data pool.
///
/// Fields may be edited before encoding. Encoding and merging revalidate paths, metadata,
/// ordering, and per-layer conflicts. File content is loaded only when requested.
#[derive(Clone, Debug)]
pub struct ResourceRoot {
    /// The blob reference written for the root's shared data pool.
    pub data_pool: BlobRef,
    /// Resolved pool used by names and CLASSFILE transforms without an override.
    pub data: DataPool,
    /// Text-keyed root metadata, retaining extensions.
    pub metadata: Value,
    /// Layers in application order.
    pub layers: Vec<Layer>,
}

/// Owns a structurally validated root without exposing mutable access to its fields.
/// Merging reuses validation unless the caller supplies stricter limits. Conditions and
/// cross-layer conflicts are still checked for every merge.
#[derive(Clone, Debug)]
pub struct ValidatedRoot {
    /// Immutable root storage.
    root: ResourceRoot,
    /// Limits under which the structure was validated.
    limits: Limits,
}

impl ValidatedRoot {
    /// Validates and takes ownership of an editable root without copying its data.
    pub fn new(root: ResourceRoot, limits: Limits) -> Result<Self> {
        root.validate(limits)?;
        Ok(Self { root, limits })
    }

    /// Decodes and validates a root once, retaining the blob reader's structural limits.
    pub fn decode<R: Read + Seek>(bytes: &[u8], blobs: &mut BlobStore<R>) -> Result<Self> {
        let root = ResourceRoot::decode(bytes, blobs)?;
        Ok(Self {
            root,
            limits: blobs.reader().limits(),
        })
    }

    /// Merges matching layers, rechecking structure only when any limit is stricter.
    /// The returned tree borrows this root; file payloads remain lazy.
    pub fn merge(&self, context: &Context, limits: Limits) -> Result<ResourceTree<'_>> {
        if limits.max_bytes < self.limits.max_bytes
            || limits.max_elements < self.limits.max_elements
            || limits.max_depth < self.limits.max_depth
        {
            self.root.validate(limits)?;
        }
        self.root.merge_validated(context, limits)
    }
}

impl Deref for ValidatedRoot {
    type Target = ResourceRoot;

    /// Borrows the root without allowing edits that would invalidate its validation state.
    fn deref(&self) -> &ResourceRoot {
        &self.root
    }
}

impl ResourceRoot {
    /// Reads a complete root and its pool, resolving all directory-entry blobs.
    ///
    /// Even nonmatching layers are structurally checked. Regular-file blobs and
    /// transform override pools remain lazy until their content is requested.
    pub fn decode<R: Read + Seek>(bytes: &[u8], blobs: &mut BlobStore<R>) -> Result<Self> {
        let limits = blobs.reader().limits();
        let mut decoder = Decoder::new(bytes, limits)?;
        let data_pool = BlobRef::read(&mut decoder)?;
        let strings = DataPool::decode(&blobs.resolve(data_pool)?, limits)?;
        let metadata = cbor::read_sized(&mut decoder)?;
        let mut layers = Vec::new();
        for _ in 0..decoder.count()? {
            let condition = Condition::from_value(cbor::read_sized(&mut decoder)?)?;
            let mut directories = Vec::new();
            for _ in 0..decoder.count()? {
                let path = crate::strings::text(&strings, decoder.vuint()?)?.to_owned();
                let metadata = cbor::read_sized(&mut decoder)?;
                let count = decoder.count()?;
                let content = Content::read(&mut decoder)?;
                let bytes = content.resolve_entries(blobs)?;
                let mut entries_decoder = Decoder::new(&bytes, limits)?;
                let mut entries = Vec::new();
                for _ in 0..count {
                    entries.push(DirectoryEntry::read(&mut entries_decoder, &strings)?);
                }
                entries_decoder.finish()?;
                directories.push(Directory {
                    path,
                    metadata,
                    entries,
                });
            }
            layers.push(Layer {
                condition,
                directories,
            });
        }
        decoder.finish()?;
        let root = Self {
            data_pool,
            data: strings,
            metadata,
            layers,
        };
        root.validate(limits)?;
        Ok(root)
    }

    /// Encodes a root using inline directory-entry arrays and interns any missing names.
    ///
    /// Encode `data` after this call and store it at `data_pool`. Existing pool
    /// indices remain stable. Failure may leave newly interned strings in the pool.
    pub fn encode(&mut self, limits: Limits) -> Result<Vec<u8>> {
        self.validate(limits)?;
        let mut data = DataPoolBuilder::from(std::mem::take(&mut self.data));
        let result = self.encode_with_pool(limits, &mut data);
        self.data = data.finish();
        result
    }

    /// Encodes names using an encoding-only reverse index prepared by the caller.
    fn encode_with_pool(&self, limits: Limits, data: &mut DataPoolBuilder) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.data_pool.write(&mut bytes)?;
        cbor::write_sized(&mut bytes, &self.metadata)?;
        write_vuint(&mut bytes, self.layers.len() as u64)?;
        for layer in &self.layers {
            cbor::write_sized(&mut bytes, layer.condition.value())?;
            write_vuint(&mut bytes, layer.directories.len() as u64)?;
            for directory in &layer.directories {
                write_vuint(&mut bytes, data.intern(&directory.path))?;
                cbor::write_sized(&mut bytes, &directory.metadata)?;
                write_vuint(&mut bytes, directory.entries.len() as u64)?;
                let mut entries = Vec::new();
                for entry in &directory.entries {
                    entry.write(&mut entries, data)?;
                    limits.bytes(entries.len() as u64)?;
                }
                Content::inline(entries).write(&mut bytes)?;
                limits.bytes(bytes.len() as u64)?;
            }
        }
        limits.bytes(bytes.len() as u64)?;
        Ok(bytes)
    }

    /// Returns the preserved JAR filename, or `resources.jar` when absent.
    pub fn jar_name(&self) -> Result<&str> {
        // Borrow the text directly from the original map to avoid returning a copied value.
        let mut decoder = minicbor::Decoder::new(self.metadata.as_bytes());
        let count = decoder
            .map()
            .map_err(|_| invalid("root metadata must be a map"))?
            .ok_or_else(|| invalid("indefinite metadata map"))?;
        for _ in 0..count {
            let key = decoder
                .str()
                .map_err(|_| invalid("root metadata key must be text"))?;
            if key == "janex.java.jar_name" {
                let name = decoder
                    .str()
                    .map_err(|_| invalid("JAR filename must be text"))?;
                validate_jar_name(name)?;
                return Ok(name);
            }
            decoder
                .skip()
                .map_err(|_| invalid("invalid root metadata value"))?;
        }
        Ok("resources.jar")
    }

    /// Validates and merges matching layers into a borrowed resource-tree snapshot.
    ///
    /// Tombstones apply before additions within each layer. Directory metadata is
    /// replaced only by explicit directory records; implicit parents retain it.
    pub fn merge(&self, context: &Context, limits: Limits) -> Result<ResourceTree<'_>> {
        self.validate(limits)?;
        self.merge_validated(context, limits)
    }

    /// Applies matching layers after structural validation under the supplied limits.
    fn merge_validated(&self, context: &Context, limits: Limits) -> Result<ResourceTree<'_>> {
        let mut tree = ResourceTree {
            nodes: BTreeMap::from([(String::new(), Node::Directory(None))]),
            data: &self.data,
            limits,
        };
        for layer in &self.layers {
            if !layer.condition.matches(context)? {
                continue;
            }
            for directory in &layer.directories {
                for entry in &directory.entries {
                    if let DirectoryEntry::Tombstone { name } = entry {
                        let path = joined(&directory.path, name);
                        if !matches!(tree.nodes.get(&path), Some(Node::Directory(_))) {
                            tree.nodes.remove(&path);
                        }
                    }
                }
            }
            for directory in &layer.directories {
                tree.add_directory(&directory.path)?;
                tree.nodes.insert(
                    directory.path.clone(),
                    Node::Directory(Some(&directory.metadata)),
                );
                for entry in &directory.entries {
                    let node = match entry {
                        DirectoryEntry::File {
                            content, metadata, ..
                        } => Node::File { content, metadata },
                        DirectoryEntry::SymbolicLink {
                            target, metadata, ..
                        } => Node::SymbolicLink { target, metadata },
                        DirectoryEntry::Tombstone { .. } => continue,
                    };
                    let path = joined(&directory.path, entry.name());
                    if matches!(tree.nodes.get(&path), Some(Node::Directory(_))) {
                        return Err(invalid(format!(
                            "resource directory conflicts with entry: {path}"
                        )));
                    }
                    tree.nodes.insert(path, node);
                    limits.elements(tree.nodes.len() as u64)?;
                }
            }
        }
        Ok(tree)
    }

    /// Checks structure independently of condition matching or file-content access.
    fn validate(&self, limits: Limits) -> Result<()> {
        for (key, _) in self.metadata.as_map()? {
            nonempty(&key)?;
        }
        self.jar_name()?;
        limits.elements(self.layers.len() as u64)?;
        for layer in &self.layers {
            limits.elements(layer.directories.len() as u64)?;
            let mut previous: Option<&str> = None;
            let mut directories = BTreeSet::from([""]);
            let mut files = BTreeSet::new();
            for directory in &layer.directories {
                validate_path(&directory.path, true, false, limits)?;
                if previous.is_some_and(|path| path >= directory.path.as_str()) {
                    return Err(invalid("directory paths must be unique and sorted"));
                }
                previous = Some(&directory.path);
                validate_metadata(&directory.metadata, false, true)?;
                let mut path = directory.path.as_str();
                loop {
                    if !directories.insert(path) {
                        break;
                    }
                    limits.elements(directories.len() as u64)?;
                    let Some((parent, _)) = path.rsplit_once('/') else {
                        break;
                    };
                    path = parent;
                }
                limits.elements(directory.entries.len() as u64)?;
                let mut previous: Option<&str> = None;
                for entry in &directory.entries {
                    let name = entry.name();
                    if name.is_empty() || name.contains('/') || matches!(name, "." | "..") {
                        return Err(invalid("invalid resource entry name"));
                    }
                    if previous.is_some_and(|previous| previous >= name) {
                        return Err(invalid("entry names must be unique and sorted"));
                    }
                    previous = Some(name);
                    limits.bytes(
                        directory.path.len() as u64
                            + name.len() as u64
                            + u64::from(!directory.path.is_empty()),
                    )?;
                    match entry {
                        DirectoryEntry::File { metadata, .. } => {
                            validate_metadata(metadata, true, true)?;
                        }
                        DirectoryEntry::SymbolicLink {
                            target, metadata, ..
                        } => {
                            validate_path(target, false, true, limits)?;
                            validate_metadata(metadata, false, false)?;
                        }
                        DirectoryEntry::Tombstone { .. } => continue,
                    }
                    files.insert(joined(&directory.path, name));
                    limits.elements(files.len() as u64)?;
                }
            }
            if directories.iter().any(|path| files.contains(*path)) {
                return Err(invalid(
                    "file or symbolic link conflicts with a layer directory",
                ));
            }
        }
        Ok(())
    }
}

/// A merged node borrowing its content and metadata from the resource root.
#[derive(Clone, Copy, Debug)]
pub enum Node<'a> {
    /// A directory; `None` means an implicit directory without metadata.
    Directory(Option<&'a Value>),
    /// A regular file whose logical bytes have not necessarily been loaded.
    File {
        /// Logical file content.
        content: &'a Content,
        /// Complete resource metadata.
        metadata: &'a Value,
    },
    /// An unresolved symbolic link.
    SymbolicLink {
        /// Relative target as stored in the resource root.
        target: &'a str,
        /// Complete resource metadata.
        metadata: &'a Value,
    },
}

/// A merged tree borrowing one root, with paths in UTF-8 byte order.
///
/// Links are resolved on demand. Iteration does not expand links to directories;
/// materializers must guard against cycles when recursively traversing those links.
pub struct ResourceTree<'a> {
    /// Canonical paths, including the empty root path.
    nodes: BTreeMap<String, Node<'a>>,
    /// Default pool for regular-file transforms.
    data: &'a DataPool,
    /// Bounds on merged collections and link expansion.
    limits: Limits,
}

impl<'a> ResourceTree<'a> {
    /// Iterates canonical paths and unresolved nodes, including the root directory.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &Node<'a>)> {
        self.nodes.iter().map(|(path, node)| (path.as_str(), node))
    }

    /// Looks up an exact canonical path without following symbolic links.
    pub fn get(&self, path: &str) -> Option<&Node<'a>> {
        self.nodes.get(path)
    }

    /// Resolves a relative path component by component and returns its canonical target.
    ///
    /// The empty path denotes the root. Links resolve before subsequent `..` components;
    /// escaping the root, missing targets, and traversing files are errors. At most
    /// `limits.max_depth` links may be followed in one lookup.
    pub fn resolve(&self, path: &str) -> Result<(&str, &Node<'a>)> {
        validate_path(path, true, true, self.limits)?;
        let mut pending: VecDeque<String> = if path.is_empty() {
            VecDeque::new()
        } else {
            path.split('/').map(str::to_owned).collect()
        };
        let mut current = Vec::<String>::new();
        let mut followed = 0usize;
        while let Some(component) = pending.pop_front() {
            let parent = current.join("/");
            if !matches!(self.nodes.get(&parent), Some(Node::Directory(_))) {
                return Err(invalid("resource path traverses a nondirectory"));
            }
            match component.as_str() {
                "." => continue,
                ".." => {
                    current
                        .pop()
                        .ok_or_else(|| invalid("symbolic link escapes resource root"))?;
                    continue;
                }
                _ => {}
            }
            let candidate = joined(&parent, &component);
            self.limits.bytes(candidate.len() as u64)?;
            match self
                .nodes
                .get(&candidate)
                .ok_or_else(|| invalid(format!("missing resource: {candidate}")))?
            {
                Node::SymbolicLink { target, .. } => {
                    followed += 1;
                    if followed > self.limits.max_depth {
                        return Err(Error::new(
                            ErrorKind::Limit,
                            "symbolic-link expansion limit exceeded",
                        ));
                    }
                    self.limits
                        .elements(pending.len() as u64 + target.split('/').count() as u64)?;
                    for part in target.split('/').rev() {
                        pending.push_front(part.into());
                    }
                }
                _ => current.push(component),
            }
        }
        let path = current.join("/");
        let (path, node) = self
            .nodes
            .get_key_value(&path)
            .ok_or_else(|| invalid("missing resolved resource"))?;
        Ok((path, node))
    }

    /// Resolves a file, restores its transforms, and verifies its optional logical checksum.
    pub fn read_file<R: Read + Seek>(
        &self,
        path: &str,
        blobs: &mut BlobStore<R>,
    ) -> Result<Vec<u8>> {
        let (_, node) = self.resolve(path)?;
        let Node::File { content, metadata } = node else {
            return Err(invalid("resource is not a regular file"));
        };
        let bytes = content.resolve_file(blobs, self.data)?;
        if let Some(checksum) = metadata.get(0)? {
            Checksum::decode(checksum.as_byte_string()?)?.verify(bytes.as_slice())?;
        }
        Ok(bytes)
    }

    /// Adds implicit parents without replacing existing explicit directory metadata.
    fn add_directory(&mut self, path: &str) -> Result<()> {
        let mut current = path;
        while !current.is_empty() {
            match self.nodes.get(current) {
                Some(Node::Directory(_)) => break,
                Some(_) => {
                    return Err(invalid(format!(
                        "resource entry conflicts with directory: {current}"
                    )));
                }
                None => {
                    self.nodes.insert(current.to_owned(), Node::Directory(None));
                }
            }
            self.limits.elements(self.nodes.len() as u64)?;
            current = current.rsplit_once('/').map_or("", |(parent, _)| parent);
        }
        Ok(())
    }
}

/// Validates metadata fields according to the kind of resource carrying them.
fn validate_metadata(value: &Value, file: bool, permissions: bool) -> Result<()> {
    integer_keys(value)?;
    if let Some(checksum) = value.get(0)? {
        if !file {
            return Err(invalid("only regular files may have resource checksums"));
        }
        Checksum::decode(checksum.as_byte_string()?)?;
    }
    if let Some(comment) = value.get(1)? {
        comment.as_text()?;
    }
    for key in [2, 3, 4] {
        if let Some(time) = value.get(key)? {
            time.as_i128()?;
        }
    }
    if let Some(mode) = value.get(5)?
        && (!permissions || mode.as_u64()? > 4095)
    {
        return Err(invalid("invalid POSIX permissions for resource kind"));
    }
    Ok(())
}

/// Checks a relative slash-separated path, optionally allowing the root and navigation components.
fn validate_path(path: &str, root: bool, navigation: bool, limits: Limits) -> Result<()> {
    limits.bytes(path.len() as u64)?;
    if root && path.is_empty() {
        return Ok(());
    }
    let mut count = 0u64;
    for part in path.split('/') {
        if part.is_empty() || (!navigation && matches!(part, "." | "..")) {
            return Err(invalid("invalid relative resource path"));
        }
        count += 1;
    }
    limits.elements(count)?;
    Ok(())
}

/// Joins an already validated directory path and a single component.
fn joined(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.into()
    } else {
        format!("{parent}/{name}")
    }
}

/// Checks the format's portable JAR filename attribute.
fn validate_jar_name(name: &str) -> Result<()> {
    if !name.ends_with(".jar") || name.contains(['/', '\\', '\0']) {
        return Err(invalid("invalid resource-root JAR filename"));
    }
    Ok(())
}

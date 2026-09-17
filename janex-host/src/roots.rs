// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Launch-owned resource roots from the authenticated container and resolved dependencies.

use crate::{
    Result, dependency,
    error::invalid,
    import::{ImportOptions, import_jar},
};
use janex_format::{
    application::PathEntry,
    blob::{BlobRef, BlobStore},
    content::Source,
    resource::{DirectoryEntry, ValidatedRoot},
};
use std::{collections::BTreeMap, io::Cursor};

/// Distinguishes local roots from external content constrained by its URI and checksum.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum RootKey {
    /// Container-local blob identity.
    Local(BlobRef),
    /// Exact external name and optional encoded content constraint.
    External(String, Vec<u8>),
}

impl RootKey {
    /// Captures the full identity needed to avoid reusing a differently constrained dependency.
    pub(crate) fn of(entry: &PathEntry) -> Self {
        match entry {
            PathEntry::Local(reference) => Self::Local(*reference),
            PathEntry::External { uri, checksum } => Self::External(
                uri.clone(),
                checksum
                    .as_ref()
                    .map_or_else(Vec::new, |value| value.encode()),
            ),
        }
    }
}

/// Owns decoded roots across runtime candidates; external bytes never alias persistent cache files.
#[derive(Default)]
pub(crate) struct Roots {
    /// Roots indexed by complete reference identity.
    pub(crate) entries: BTreeMap<RootKey, ValidatedRoot>,
    /// Acquired, verified external archives, imported only when native preparation needs a tree.
    pub(crate) archives: BTreeMap<RootKey, dependency::Dependency>,
    /// Aggregate imported bytes, including all multi-release layers, retained in memory.
    imported_bytes: u64,
    /// Aggregate raw JAR bytes fetched or read from cache for this launch.
    acquired_bytes: u64,
    /// Aggregate external import allowance shared across candidate runtimes.
    acquired_limit: u64,
}

impl Roots {
    /// Resolves explicit external entries once after runtime conditions and authentication succeed.
    pub(crate) fn acquire<'a>(
        &mut self,
        entries: impl Iterator<Item = &'a PathEntry>,
        options: &dependency::DependencyOptions,
        import: ImportOptions,
        require_secure: bool,
    ) -> Result<()> {
        let maximum = import.max_total_bytes;
        self.acquired_limit = maximum;
        for entry in entries {
            let PathEntry::External { uri, checksum } = entry else {
                continue;
            };
            if entry.module_requirement().is_some()
                || self.archives.contains_key(&RootKey::of(entry))
            {
                continue;
            }
            let mut acquisition = options.clone();
            acquisition.max_bytes = acquisition
                .max_bytes
                .min(maximum.saturating_sub(self.acquired_bytes));
            let resolved =
                dependency::resolve(uri, checksum.as_ref(), &acquisition, require_secure)?;
            self.acquired_bytes += resolved.bytes.len() as u64;
            self.archives.insert(RootKey::of(entry), resolved);
        }
        Ok(())
    }

    /// Returns an acquired external root or lazily decodes a container-local root.
    pub(crate) fn get(
        &mut self,
        entry: &PathEntry,
        blobs: &mut BlobStore<Cursor<Vec<u8>>>,
    ) -> Result<&ValidatedRoot> {
        let key = RootKey::of(entry);
        if !self.entries.contains_key(&key) {
            let root = match entry {
                PathEntry::Local(reference) => {
                    let bytes = blobs.resolve(*reference)?;
                    ValidatedRoot::decode(&bytes, blobs)?
                }
                PathEntry::External { .. } => {
                    let archive = self
                        .archives
                        .get(&key)
                        .ok_or_else(|| invalid("external dependency has not been acquired"))?;
                    let import = ImportOptions {
                        limits: blobs.reader().limits(),
                        max_total_bytes: self.acquired_limit.saturating_sub(self.imported_bytes),
                    };
                    let root = import_jar(&archive.bytes, &archive.jar_name, import)?;
                    for layer in &root.layers {
                        for directory in &layer.directories {
                            for entry in &directory.entries {
                                if let DirectoryEntry::File { content, .. } = entry
                                    && let Source::Inline(bytes) = &content.source
                                {
                                    self.imported_bytes += bytes.len() as u64;
                                }
                            }
                        }
                    }
                    ValidatedRoot::new(
                        root.into_resource_root(BlobRef { pool: 0, index: 0 })?,
                        import.limits,
                    )?
                }
            };
            self.entries.insert(key.clone(), root);
        }
        Ok(&self.entries[&key])
    }
}

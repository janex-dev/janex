// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Descriptor-only module inventory for candidate selection and native startup arguments.

use crate::{Result, adapters::java_limits, error::invalid, roots::Roots};
use janex_format::{application::PathEntry, blob::BlobStore, classfile, condition::Context};
use janex_java::{manifest::Manifest, modules::Module};
use std::io::Cursor;

/// Reads only module descriptors and automatic-module manifests from candidate-selected trees.
pub(crate) fn inventory(
    entries: &[PathEntry],
    context: &Context,
    blobs: &mut BlobStore<Cursor<Vec<u8>>>,
    roots: &mut Roots,
) -> Result<Vec<Module>> {
    let mut modules = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.module_requirement().is_none())
    {
        let root = roots.module(entry, blobs)?;
        let limits = blobs.reader().limits();
        let tree = root.merge(context, limits)?;
        let module = if tree.get("module-info.class").is_some() {
            let bytes = tree.read_file("module-info.class", blobs)?;
            let info = classfile::inspect(&bytes, limits)?
                .module
                .ok_or_else(|| invalid("module-info.class has no module descriptor"))?;
            Module {
                name: info.name,
                version: info.version,
                requires: info
                    .requires
                    .into_iter()
                    .filter(|r| r.flags & 0x0040 == 0)
                    .map(|r| r.name)
                    .collect(),
            }
        } else if let Some(name) = crate::adapters::automatic_module_name(root)? {
            janex_java::modules::automatic(&crate::adapters::jar_name(root)?, Some(&name))
        } else {
            let manifest = tree
                .entries()
                .find(|(path, _)| path.eq_ignore_ascii_case("META-INF/MANIFEST.MF"))
                .map(|(path, _)| {
                    tree.read_file(path, blobs)
                        .map_err(crate::Error::from)
                        .and_then(|bytes| Ok(Manifest::parse(&bytes, java_limits(limits))?))
                })
                .transpose()?;
            janex_java::modules::automatic(
                &crate::adapters::jar_name(root)?,
                manifest
                    .as_ref()
                    .and_then(|m| m.get("Automatic-Module-Name")),
            )
        };
        modules.push(module);
    }
    Ok(modules)
}

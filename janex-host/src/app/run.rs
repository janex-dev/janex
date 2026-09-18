// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Application execution directly from the shared artifact cache.

use super::{AppManager, AppRequest, exact_request, runtime_dependencies};
use crate::{Result, dependency, error::invalid, run::RunOptions};
use std::{io::Cursor, process::ExitStatus};
use url::Url;

impl AppManager {
    /// Acquires a complete runtime classpath before launching, without registering an installation.
    pub(super) fn execute_cached(
        &self,
        request: &AppRequest,
        mut options: RunOptions,
    ) -> Result<ExitStatus> {
        if options.dependencies.cache_directory.is_none() {
            options.dependencies.cache_directory = Some(self.root.join("cache/dependencies"));
        }
        let fetch = |url: &Url, limit| {
            let mut policy = options.dependencies.clone();
            policy.max_bytes = limit;
            dependency::artifact(url.as_str(), &policy)
        };
        let exact = exact_request(request, |url| Ok(fetch(url, 1024 * 1024)?.bytes))?;
        let artifact = fetch(&exact.url(false)?, options.dependencies.max_bytes)?;
        options.target = artifact.path;
        // Explicit package selection has the same unsigned policy as an installed application.
        // Signer pins and signed containers still pass through the normal authentication path.
        options.allow_unsigned = true;
        if exact.kind == "janex" {
            return crate::run::prepare_snapshot(&options, artifact.bytes)?.execute();
        }
        let (main, feature) =
            crate::run::jar_entry(Cursor::new(artifact.bytes), exact.jar.main_class.as_deref())?;
        let dependencies =
            runtime_dependencies(&exact, |url| Ok(fetch(url, 4 * 1024 * 1024)?.bytes))?;
        let repository = Url::parse(&exact.repository).unwrap();
        let mut class_path = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            let artifact = fetch(&dependency.url(&repository), options.dependencies.max_bytes)?;
            zip::ZipArchive::new(Cursor::new(artifact.bytes)).map_err(|e| {
                invalid(format!(
                    "invalid dependency JAR {}: {e}",
                    dependency.filename()
                ))
            })?;
            class_path.push(artifact.path);
        }
        crate::run::prepare_jar(&options, &main, feature, &class_path)?.execute()
    }
}

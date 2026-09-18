// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Janex installation and self-update commands.

use clap::{Args, Subcommand};
use janex_host::{
    Result,
    self_manage::{self, UpdateOptions},
};
use std::path::PathBuf;

/// Operations on the Janex executable itself.
#[derive(Subcommand)]
pub(super) enum SelfCommand {
    /// Copy this executable to JANEX_HOME/bin and initialize shell integration.
    Install,
    /// Update the managed executable from GitHub Releases or a local distribution.
    Update(UpdateArgs),
}

/// Exact release selection or an explicitly trusted local distribution.
#[derive(Args)]
pub(super) struct UpdateArgs {
    /// Exact release version; omitted selects the latest stable release.
    #[arg(long, value_name = "VERSION", conflicts_with = "from")]
    version: Option<String>,
    /// Use a local ZIP or tar.xz distribution without network access.
    #[arg(long, value_name = "ARCHIVE")]
    from: Option<PathBuf>,
    /// Require this SHA-256 digest for the local archive.
    #[arg(long, value_name = "HEX", requires = "from")]
    sha256: Option<String>,
}

/// Installs or updates only the managed Janex binary, leaving SDK and application selections intact.
pub(super) fn run(command: SelfCommand) -> Result<i32> {
    let home = janex_platform::janex_home()?;
    let current = std::env::current_exe()?;
    match command {
        SelfCommand::Install => {
            let target = self_manage::install(&home, &current)?;
            println!("Installed Janex in {}", target.display());
            super::shell::install_for(&target)?;
        }
        SelfCommand::Update(args) => {
            let options = UpdateOptions {
                version: args.version,
                archive: args.from,
                sha256: args.sha256,
            };
            match self_manage::update(&home, &current, &options)? {
                Some(version) => println!("Updated Janex to {version}"),
                None => println!("Janex is already up to date"),
            }
        }
    }
    Ok(0)
}

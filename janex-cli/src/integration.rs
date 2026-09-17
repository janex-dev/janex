// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Operating-system integration commands.

use clap::{Args, Subcommand};
use janex_host::{Result, integration};
use std::path::PathBuf;

/// Explicit registration and distribution export operations.
#[derive(Subcommand)]
pub(super) enum IntegrationCommand {
    /// Register Janex as an available file handler without selecting a default application.
    Register(RegisterArgs),
    /// Inspect registered files and handlers.
    Status(StatusArgs),
    /// Remove unchanged entries installed by Janex.
    Unregister(ScopeArgs),
    /// Generate native-platform registration assets in a new directory.
    Export(ExportArgs),
}

/// Registration scope shared by all operations.
#[derive(Args)]
pub(super) struct ScopeArgs {
    /// Use machine-wide locations; modifying them may require elevated privileges.
    #[arg(long)]
    system: bool,
}

/// Handler scope and execution policy.
#[derive(Args)]
pub(super) struct RegisterArgs {
    /// Scope of the generated handlers.
    #[command(flatten)]
    scope: ScopeArgs,
    /// Also enable direct execution through Linux binfmt_misc.
    #[arg(long, requires = "system")]
    binfmt: bool,
    /// Authentication policy used when opening files.
    #[command(flatten)]
    trust: super::TrustArgs,
}

/// Read-only status options.
#[derive(Args)]
pub(super) struct StatusArgs {
    /// Scope to inspect.
    #[command(flatten)]
    scope: ScopeArgs,
    /// Write machine-readable JSON.
    #[arg(long)]
    json: bool,
}

/// Offline distribution asset options.
#[derive(Args)]
pub(super) struct ExportArgs {
    /// Handler configuration.
    #[command(flatten)]
    registration: RegisterArgs,
    /// New directory for exported assets.
    #[arg(long, value_name = "DIRECTORY")]
    output: PathBuf,
    /// Absolute installed executable path; defaults to this executable.
    #[arg(long, value_name = "PATH")]
    executable: Option<PathBuf>,
}

/// Converts policy paths to absolute Unicode command arguments.
fn options(args: RegisterArgs, executable: Option<PathBuf>) -> Result<integration::Options> {
    let mut trust_arguments = Vec::new();
    if args.trust.allow_unsigned {
        trust_arguments.push("--allow-unsigned".into());
    }
    for (flag, paths) in [
        ("--trust-cms-certificate", args.trust.trust_cms_certificate),
        (
            "--trust-openpgp-key",
            args.trust.trust_openpgp_key.into_iter().collect(),
        ),
        ("--cms-issuer", args.trust.cms_issuer),
        ("--cms-crl", args.trust.cms_crl),
    ] {
        for path in paths {
            let path = janex_java::runtime::java_path(&std::path::absolute(path)?);
            trust_arguments.push(flag.into());
            trust_arguments.push(path.into_os_string().into_string().map_err(|_| {
                janex_host::Error::InvalidInput("trust path is not Unicode".into())
            })?);
        }
    }
    Ok(integration::Options {
        executable: janex_java::runtime::java_path(&match executable {
            Some(path) => path,
            None => std::env::current_exe()?,
        }),
        trust_arguments,
        system: args.scope.system,
        binfmt: args.binfmt,
    })
}

/// Runs one explicit integration operation.
pub(super) fn run(command: IntegrationCommand) -> Result<()> {
    let warnings = match command {
        IntegrationCommand::Register(args) => {
            let warnings = integration::register(&options(args, None)?)?;
            println!("Janex file handler registered.");
            warnings
        }
        IntegrationCommand::Unregister(args) => {
            let warnings = integration::unregister(args.system)?;
            println!("Janex file handler unregistered.");
            warnings
        }
        IntegrationCommand::Export(args) => {
            integration::export(&options(args.registration, args.executable)?, &args.output)?;
            println!("Exported integration assets to {}", args.output.display());
            Vec::new()
        }
        IntegrationCommand::Status(args) => {
            let status = integration::status(args.scope.system)?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&status)
                        .map_err(|e| janex_host::Error::InvalidInput(e.to_string()))?
                );
            } else {
                println!(
                    "Registration: {}",
                    if !status.registered {
                        "absent"
                    } else if status.complete {
                        "complete"
                    } else {
                        "incomplete"
                    }
                );
                if let Some(path) = status.executable {
                    println!("Executable: {}", path.display());
                }
                for problem in status.problems {
                    println!("  {problem}");
                }
            }
            Vec::new()
        }
    };
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

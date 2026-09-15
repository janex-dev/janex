// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Shared CLI entry points for native SDK management.

use clap::{Args, Subcommand, ValueEnum};
use janex_host::{
    Error, Result,
    sdk::{CatalogOptions, Installation, JavaRequest, SdkManager, Shell},
};
use std::{ffi::OsString, path::PathBuf};

/// SDK operations in the shared Janex command namespace.
#[derive(Subcommand)]
pub(super) enum SdkCommand {
    /// List downloadable Java SDK versions without installing them.
    Available(AvailableArgs),
    /// Install SDK versions while retaining all existing versions.
    Install(InstallArgs),
    /// List installed SDKs and their persistent IDs.
    List(ListArgs),
    /// Update saved SDK requirements, retaining old versions and respecting pins.
    Update(UpdateArgs),
    /// Uninstall one exact SDK version or registration.
    Uninstall(TargetArgs),
    /// Set or clear the global SDK default without downloading.
    Default(DefaultArgs),
    /// Show the Java home selected by the shell, project, or global default.
    Current(CurrentArgs),
    /// Print the Java home for an installed SDK target.
    Home(TargetArgs),
    /// Execute a command with a selected SDK's JAVA_HOME and PATH.
    Exec(ExecArgs),
    /// Print shell environment assignments for an installed SDK.
    Env(EnvArgs),
    /// Save an installed Java selection in the current project's toolchain file.
    Use(UseArgs),
    /// Prevent updates from changing a saved SDK requirement's selected build.
    Pin(PinArgs),
    /// Allow updates within a saved SDK version requirement again.
    Unpin(PinArgs),
}

/// Pin policy for one saved requirement and platform variant.
#[derive(Args)]
pub(super) struct PinArgs {
    /// Saved Java requirement.
    target: String,
    /// Variant identifying the saved requirement.
    #[command(flatten)]
    variant: VariantArgs,
}

/// Java archive variant overrides.
#[derive(Args, Default)]
pub(super) struct VariantArgs {
    /// Override the native operating-system architecture.
    #[arg(long, value_name = "ARCH")]
    arch: Option<String>,
    /// Install development tools or only the runtime.
    #[arg(long, value_parser = ["jdk", "jre"], default_value = "jdk")]
    kind: String,
    /// Select a JavaFX-bundled distribution.
    #[arg(long)]
    javafx: bool,
    /// Select the Linux C library independently of the Janex executable's build target.
    #[arg(long, value_parser = ["glibc", "musl"])]
    libc: Option<String>,
}

impl VariantArgs {
    /// Applies explicit CLI variant values to a parsed request.
    fn request(&self, target: &str) -> Result<JavaRequest> {
        let mut request = JavaRequest::parse(target)?;
        if let Some(arch) = &self.arch {
            request.architecture = match arch.as_str() {
                "amd64" | "x64" | "x86_64" => "x86-64",
                "arm64" => "aarch64",
                other => other,
            }
            .into();
        }
        request.kind = self.kind.clone();
        request.javafx = self.javafx;
        if let Some(libc) = &self.libc {
            request.libc = libc.clone();
        }
        request.validate()?;
        Ok(request)
    }
}

/// Catalog listing request.
#[derive(Args)]
pub(super) struct AvailableArgs {
    /// Java vendor and version requirement, for example bellsoft@21.
    #[arg(default_value = "bellsoft@latest")]
    target: String,
    /// Archive variant filters.
    #[command(flatten)]
    variant: VariantArgs,
    /// Use only cached catalog metadata.
    #[arg(long, conflicts_with = "refresh")]
    offline: bool,
    /// Refresh catalog metadata.
    #[arg(long)]
    refresh: bool,
    /// Emit structured JSON.
    #[arg(long)]
    json: bool,
}

/// Installation or external registration request.
#[derive(Args)]
pub(super) struct InstallArgs {
    /// SDK requests; each is installed independently.
    #[arg(required = true)]
    targets: Vec<String>,
    /// Archive variant filters.
    #[command(flatten)]
    variant: VariantArgs,
    /// Fix the selected build against subsequent update commands.
    #[arg(long)]
    pin: bool,
    /// Register an existing Java home without copying or owning it; requires one target.
    #[arg(long, value_name = "JAVA_HOME")]
    path: Option<PathBuf>,
    /// Reuse installed SDKs without network access.
    #[arg(long)]
    offline: bool,
    /// Maximum seconds for an archive download, including segment retries.
    #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    /// Emit structured JSON.
    #[arg(long)]
    json: bool,
}

/// Installed SDK listing options.
#[derive(Args)]
pub(super) struct ListArgs {
    /// Emit structured JSON including saved requests and the global default.
    #[arg(long)]
    json: bool,
}

/// Saved requirement update request.
#[derive(Args)]
pub(super) struct UpdateArgs {
    /// Saved requirements to update; use --all for every request.
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    targets: Vec<String>,
    /// Update every saved SDK requirement.
    #[arg(long)]
    all: bool,
    /// Archive variant filters for explicitly named requests.
    #[command(flatten)]
    variant: VariantArgs,
    /// Maximum seconds per archive download, including segment retries.
    #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    /// Emit structured JSON.
    #[arg(long)]
    json: bool,
}

/// An exact installed target.
#[derive(Args)]
pub(super) struct TargetArgs {
    /// Installed Java target or full installation ID.
    target: String,
}

/// Global default selection request.
#[derive(Args)]
pub(super) struct DefaultArgs {
    /// Installed Java target or full installation ID.
    #[arg(required_unless_present = "clear", conflicts_with = "clear")]
    target: Option<String>,
    /// Remove the global default without uninstalling SDKs.
    #[arg(long)]
    clear: bool,
}

/// Current SDK selection query.
#[derive(Args)]
pub(super) struct CurrentArgs {
    /// Emit structured JSON.
    #[arg(long)]
    json: bool,
}

/// Command and SDK selection for a child process.
#[derive(Args)]
pub(super) struct ExecArgs {
    /// Explicit installed Java target or installation ID.
    #[arg(long, value_name = "TARGET")]
    java: Option<String>,
    /// Child executable and uninterpreted arguments.
    #[arg(required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<OsString>,
}

/// CLI shell names.
#[derive(Clone, Copy, ValueEnum)]
pub(super) enum ShellArg {
    /// sh, bash, or zsh.
    Sh,
    /// PowerShell.
    Powershell,
    /// Fish.
    Fish,
}

/// Shell environment rendering options.
#[derive(Args)]
pub(super) struct EnvArgs {
    /// Explicit installed Java target or installation ID.
    #[arg(long, value_name = "TARGET")]
    java: Option<String>,
    /// Shell whose assignments should be emitted for evaluation by the caller.
    #[arg(long, value_enum)]
    shell: ShellArg,
}

/// Project-local selection request.
#[derive(Args)]
pub(super) struct UseArgs {
    /// Installed Java target or installation ID.
    target: String,
    /// Save the exact installation ID instead of its version requirement.
    #[arg(long)]
    pin: bool,
}

/// Executes SDK commands without embedding policy in argument parsing.
pub(super) fn run(command: SdkCommand) -> Result<i32> {
    let manager = SdkManager::user()?;
    match command {
        SdkCommand::Pin(args) => {
            manager.set_pin(&args.variant.request(&args.target)?, true)?;
            println!("Pinned {}", args.target);
        }
        SdkCommand::Unpin(args) => {
            manager.set_pin(&args.variant.request(&args.target)?, false)?;
            println!("Unpinned {}", args.target);
        }
        SdkCommand::Available(args) => {
            let request = args.variant.request(&args.target)?;
            let packages = manager.available(
                &request,
                &CatalogOptions {
                    offline: args.offline,
                    refresh: args.refresh,
                    ..Default::default()
                },
            )?;
            if args.json {
                json(&packages)?;
            } else {
                for package in packages {
                    println!(
                        "{}  {}  {}  {}",
                        package.version,
                        package.request.architecture,
                        package.request.kind,
                        package.filename
                    );
                }
            }
        }
        SdkCommand::Install(args) => {
            if args.path.is_some() && args.targets.len() != 1 {
                return Err(Error::InvalidInput(
                    "--path requires exactly one target".into(),
                ));
            }
            let mut installed = Vec::new();
            for target in args.targets {
                let request = args.variant.request(&target)?;
                if !args.json {
                    eprintln!(
                        "Installing {} ({}, {})",
                        request.target(),
                        request.architecture,
                        request.kind
                    );
                }
                let installation = if let Some(path) = &args.path {
                    manager.register(&request, path)?
                } else {
                    manager.install(
                        &request,
                        args.pin,
                        &CatalogOptions {
                            offline: args.offline,
                            timeout: std::time::Duration::from_secs(args.timeout),
                            progress: (!args.json).then_some(progress),
                            ..Default::default()
                        },
                    )?
                };
                if !args.json {
                    show(&manager, &installation)?;
                }
                installed.push(installation);
            }
            if args.json {
                json(&installed)?;
            }
        }
        SdkCommand::List(args) => {
            let status = manager.status()?;
            if args.json {
                json(&status)?;
            } else {
                for installed in status.installations {
                    show(&manager, &installed)?;
                }
            }
        }
        SdkCommand::Update(args) => {
            let requests = if args.all {
                manager
                    .selections()?
                    .into_iter()
                    .map(|s| s.request)
                    .collect()
            } else {
                args.targets
                    .iter()
                    .map(|t| args.variant.request(t))
                    .collect::<Result<Vec<_>>>()?
            };
            let mut installed = Vec::new();
            for request in requests {
                if !args.json {
                    eprintln!("Updating {}", request.target());
                }
                let installation = manager.update(
                    &request,
                    &CatalogOptions {
                        progress: (!args.json).then_some(progress),
                        timeout: std::time::Duration::from_secs(args.timeout),
                        ..Default::default()
                    },
                )?;
                if !args.json {
                    show(&manager, &installation)?;
                }
                installed.push(installation);
            }
            if args.json {
                json(&installed)?;
            }
        }
        SdkCommand::Uninstall(args) => {
            let removed = manager.uninstall(&args.target)?;
            println!("Uninstalled {} ({})", removed.java.target(), removed.id);
        }
        SdkCommand::Default(args) => {
            if args.clear {
                manager.clear_default()?;
                println!("Cleared the default Java selection");
            } else {
                show(
                    &manager,
                    &manager.set_default(args.target.as_deref().unwrap())?,
                )?;
            }
        }
        SdkCommand::Home(args) => {
            println!(
                "{}",
                manager.home(&manager.resolve(&args.target)?)?.display()
            );
        }
        SdkCommand::Current(args) => {
            let execution = manager.execution(None, Some(&std::env::current_dir()?))?;
            if args.json {
                json(&serde_json::json!({"java_home": execution.home()}))?;
            } else {
                println!("{}", execution.home().display());
            }
        }
        SdkCommand::Exec(args) => {
            let execution =
                manager.execution(args.java.as_deref(), Some(&std::env::current_dir()?))?;
            return execution
                .execute(&args.command[0], &args.command[1..])
                .map(super::exit_code);
        }
        SdkCommand::Env(args) => {
            let execution =
                manager.execution(args.java.as_deref(), Some(&std::env::current_dir()?))?;
            let shell = match args.shell {
                ShellArg::Sh => Shell::Sh,
                ShellArg::Powershell => Shell::PowerShell,
                ShellArg::Fish => Shell::Fish,
            };
            print!("{}", execution.environment(shell)?);
        }
        SdkCommand::Use(args) => {
            println!(
                "{}",
                manager
                    .use_project(&args.target, &std::env::current_dir()?, args.pin)?
                    .display()
            );
        }
    }
    Ok(0)
}

/// Displays an installation's exact target, ID, and home.
fn show(manager: &SdkManager, installed: &Installation) -> Result<()> {
    println!(
        "{}  {}  {}\n  {}\n  {}",
        installed.java.target(),
        installed.java.architecture,
        if installed.managed {
            "managed"
        } else {
            "external"
        },
        installed.id,
        manager.home(installed)?.display()
    );
    Ok(())
}

/// Reports archive transfer milestones without contaminating machine-readable stdout.
fn progress(bytes: u64, total: Option<u64>) {
    if bytes == 0 {
        eprintln!(
            "Downloading SDK archive ({} MiB)",
            total.unwrap_or(0).div_ceil(1024 * 1024)
        );
    } else if total == Some(bytes) || bytes.is_multiple_of(64 * 1024 * 1024) {
        eprintln!(
            "Downloaded {} / {} MiB",
            bytes.div_ceil(1024 * 1024),
            total.unwrap_or(0).div_ceil(1024 * 1024)
        );
    }
}

/// Emits one JSON document without human progress messages on stdout.
fn json(value: &impl serde::Serialize) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|e| Error::InvalidInput(e.to_string()))?
    );
    Ok(())
}

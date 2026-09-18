// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Shared CLI entry points for SDK and application management.

use clap::{Args, Subcommand, ValueEnum};
use janex_host::{
    Error, Result,
    app::{AppManager, AppRequest},
    dependency::DependencyOptions,
    sdk::{CatalogOptions, Installation, PRODUCTS, SdkManager, SdkRequest, Shell},
};
use std::{ffi::OsString, path::PathBuf};

/// SDK and application operations in the shared Janex command namespace.
#[derive(Subcommand)]
pub(super) enum SdkCommand {
    /// Write initialization scripts for all supported shells into JANEX_HOME.
    Init(InitArgs),
    /// Activate SDK selections in the initialized shell.
    Activate(ActivateArgs),
    /// Restore the activation-time environment, retaining shell integration.
    Deactivate(DeactivateArgs),
    /// List downloadable SDK versions without installing them.
    Available(AvailableArgs),
    /// Install SDKs or Maven applications while retaining all existing versions.
    Install(InstallArgs),
    /// List installed SDKs and applications with their persistent IDs.
    List(ListArgs),
    /// Update saved SDK and application requirements, retaining old versions and respecting pins.
    Update(UpdateArgs),
    /// Uninstall one exact SDK or application version.
    Uninstall(TargetArgs),
    /// Select or clear an SDK default or application command version.
    Default(DefaultArgs),
    /// Show SDK homes selected by the shell, project, or global defaults.
    Current(CurrentArgs),
    /// Print the directory for an installed SDK or application.
    Home(TargetArgs),
    /// Execute a command with selected SDK homes and tools on PATH.
    Exec(ExecArgs),
    /// Print shell environment assignments for an installed SDK.
    Env(EnvArgs),
    /// Select SDKs in the current shell, or save one project selection with --project.
    Use(UseArgs),
    /// Prevent updates from changing a saved requirement's selected build.
    Pin(PinArgs),
    /// Allow updates of a saved SDK or application requirement again.
    Unpin(PinArgs),
}

/// Pin policy for one saved requirement and platform variant.
#[derive(Args)]
pub(super) struct PinArgs {
    /// Saved SDK product, variant and version requirement.
    target: String,
}

/// Catalog listing request.
#[derive(Args)]
pub(super) struct AvailableArgs {
    /// sdk:product@version selector, or a tool family such as java to list its products.
    #[arg(default_value = "java")]
    target: String,
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
    /// sdk:product@version, a Maven PURL, or maven:group:artifact@version.
    /// JAR targets accept [main-class=...,dependencies=none] local options.
    #[arg(required = true)]
    targets: Vec<String>,
    /// Fix the selected build against subsequent update commands.
    #[arg(long)]
    pin: bool,
    /// Register an existing SDK home without copying or owning it; requires one target.
    #[arg(long, value_name = "SDK_HOME")]
    path: Option<PathBuf>,
    /// Reuse installed SDKs, cached applications, or local repositories without network access.
    #[arg(long)]
    offline: bool,
    /// Maximum seconds for an archive download, including segment retries.
    #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    /// Emit structured JSON.
    #[arg(long)]
    json: bool,
}

/// Installed SDK and application listing options.
#[derive(Args)]
pub(super) struct ListArgs {
    /// Emit structured JSON including saved requests, SDK defaults, and application commands.
    #[arg(long)]
    json: bool,
}

/// Saved requirement update request.
#[derive(Args)]
pub(super) struct UpdateArgs {
    /// Saved requirements to update; use --all for every request.
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    targets: Vec<String>,
    /// Update every saved SDK and application requirement.
    #[arg(long)]
    all: bool,
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
    /// Installed SDK or application selector, or an exact installation ID.
    target: String,
}

/// Global default selection request.
#[derive(Args)]
pub(super) struct DefaultArgs {
    /// Installed SDK or application selector, or an exact installation ID.
    #[arg(required_unless_present = "clear")]
    target: Option<String>,
    /// Clear the target's SDK default or application command; omitting the target clears native Java.
    #[arg(long)]
    clear: bool,
    /// SDK family whose default is cleared; setting a target infers its family.
    #[arg(long, value_parser = ["java", "gradle", "maven"], default_value = "java", requires = "clear", conflicts_with = "target")]
    family: String,
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
    /// Explicit installed Gradle target or installation ID.
    #[arg(long, value_name = "TARGET")]
    gradle: Option<String>,
    /// Explicit installed Maven target or installation ID.
    #[arg(long, value_name = "TARGET")]
    maven: Option<String>,
    /// Child executable and uninterpreted arguments.
    #[arg(required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<OsString>,
}

/// CLI shell names.
#[derive(Clone, Copy, ValueEnum)]
pub(super) enum ShellArg {
    /// sh, bash, or zsh.
    #[value(alias = "bash", alias = "zsh")]
    Sh,
    /// PowerShell.
    Powershell,
    /// Fish.
    Fish,
}

impl ShellArg {
    /// Returns the environment renderer used by this shell syntax.
    pub(super) fn shell(self) -> Shell {
        match self {
            Self::Sh => Shell::Sh,
            Self::Powershell => Shell::PowerShell,
            Self::Fish => Shell::Fish,
        }
    }
}

/// Shell initialization request.
#[derive(Args)]
pub(super) struct InitArgs {
    /// Internal rendering protocol used by the installed initialization scripts.
    #[arg(long, value_enum, hide = true)]
    shell: Option<ShellArg>,
}

/// Shell activation request, normally supplied by the shell function.
#[derive(Args)]
pub(super) struct ActivateArgs {
    /// Internal rendering protocol used by the shell function.
    #[arg(long, value_enum, hide = true)]
    shell: Option<ShellArg>,
}

/// Shell restoration request, normally supplied by the shell function.
#[derive(Args)]
pub(super) struct DeactivateArgs {
    /// Internal rendering protocol used by the shell function.
    #[arg(long, value_enum, hide = true)]
    shell: Option<ShellArg>,
}

/// Shell environment rendering options.
#[derive(Args)]
pub(super) struct EnvArgs {
    /// Explicit installed Java target or installation ID.
    #[arg(long, value_name = "TARGET")]
    java: Option<String>,
    /// Explicit installed Gradle target or installation ID.
    #[arg(long, value_name = "TARGET")]
    gradle: Option<String>,
    /// Explicit installed Maven target or installation ID.
    #[arg(long, value_name = "TARGET")]
    maven: Option<String>,
    /// Shell whose assignments should be emitted for evaluation by the caller.
    #[arg(long, value_enum)]
    shell: ShellArg,
}

/// Shell or project selection request.
#[derive(Args)]
pub(super) struct UseArgs {
    /// Installed SDK target or installation ID.
    targets: Vec<String>,
    /// Save one selection in the current project without changing the shell.
    #[arg(long)]
    project: bool,
    /// Save the exact installation ID instead of its version requirement.
    #[arg(long, requires = "project")]
    pin: bool,
    /// Internal rendering protocol used by the shell function.
    #[arg(long, value_enum, hide = true, conflicts_with = "project")]
    shell: Option<ShellArg>,
}

/// Executes SDK commands without embedding policy in argument parsing.
pub(super) fn run(command: SdkCommand) -> Result<i32> {
    if let SdkCommand::Init(args) = command {
        if let Some(shell) = args.shell {
            print!("{}", super::shell::init(shell.shell())?);
        } else {
            super::shell::install()?;
        }
        return Ok(0);
    }
    let manager = SdkManager::user()?;
    let applications = AppManager::user()?;
    match command {
        SdkCommand::Init(_) => unreachable!(),
        SdkCommand::Activate(args) => {
            let shell =
                args.shell
                    .ok_or_else(|| {
                        Error::InvalidInput(
                "run janex init and load the generated shell script before using activate".into(),
            )
                    })?
                    .shell();
            let environment =
                manager.shell_environment(&[], &std::env::current_dir()?, shell, false)?;
            print!("{environment}");
        }
        SdkCommand::Deactivate(args) => {
            let shell = args
                .shell
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "load Janex shell integration before using deactivate".into(),
                    )
                })?
                .shell();
            print!("{}", manager.shell_deactivate(shell)?);
        }
        SdkCommand::Pin(args) => {
            if app_target(&args.target) {
                applications.set_pin(&AppRequest::parse(&args.target)?, true)?;
            } else {
                manager.set_pin(&SdkRequest::parse(&args.target)?, true)?;
            }
            println!("Pinned {}", args.target);
        }
        SdkCommand::Unpin(args) => {
            if app_target(&args.target) {
                applications.set_pin(&AppRequest::parse(&args.target)?, false)?;
            } else {
                manager.set_pin(&SdkRequest::parse(&args.target)?, false)?;
            }
            println!("Unpinned {}", args.target);
        }
        SdkCommand::Available(args) => {
            if matches!(args.target.as_str(), "java" | "gradle" | "maven") {
                let products = PRODUCTS
                    .iter()
                    .filter(|p| p.family == args.target)
                    .collect::<Vec<_>>();
                if args.json {
                    json(&products)?;
                } else {
                    for product in products {
                        println!("sdk:{}  {}", product.id, product.variants.join(", "));
                    }
                }
                return Ok(0);
            }
            let request = SdkRequest::parse(&args.target)?;
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
                        "{}  {}  {}",
                        package.version,
                        package.request.target(),
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
            let requests = args
                .targets
                .iter()
                .map(|target| InstallRequest::parse(target))
                .collect::<Result<Vec<_>>>()?;
            if args.path.is_some() && requests.iter().any(|r| matches!(r, InstallRequest::App(_))) {
                return Err(Error::InvalidInput("--path registers SDK homes; use a file repository qualifier for local applications".into()));
            }
            for request in requests {
                let request = match request {
                    InstallRequest::Sdk(request) => request,
                    InstallRequest::App(request) => {
                        if !args.json {
                            eprintln!("Installing {}", request.target());
                        }
                        let application = applications.install(
                            &request,
                            args.pin,
                            &DependencyOptions {
                                offline: args.offline,
                                timeout: std::time::Duration::from_secs(args.timeout),
                                ..Default::default()
                            },
                            &std::env::current_exe()?,
                        )?;
                        if !args.json {
                            show_application(&applications, &application)?;
                        }
                        installed.push(value(&application)?);
                        continue;
                    }
                };
                if !args.json {
                    eprintln!("Installing {}", request.target());
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
                installed.push(value(&installation)?);
            }
            if args.json {
                json(&installed)?;
            }
        }
        SdkCommand::List(args) => {
            let status = manager.status()?;
            let app_status = applications.status()?;
            if args.json {
                let mut status = value(&status)?;
                status["applications"] = value(&app_status)?;
                json(&status)?;
            } else {
                for installed in status.installations {
                    show(&manager, &installed)?;
                }
                for installed in app_status.installations {
                    show_application(&applications, &installed)?;
                }
            }
        }
        SdkCommand::Update(args) => {
            let requests = if args.all {
                let mut requests = manager
                    .selections()?
                    .into_iter()
                    .map(|s| InstallRequest::Sdk(s.request))
                    .collect::<Vec<_>>();
                requests.extend(
                    applications
                        .status()?
                        .selections
                        .into_iter()
                        .map(|s| InstallRequest::App(s.request)),
                );
                requests
            } else {
                args.targets
                    .iter()
                    .map(|t| InstallRequest::parse(t))
                    .collect::<Result<Vec<_>>>()?
            };
            let mut installed = Vec::new();
            for request in requests {
                let request = match request {
                    InstallRequest::Sdk(request) => request,
                    InstallRequest::App(request) => {
                        if !args.json {
                            eprintln!("Updating {}", request.target());
                        }
                        let application = applications.update(
                            &request,
                            &DependencyOptions {
                                timeout: std::time::Duration::from_secs(args.timeout),
                                ..Default::default()
                            },
                            &std::env::current_exe()?,
                        )?;
                        if !args.json {
                            show_application(&applications, &application)?;
                        }
                        installed.push(value(&application)?);
                        continue;
                    }
                };
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
                installed.push(value(&installation)?);
            }
            if args.json {
                json(&installed)?;
            }
        }
        SdkCommand::Uninstall(args) => {
            if app_target(&args.target) {
                let removed = applications.uninstall(&args.target)?;
                println!(
                    "Uninstalled {} ({})",
                    removed.application.target(),
                    removed.id
                );
                return Ok(0);
            }
            let removed = manager.uninstall(&args.target)?;
            println!("Uninstalled {} ({})", removed.sdk.target(), removed.id);
        }
        SdkCommand::Default(args) => {
            if let Some(target) = &args.target
                && app_target(target)
            {
                if args.clear {
                    applications.clear_default(target)?;
                    println!("Cleared the application command selection");
                } else {
                    show_application(
                        &applications,
                        &applications.set_default(target, &std::env::current_exe()?)?,
                    )?;
                }
                return Ok(0);
            }
            if args.clear {
                let family = if let Some(target) = &args.target {
                    let request = manager.resolve(target)?.sdk;
                    if let Some(platform) = &request.platform {
                        manager.clear_default_for(request.family(), platform)?;
                    } else {
                        manager.clear_default(request.family())?;
                    }
                    request.family()
                } else {
                    manager.clear_default(&args.family)?;
                    &args.family
                };
                println!("Cleared the default {family} selection");
            } else {
                show(
                    &manager,
                    &manager.set_default(args.target.as_deref().unwrap())?,
                )?;
            }
        }
        SdkCommand::Home(args) => {
            if app_target(&args.target) {
                println!("{}", applications.home(&args.target)?.display());
                return Ok(0);
            }
            println!(
                "{}",
                manager.home(&manager.resolve(&args.target)?)?.display()
            );
        }
        SdkCommand::Current(args) => {
            let execution = manager.execution(None, Some(&std::env::current_dir()?))?;
            if args.json {
                json(
                    &serde_json::json!({"java_home": execution.home(), "homes": execution.homes()}),
                )?;
            } else {
                for (family, home) in execution.homes() {
                    println!("{family}  {}", home.display());
                }
            }
        }
        SdkCommand::Exec(args) => {
            let execution = manager.execution_with(
                args.java.as_deref(),
                args.gradle.as_deref(),
                args.maven.as_deref(),
                Some(&std::env::current_dir()?),
            )?;
            return execution
                .execute(&args.command[0], &args.command[1..])
                .map(super::exit_code);
        }
        SdkCommand::Env(args) => {
            let execution = manager.execution_with(
                args.java.as_deref(),
                args.gradle.as_deref(),
                args.maven.as_deref(),
                Some(&std::env::current_dir()?),
            )?;
            let shell = match args.shell {
                ShellArg::Sh => Shell::Sh,
                ShellArg::Powershell => Shell::PowerShell,
                ShellArg::Fish => Shell::Fish,
            };
            print!("{}", execution.environment(shell)?);
        }
        SdkCommand::Use(args) => {
            if args.project {
                if args.targets.len() != 1 {
                    return Err(Error::InvalidInput(
                        "use --project requires exactly one target".into(),
                    ));
                }
                println!(
                    "{}",
                    manager
                        .use_project(&args.targets[0], &std::env::current_dir()?, args.pin)?
                        .display()
                );
            } else {
                let shell = args.shell.ok_or_else(|| Error::InvalidInput("load Janex shell integration before using use; use --project to save a project selection".into()))?.shell();
                print!(
                    "{}",
                    manager.shell_environment(
                        &args.targets,
                        &std::env::current_dir()?,
                        shell,
                        args.targets.is_empty()
                    )?
                );
            }
        }
    }
    Ok(0)
}

/// Displays an installation's exact target, ID, and home.
fn show(manager: &SdkManager, installed: &Installation) -> Result<()> {
    println!(
        "{}  {}  {}\n  {}\n  {}",
        installed.sdk.target(),
        installed
            .sdk
            .platform
            .as_ref()
            .map(|platform| platform.key())
            .unwrap_or_else(|| "portable".into()),
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

/// A fully parsed target; all targets are validated before any installation begins.
enum InstallRequest {
    /// A runtime or toolchain product.
    Sdk(SdkRequest),
    /// A Maven application with independent repository and command qualifiers.
    App(AppRequest),
}

impl InstallRequest {
    /// Selects the resolver from an explicit PURL, maven: or sdk: prefix.
    fn parse(target: &str) -> Result<Self> {
        if AppRequest::recognizes(target) {
            AppRequest::parse(target).map(Self::App)
        } else {
            SdkRequest::parse(target).map(Self::Sdk)
        }
    }
}

/// Recognizes Maven application selectors and exact application installation IDs.
fn app_target(target: &str) -> bool {
    AppRequest::recognizes(target) || target.starts_with("app-")
}

/// Displays the original coordinates, command name, and persistent installation directory.
fn show_application(manager: &AppManager, installed: &janex_host::app::Installation) -> Result<()> {
    println!(
        "{}  command={}\n  {}\n  {}",
        installed.application.target(),
        installed.application.command,
        installed.id,
        manager.home(&installed.id)?.display()
    );
    Ok(())
}

/// Converts either installation kind without changing SDK JSON record fields.
fn value(input: &impl serde::Serialize) -> Result<serde_json::Value> {
    serde_json::to_value(input).map_err(|e| Error::InvalidInput(e.to_string()))
}

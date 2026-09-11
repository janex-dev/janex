// Copyright (c) 2025 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Janex command-line entry point.

use clap::{Args, Parser, Subcommand};
use janex_core::pack::{PackOptions, pack};
use janex_core::{
    java::JavaOptions,
    run::{RunOptions, prepare},
};
use std::{ffi::OsString, path::PathBuf, process::ExitStatus};

/// Local Janex packaging and execution commands.
#[derive(Parser)]
#[command(
    name = "janex",
    version,
    about = "Package and launch Java applications with Janex"
)]
struct Cli {
    /// Operation to perform.
    #[command(subcommand)]
    command: Command,
}

/// Supported command implementations.
#[derive(Subcommand)]
enum Command {
    /// Package a directory or JAR as a Janex application.
    Pack(PackArgs),
    /// Run an application from a local Janex file.
    Run(RunArgs),
}

/// Runtime selection and local execution policy, followed by uninterpreted application arguments.
#[derive(Args)]
#[command(override_usage = "janex run [OPTIONS] <TARGET> [ARGS...]")]
struct RunArgs {
    /// Select an application ID; otherwise the package must contain exactly one application.
    #[arg(long, value_name = "ID")]
    application: Option<String>,
    /// Explicit Java executable, disabling runtime fallback.
    #[arg(long, value_name = "PATH", conflicts_with = "java_home")]
    java: Option<PathBuf>,
    /// Explicit Java home, disabling runtime fallback.
    #[arg(long, value_name = "PATH")]
    java_home: Option<PathBuf>,
    /// Permit local None or Checksum inputs; signed input still requires authentication.
    #[arg(long)]
    allow_unsigned: bool,
    /// Local path or file URI, then program arguments forwarded without Janex option parsing.
    #[arg(value_name = "TARGET", required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
    target: Vec<OsString>,
}

/// Local packaging inputs and Java launch arguments.
#[derive(Args)]
struct PackArgs {
    /// Primary directory or JAR.
    source: PathBuf,
    /// Destination file; existing files are never replaced.
    #[arg(long, value_name = "FILE")]
    output: PathBuf,
    /// Append a local directory or JAR to the classpath.
    #[arg(long, value_name = "PATH")]
    class_path: Vec<PathBuf>,
    /// Append a local directory or JAR to the module path.
    #[arg(long, value_name = "PATH")]
    module_path: Vec<PathBuf>,
    /// Binary main-class name, overriding entry-point inference.
    #[arg(long, value_name = "NAME")]
    main_class: Option<String>,
    /// Main module; place the primary input on the module path.
    #[arg(long, value_name = "NAME")]
    main_module: Option<String>,
    /// Application ID within the package.
    #[arg(long, value_name = "ID", default_value = "main")]
    application: String,
    /// Append one complete JVM argument, without shell splitting.
    #[arg(long = "jvm-option", value_name = "ARG", allow_hyphen_values = true)]
    jvm_options: Vec<String>,
    /// Append one preset program argument, including an empty argument.
    #[arg(long = "argument", value_name = "ARG", allow_hyphen_values = true)]
    arguments: Vec<String>,
    /// Required Java version range, using vers:jep322 syntax.
    #[arg(long, value_name = "VERS")]
    java_version: Option<String>,
}

/// Parses arguments, performs the requested operation, and reports service failures.
fn main() {
    let code = match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    };
    std::process::exit(code);
}

/// Executes a parsed command without interpreting argument contents as shell text.
fn run(cli: Cli) -> janex_core::Result<i32> {
    match cli.command {
        Command::Pack(args) => {
            let mut options = PackOptions::new(args.source, args.output);
            options.class_path = args.class_path;
            options.module_path = args.module_path;
            options.main_class = args.main_class;
            options.main_module = args.main_module;
            options.application = args.application;
            options.jvm_options = args.jvm_options;
            options.arguments = args.arguments;
            options.java_version = args.java_version;
            let report = pack(&options)?;
            println!(
                "Packed {} ({} bytes, {} resource roots)",
                options.output.display(),
                report.file_bytes,
                report.resource_roots
            );
        }
        Command::Run(args) => {
            let mut target = args.target.into_iter();
            let mut options =
                RunOptions::new(PathBuf::from(target.next().expect("required target")));
            options.application = args.application;
            options.java = JavaOptions {
                java: args.java,
                java_home: args.java_home,
            };
            options.allow_unsigned = args.allow_unsigned;
            options.arguments = target.collect();
            return prepare(&options)?.execute().map(exit_code);
        }
    }
    Ok(0)
}

/// Preserves native exit codes and maps Unix signals to the conventional shell status.
fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    //! Command parsing must stop interpreting options at the local target.
    use super::*;

    #[test]
    fn run_forwards_every_argument_after_the_target() {
        let cli = Cli::try_parse_from([
            "janex",
            "run",
            "--allow-unsigned",
            "--application",
            "main",
            "app.janex",
            "--java",
            "fake",
            "--help",
            "--",
            "",
            "@args",
            "two words",
        ])
        .unwrap();
        let Command::Run(args) = cli.command else {
            panic!("expected run")
        };
        assert!(args.allow_unsigned);
        assert_eq!(args.application.as_deref(), Some("main"));
        assert!(args.java.is_none());
        assert_eq!(
            args.target,
            [
                "app.janex",
                "--java",
                "fake",
                "--help",
                "--",
                "",
                "@args",
                "two words"
            ]
            .map(OsString::from)
        );
        assert!(
            Cli::try_parse_from([
                "janex",
                "run",
                "--java",
                "java",
                "--java-home",
                "jdk",
                "app.janex"
            ])
            .is_err()
        );
    }
}

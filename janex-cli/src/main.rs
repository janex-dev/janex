// Copyright (c) 2025 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Janex command-line entry point.

mod inspect;
mod sdk;
mod shell;

use clap::{Args, Parser, Subcommand, ValueEnum};
use janex_host::pack::{PackOptions, PackSigner, pack};
use janex_host::{
    authentication::{self, CmsAlgorithm, MATERIAL_LIMITS, OpenPgpAlgorithm},
    run::{LaunchMode, RunOptions, prepare},
};
use janex_java::runtime::JavaOptions;
use std::{
    ffi::OsString,
    io::IsTerminal,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
};
use zeroize::Zeroizing;

/// Allocates Rust and native dependency memory with mimalloc in Linux musl builds.
#[cfg(all(target_os = "linux", target_env = "musl"))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Local Janex packaging and execution commands.
#[derive(Parser)]
#[command(
    name = "janex",
    version,
    about = "Manage SDKs, package and launch applications with Janex"
)]
struct Cli {
    /// Operation to perform.
    #[command(subcommand)]
    command: Command,
}

/// Supported command implementations.
#[derive(Subcommand)]
enum Command {
    /// Inspect local container structure without launching or acquiring dependencies.
    Inspect(inspect::InspectArgs),
    /// Manage installed SDK versions and execution environments.
    #[command(flatten)]
    Sdk(sdk::SdkCommand),
    /// Package a directory or JAR as a Janex application.
    Pack(Box<PackArgs>),
    /// Run an application from a local Janex file.
    Run(Box<RunArgs>),
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
    /// Preserve Unicode arguments through a bootstrap, or use the native Java entry point.
    #[arg(long, value_enum, default_value = "bootstrap")]
    launch_mode: LaunchModeArg,
    /// Resolve external dependencies only from the verified local cache.
    #[arg(long, conflicts_with = "refresh_dependencies")]
    offline: bool,
    /// Store dependency cache entries in this directory.
    #[arg(long, value_name = "DIRECTORY")]
    dependency_cache: Option<PathBuf>,
    /// Refresh dependencies even when cache entries are valid.
    #[arg(long)]
    refresh_dependencies: bool,
    /// Override Maven Central for PURLs without a repository_url qualifier.
    #[arg(long, value_name = "URL")]
    maven_repository: Option<String>,
    /// Permit local None or Checksum inputs; signed input still requires authentication.
    #[arg(long)]
    allow_unsigned: bool,
    /// Require this CMS signer certificate; repeat to require every listed signer.
    #[arg(long, value_name = "FILE", conflicts_with = "trust_openpgp_key")]
    trust_cms_certificate: Vec<PathBuf>,
    /// Trust this OpenPGP primary key and its valid signing subkeys, using supplied revocation data.
    #[arg(long, value_name = "FILE")]
    trust_openpgp_key: Option<PathBuf>,
    /// Supply an issuer certificate for offline CRL authentication, without adding a signer pin.
    #[arg(long, value_name = "FILE", requires = "trust_cms_certificate")]
    cms_issuer: Vec<PathBuf>,
    /// Supply a complete direct X.509 v2 revocation list.
    #[arg(long, value_name = "FILE", requires = "trust_cms_certificate")]
    cms_crl: Vec<PathBuf>,
    /// Local path or file URI, then program arguments forwarded without Janex option parsing.
    #[arg(value_name = "TARGET", required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
    target: Vec<OsString>,
}

/// CLI choices for application entry-point invocation.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum LaunchModeArg {
    /// Restore Unicode program arguments in a Java 8-compatible entry layer.
    Bootstrap,
    /// Use the runtime's native entry point and argument conversion.
    Direct,
}

/// Local packaging inputs and Java launch arguments.
#[derive(Args)]
struct PackArgs {
    /// Include a launcher for java -jar execution.
    #[arg(long)]
    with_launcher: bool,
    /// Prepend a native Janex launcher executable for direct execution.
    #[arg(long, value_name = "FILE")]
    native_launcher: Option<PathBuf>,
    /// Select how the native launcher invokes Java.
    #[arg(long, value_enum, requires = "native_launcher")]
    native_launch_mode: Option<LaunchModeArg>,
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
    /// Append an external classpath JAR; CHECKSUM is algorithm:hex or none.
    #[arg(long, value_names = ["URI", "CHECKSUM"], num_args = 2, action = clap::ArgAction::Append)]
    external_class_path: Vec<String>,
    /// Append an external module JAR or virtual module requirement, with a checksum or none.
    #[arg(long, value_names = ["URI", "CHECKSUM"], num_args = 2, action = clap::ArgAction::Append)]
    external_module_path: Vec<String>,
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
    /// Sign with this CMS certificate and its matching PKCS#8 private key.
    #[arg(long, value_name = "FILE", requires = "cms_key")]
    cms_certificate: Option<PathBuf>,
    /// Matching DER or PEM PKCS#8 private key, optionally encrypted.
    #[arg(
        long,
        value_name = "FILE",
        requires = "cms_certificate",
        group = "signing_key"
    )]
    cms_key: Option<PathBuf>,
    /// Select a CMS signing combination; otherwise infer it from the certificate.
    #[arg(long, value_enum, requires = "cms_certificate")]
    cms_algorithm: Option<CmsAlgorithmArg>,
    /// Sign with a binary or armored transferable OpenPGP secret key.
    #[arg(long, value_name = "FILE", group = "signing_key")]
    openpgp_key: Option<PathBuf>,
    /// Select a primary key or signing subkey by its complete hexadecimal fingerprint.
    #[arg(long, value_name = "FINGERPRINT", requires = "openpgp_key")]
    openpgp_signing_key: Option<String>,
    /// Select an OpenPGP signing combination; otherwise infer it from the selected key.
    #[arg(long, value_enum, requires = "openpgp_key")]
    openpgp_algorithm: Option<OpenPgpAlgorithmArg>,
    /// Read an encrypted key's password from a file instead of a hidden terminal prompt.
    #[arg(long, value_name = "FILE", requires = "signing_key")]
    key_password_file: Option<PathBuf>,
}

/// CLI names for supported OpenPGP public-key and digest combinations.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum OpenPgpAlgorithmArg {
    /// RSA with SHA-256.
    RsaSha256,
    /// RSA with SHA-512.
    RsaSha512,
    /// NIST P-256 ECDSA with SHA-256.
    EcdsaP256Sha256,
    /// NIST P-384 ECDSA with SHA-384.
    EcdsaP384Sha384,
    /// Ed25519 with SHA-256.
    Ed25519Sha256,
    /// Ed25519 with SHA-512.
    Ed25519Sha512,
}

impl From<OpenPgpAlgorithmArg> for OpenPgpAlgorithm {
    /// Maps a parsed CLI choice to the format mechanism.
    fn from(value: OpenPgpAlgorithmArg) -> Self {
        match value {
            OpenPgpAlgorithmArg::RsaSha256 => Self::RsaSha256,
            OpenPgpAlgorithmArg::RsaSha512 => Self::RsaSha512,
            OpenPgpAlgorithmArg::EcdsaP256Sha256 => Self::EcdsaP256Sha256,
            OpenPgpAlgorithmArg::EcdsaP384Sha384 => Self::EcdsaP384Sha384,
            OpenPgpAlgorithmArg::Ed25519Sha256 => Self::Ed25519Sha256,
            OpenPgpAlgorithmArg::Ed25519Sha512 => Self::Ed25519Sha512,
        }
    }
}

/// CLI names for the supported CMS digest and signature combinations.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum CmsAlgorithmArg {
    /// RSA PKCS#1 v1.5 with SHA-256.
    RsaSha256,
    /// RSA PKCS#1 v1.5 with SHA-512.
    RsaSha512,
    /// NIST P-256 ECDSA with SHA-256.
    EcdsaP256Sha256,
    /// NIST P-384 ECDSA with SHA-384.
    EcdsaP384Sha384,
}

impl From<CmsAlgorithmArg> for CmsAlgorithm {
    /// Maps a parsed CLI choice to the format mechanism.
    fn from(value: CmsAlgorithmArg) -> Self {
        match value {
            CmsAlgorithmArg::RsaSha256 => Self::RsaSha256,
            CmsAlgorithmArg::RsaSha512 => Self::RsaSha512,
            CmsAlgorithmArg::EcdsaP256Sha256 => Self::EcdsaP256Sha256,
            CmsAlgorithmArg::EcdsaP384Sha384 => Self::EcdsaP384Sha384,
        }
    }
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
fn run(cli: Cli) -> janex_host::Result<i32> {
    match cli.command {
        Command::Inspect(args) => return inspect::run(args),
        Command::Sdk(command) => return sdk::run(command),
        Command::Pack(args) => {
            let args = *args;
            let mut options = PackOptions::new(args.source, args.output);
            options.with_launcher = args.with_launcher;
            options.native_launcher = args.native_launcher;
            if let Some(mode) = args.native_launch_mode {
                options.native_launch_mode = match mode {
                    LaunchModeArg::Bootstrap => LaunchMode::Bootstrap,
                    LaunchModeArg::Direct => LaunchMode::Direct,
                };
            }
            options.class_path = args.class_path;
            options.module_path = args.module_path;
            options.external_class_path = external_entries(&args.external_class_path, false)?;
            options.external_module_path = external_entries(&args.external_module_path, true)?;
            options.main_class = args.main_class;
            options.main_module = args.main_module;
            options.application = args.application;
            options.jvm_options = args.jvm_options;
            options.arguments = args.arguments;
            options.java_version = args.java_version;
            if let Some(certificate) = args.cms_certificate {
                let signer = authentication::load_cms_signer(
                    &certificate,
                    args.cms_key.as_deref().expect("required CMS key"),
                    args.cms_algorithm.map(Into::into),
                    MATERIAL_LIMITS,
                    || key_password(args.key_password_file.as_deref()),
                )?;
                options.signer = Some(PackSigner::Cms(Arc::new(signer)));
            } else if let Some(key) = args.openpgp_key {
                let signer = authentication::load_openpgp_signer(
                    &key,
                    args.openpgp_signing_key.as_deref(),
                    args.openpgp_algorithm.map(Into::into),
                    MATERIAL_LIMITS,
                    || key_password(args.key_password_file.as_deref()),
                )?;
                options.signer = Some(PackSigner::OpenPgp(Arc::new(signer)));
            }
            let report = pack(&options)?;
            println!(
                "Packed {} ({} bytes, {} resource roots)",
                options.output.display(),
                report.file_bytes,
                report.resource_roots
            );
        }
        Command::Run(args) => {
            let args = *args;
            let mut target = args.target.into_iter();
            let mut options =
                RunOptions::new(PathBuf::from(target.next().expect("required target")));
            options.application = args.application;
            options.java = JavaOptions {
                java: args.java,
                java_home: args.java_home,
            };
            options.launch_mode = match args.launch_mode {
                LaunchModeArg::Bootstrap => LaunchMode::Bootstrap,
                LaunchModeArg::Direct => LaunchMode::Direct,
            };
            options.allow_unsigned = args.allow_unsigned;
            options.dependencies.offline = args.offline;
            options.dependencies.refresh = args.refresh_dependencies;
            options.dependencies.cache_directory = args.dependency_cache;
            if let Some(repository) = args.maven_repository {
                options.dependencies.maven_repository = repository;
            }
            options.openpgp_trust = args
                .trust_openpgp_key
                .as_deref()
                .map(|path| authentication::load_openpgp_certificate(path, MATERIAL_LIMITS))
                .transpose()?;
            options.cms_trust.signers = args
                .trust_cms_certificate
                .iter()
                .map(|path| authentication::load_certificate(path, MATERIAL_LIMITS))
                .collect::<janex_host::Result<_>>()?;
            options.cms_trust.issuers = args
                .cms_issuer
                .iter()
                .map(|path| authentication::load_certificate(path, MATERIAL_LIMITS))
                .collect::<janex_host::Result<_>>()?;
            options.cms_trust.revocation_lists = args
                .cms_crl
                .iter()
                .map(|path| authentication::load_revocation_list(path, MATERIAL_LIMITS))
                .collect::<janex_host::Result<_>>()?;
            options.arguments = target.collect();
            return prepare(&options)?.execute().map(exit_code);
        }
    }
    Ok(0)
}

/// Parses external declarations without fetching dependencies while packing.
fn external_entries(
    values: &[String],
    module_path: bool,
) -> janex_host::Result<Vec<janex_format::application::PathEntry>> {
    use janex_format::{
        application::PathEntry,
        checksum::{Algorithm, Checksum},
    };
    values
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let checksum = if pair[1] == "none" {
                None
            } else {
                let (algorithm, hex) = pair[1].split_once(':').ok_or_else(|| {
                    janex_host::Error::InvalidInput("checksum must be algorithm:hex or none".into())
                })?;
                let algorithm = match algorithm {
                    "xxh3-64" => Algorithm::Xxh3_64,
                    "xxh3-128" => Algorithm::Xxh3_128,
                    "sha256" => Algorithm::Sha256,
                    "sha512" => Algorithm::Sha512,
                    "sm3" => Algorithm::Sm3,
                    _ => {
                        return Err(janex_host::Error::InvalidInput(
                            "unknown dependency checksum algorithm".into(),
                        ));
                    }
                };
                if hex.len() != algorithm.digest_length() * 2
                    || !hex.bytes().all(|value| value.is_ascii_hexdigit())
                {
                    return Err(janex_host::Error::InvalidInput(
                        "invalid dependency checksum hex or length".into(),
                    ));
                }
                let mut bytes = vec![algorithm as u8];
                for offset in (0..hex.len()).step_by(2) {
                    bytes.push(
                        u8::from_str_radix(&hex[offset..offset + 2], 16)
                            .expect("validated hexadecimal"),
                    );
                }
                Some(Checksum::decode(&bytes)?)
            };
            let entry = PathEntry::External {
                uri: pair[0].clone(),
                checksum,
            };
            entry.to_value(module_path)?;
            Ok(entry)
        })
        .collect()
}

/// Obtains a password without exposing it as a command-line value or echoing it to the terminal.
fn key_password(path: Option<&Path>) -> janex_host::Result<Zeroizing<Vec<u8>>> {
    if let Some(path) = path {
        let mut bytes = authentication::read_material(path, 65_536)?;
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        }
        return Ok(bytes);
    }
    if !std::io::stdin().is_terminal() {
        return Err(janex_host::Error::InvalidInput(
            "encrypted keys require --key-password-file when standard input is not a terminal"
                .into(),
        ));
    }
    Ok(Zeroizing::new(
        rpassword::prompt_password("Private key password: ")?.into_bytes(),
    ))
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

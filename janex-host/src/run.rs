// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Local snapshot verification, runtime selection, and Java process execution.

use crate::adapters::{java_limits, runtime_context, signature_limits};
use crate::{
    Error, Result,
    authentication::{Authentication, CmsTrust, OpenPgpCertificate},
    error::invalid,
    materialize::materialize_tree,
    roots::{RootKey, Roots},
};
use janex_format::{
    application::{Application, PathEntry, read_applications, select_application},
    binary::Limits,
    blob::BlobStore,
    condition::Context,
    container::{IntegrityReport, Reader, Verification},
};
pub use janex_java::launch::LaunchMode;
use janex_java::{
    launch::{EntryPoint, LaunchRequest},
    runtime::{JavaOptions, JavaRuntime},
};
use janex_signature::{cms::CmsSignature, openpgp::OpenPgpSignature};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::SystemTime,
};
use tempfile::TempDir;

/// Local input, runtime overrides, and per-launch resource limits.
#[derive(Clone, Debug)]
pub struct RunOptions {
    /// Native local path or local `file:` URI; downloading the target itself is unsupported.
    pub target: PathBuf,
    /// Explicit application ID; absent selects the sole application.
    pub application: Option<String>,
    /// Condition channel, normally `run`, `open`, or `command`.
    pub invocation: String,
    /// Java executable or home override, otherwise environment, managed SDK, and PATH candidates.
    pub java: JavaOptions,
    /// Entry-point invocation strategy; defaults to lossless bootstrap argument transport.
    pub launch_mode: LaunchMode,
    /// HTTP(S), Maven repository, cache, and offline policy for external dependencies.
    pub dependencies: crate::dependency::DependencyOptions,
    /// Arguments appended after the descriptor's preset program arguments.
    pub arguments: Vec<OsString>,
    /// Explicit permission to execute None or Checksum verification types.
    ///
    /// This never permits a signed file to bypass signature authentication.
    pub allow_unsigned: bool,
    /// Explicit CMS signer pins and revocation material. Pins also reject unsigned inputs.
    pub cms_trust: CmsTrust,
    /// One pinned OpenPGP primary key and its supplied bindings and revocations.
    ///
    /// Rejects unsigned or CMS inputs. Cannot be combined with CMS signer pins.
    pub openpgp_trust: Option<OpenPgpCertificate>,
    /// Limits on individual format values and resource expansion.
    pub limits: Limits,
    /// Maximum complete input snapshot size in bytes, including external regions.
    pub max_snapshot_bytes: u64,
    /// Maximum aggregate logical resource bytes prepared for classpath, module paths, and agents.
    ///
    /// Indexed classpath files count by their declared restored size, even when never read.
    pub max_materialized_bytes: u64,
}

impl RunOptions {
    /// Creates options requiring authenticated input, with 512 MiB aggregate limits.
    pub fn new(target: impl Into<PathBuf>) -> Self {
        Self {
            target: target.into(),
            application: None,
            invocation: "run".into(),
            java: JavaOptions::default(),
            launch_mode: LaunchMode::default(),
            dependencies: crate::dependency::DependencyOptions::default(),
            arguments: Vec::new(),
            allow_unsigned: false,
            cms_trust: CmsTrust::default(),
            openpgp_trust: None,
            limits: Limits::default(),
            max_snapshot_bytes: 512 * 1024 * 1024,
            max_materialized_bytes: 512 * 1024 * 1024,
        }
    }
}

/// A selected Java invocation borrowing input files and owning any materialized paths.
///
/// Preparation verifies owned input bytes. Bootstrap reads the original files without repeating
/// whole-file verification. The caller must keep the package and dependency cache files unchanged
/// until Java exits.
#[derive(Debug)]
pub struct ExecutionPlan {
    /// Runtime selected after condition and local-module checks.
    runtime: JavaRuntime,
    /// Entry-point strategy selected by the caller.
    launch_mode: LaunchMode,
    /// Complete ordered process arguments, without shell encoding.
    arguments: Vec<OsString>,
    /// Private environment chunks used only when launch data exceeds the compact argument budget.
    environment: Vec<(OsString, OsString)>,
    /// Full-snapshot checksum result retained for inspection.
    integrity: IntegrityReport,
    /// Publisher authentication outcome for the bytes checked during preparation.
    authentication: Authentication,
    /// Whether the application requests a windowless Windows process.
    windowed: bool,
    /// Lifetime owner of every generated Java path entry.
    directory: Option<TempDir>,
    /// Prevents uninstall of the selected managed SDK while this plan exists.
    sdk_lease: Option<fs::File>,
}

impl ExecutionPlan {
    /// Returns the selected runtime and its probed properties.
    pub fn runtime(&self) -> &JavaRuntime {
        &self.runtime
    }

    /// Returns the selected entry-point invocation strategy.
    pub fn launch_mode(&self) -> LaunchMode {
        self.launch_mode
    }

    /// Returns the actual Java process arguments in order.
    ///
    /// Bootstrap launches carry encoded program arguments in a private property or environment chunks.
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// Returns the snapshot's checksum coverage, independently of publisher authentication.
    pub fn integrity(&self) -> IntegrityReport {
        self.integrity
    }

    /// Returns publisher authentication independently of content checksum coverage.
    pub fn authentication(&self) -> &Authentication {
        &self.authentication
    }

    /// Returns the generated-file directory, or None when launching needs no temporary files.
    pub fn directory(&self) -> Option<&Path> {
        self.directory.as_ref().map(TempDir::path)
    }

    /// Creates a direct Java command inheriting the current environment, directory, and streams.
    ///
    /// The caller may customize its I/O. Keep this plan alive until the child process exits;
    /// dropping it removes files that the child may still need to load and releases its SDK lease.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.runtime.executable);
        command
            .args(&self.arguments)
            .envs(self.environment.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        if self.windowed {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command
    }

    /// Returns whether the descriptor requests windowed launching.
    pub fn windowed(&self) -> bool {
        self.windowed
    }

    /// Runs Java and waits for its exit, then releases the temporary files.
    ///
    /// Child exit failures are returned as statuses; process creation and wait failures are errors.
    pub fn execute(self) -> Result<ExitStatus> {
        Ok(self.command().status()?)
    }
}

/// Verifies a local snapshot, resolves explicit dependencies, and prepares a compatible Java invocation.
///
/// Explicit runtime selection disables fallback. None and Checksum inputs require
/// `allow_unsigned`; signed inputs require signature support and never fall back to that policy.
/// No application main method or descriptor-supplied agent runs during preparation.
/// Module identities and requirements are checked here; bootstrap resource expansion, graph resolution, and module access
/// validation occur in the launched JVM before application agents or main; failures become child exit statuses.
/// Dependency acquisition follows `options.dependencies` after authentication and condition evaluation.
pub fn prepare(options: &RunOptions) -> Result<ExecutionPlan> {
    prepare_snapshot(
        options,
        snapshot(&target_path(&options.target)?, options.max_snapshot_bytes)?,
    )
}

/// Prepares a launch from an owned snapshot, retaining the same authentication policy as [`prepare`].
/// In bootstrap mode, the caller must ensure `options.target` contains these bytes and retain it
/// unchanged until Java exits. The snapshot size limit is enforced.
pub fn prepare_snapshot(options: &RunOptions, bytes: Vec<u8>) -> Result<ExecutionPlan> {
    if options.openpgp_trust.is_some() && !options.cms_trust.signers.is_empty() {
        return Err(invalid(
            "OpenPGP and CMS signer pins are mutually exclusive",
        ));
    }
    if bytes.len() as u64 > options.max_snapshot_bytes {
        return Err(invalid("input snapshot exceeds the byte limit"));
    }
    let mut reader = Reader::open_auto(Cursor::new(bytes), options.limits)?;
    let authentication = match reader.verification() {
        Verification::None | Verification::Checksum(_)
            if options.allow_unsigned
                && options.cms_trust.signers.is_empty()
                && options.openpgp_trust.is_none() =>
        {
            Authentication::Unsigned
        }
        Verification::None | Verification::Checksum(_)
            if !options.cms_trust.signers.is_empty() || options.openpgp_trust.is_some() =>
        {
            return Err(Error::Trust(
                "signer pins require a file authenticated by the selected signature format".into(),
            ));
        }
        Verification::None | Verification::Checksum(_) => {
            return Err(invalid(
                "unsigned local execution requires --allow-unsigned",
            ));
        }
        Verification::OpenPgp(payload) => {
            let trust = options.openpgp_trust.as_ref().ok_or_else(|| {
                Error::Trust(
                    "OpenPGP authentication requires an explicitly pinned public key".into(),
                )
            })?;
            let signature = OpenPgpSignature::decode(payload, signature_limits(options.limits))?;
            Authentication::OpenPgp(trust.authenticate(
                reader.verification_input(),
                &signature,
                SystemTime::now(),
            )?)
        }
        Verification::Cms(payload) => {
            if options.openpgp_trust.is_some() {
                return Err(Error::Trust(
                    "OpenPGP key pins require an OpenPGP-authenticated file".into(),
                ));
            }
            let signature = CmsSignature::decode(payload, signature_limits(options.limits))?;
            let trust = &options.cms_trust;
            Authentication::Cms(signature.authenticate(
                reader.verification_input(),
                &trust.signers,
                &trust.issuers,
                &trust.revocation_lists,
                SystemTime::now(),
            )?)
        }
    };
    let integrity = reader.verify_checksums()?;
    if !matches!(authentication, Authentication::Unsigned) && !integrity.complete_secure_coverage {
        return Err(Error::Trust(
            "signed execution requires secure checksums covering the complete container".into(),
        ));
    }
    let applications = read_applications(&mut reader)?;
    let application = select_application(&applications, options.application.as_deref())?;
    let mut blobs = BlobStore::new(reader);
    let mut roots = Roots::default();
    let mut failures = Vec::new();
    for (runtime, sdk_lease) in crate::sdk::application_runtimes(&options.java)? {
        let executable = runtime.executable.clone();
        let result = prepare_runtime(
            options,
            application,
            runtime,
            &mut blobs,
            &mut roots,
            integrity,
            authentication.clone(),
        );
        match result {
            Ok(mut plan) => {
                plan.sdk_lease = sdk_lease;
                return Ok(plan);
            }
            Err(error) if options.java.is_explicit() => return Err(error),
            Err(error) => failures.push(format!("{}: {error}", executable.display())),
        }
    }
    Err(invalid(if failures.is_empty() {
        "no Java runtime found; specify --java or --java-home".into()
    } else {
        format!("no compatible Java runtime:\n{}", failures.join("\n"))
    }))
}

/// Prepares indexed or materialized resources for one candidate runtime.
fn prepare_runtime(
    options: &RunOptions,
    application: &Application,
    runtime: JavaRuntime,
    blobs: &mut BlobStore<Cursor<Vec<u8>>>,
    roots: &mut Roots,
    integrity: IntegrityReport,
    authentication: Authentication,
) -> Result<ExecutionPlan> {
    let context = runtime_context(&runtime, Some(&options.invocation))?;
    let launch = application
        .evaluate_java(&context)?
        .ok_or_else(|| invalid("application conditions do not match this runtime"))?;
    if runtime.feature < 9
        && (!launch.module_path.is_empty() || launch.entry_point.main_module.is_some())
    {
        return Err(invalid("module launching requires Java 9 or later"));
    }
    let directory = if options.launch_mode == LaunchMode::Direct || !launch.agents.is_empty() {
        Some(tempfile::Builder::new().prefix("janex-run-").tempdir()?)
    } else {
        None
    };
    let work_path = directory
        .as_ref()
        .map(TempDir::path)
        .unwrap_or_else(|| Path::new(""));
    roots.acquire(
        launch
            .class_path
            .iter()
            .chain(&launch.module_path)
            .chain(launch.agents.iter().map(|agent| &agent.reference)),
        &options.dependencies,
        crate::import::ImportOptions {
            limits: options.limits,
            max_total_bytes: options.max_materialized_bytes,
        },
        !matches!(authentication, Authentication::Unsigned),
    )?;
    let inventory = crate::modules::inventory(&launch.module_path, &context, blobs, roots)?;
    let requirements: Vec<_> = launch
        .module_path
        .iter()
        .filter_map(PathEntry::module_requirement)
        .collect();
    let system_roots = janex_java::modules::system_roots(
        &runtime,
        &inventory,
        &requirements,
        launch.entry_point.main_module.as_deref(),
    )?;
    let mut materializer = Paths {
        directory: work_path,
        context: &context,
        roots,
        blobs,
        paths: BTreeMap::new(),
        remaining_bytes: options.max_materialized_bytes,
    };
    let class_path = launch
        .class_path
        .iter()
        .filter(|_| options.launch_mode == LaunchMode::Direct)
        .map(|entry| materializer.local(entry))
        .collect::<Result<Vec<_>>>()?;
    let mut module_path = Vec::new();
    for entry in &launch.module_path {
        if entry.module_requirement().is_none() && options.launch_mode == LaunchMode::Direct {
            module_path.push(materializer.local(entry)?);
        }
    }
    let mut agents = Vec::new();
    for (index, agent) in launch.agents.iter().enumerate() {
        let source = materializer.local(&agent.reference)?;
        // The native agent loader rejects Windows verbatim paths before calling the system loader.
        let path = work_path.join(format!("agent-{index}.jar"));
        if path.as_os_str().as_encoded_bytes().contains(&b'=') {
            return Err(invalid(
                "Java agent path contains an unrepresentable equals sign",
            ));
        }
        fs::hard_link(&source, &path)?;
        let mut argument = OsString::from("-javaagent:");
        argument.push(path);
        if !agent.option.is_empty() {
            argument.push("=");
            argument.push(&agent.option);
        }
        agents.push(argument);
    }
    let remaining_bytes = materializer.remaining_bytes;
    drop(materializer);
    let resources = if options.launch_mode == LaunchMode::Bootstrap {
        Some(crate::bootstrap::prepare(
            &launch.class_path,
            &launch.module_path,
            &context,
            blobs.reader().limits(),
            roots,
            &fs::canonicalize(target_path(&options.target)?)?,
            remaining_bytes,
        )?)
    } else {
        None
    };
    options
        .limits
        .elements(launch.arguments.len() as u64 + options.arguments.len() as u64)?;
    let program_arguments: Vec<_> = launch
        .arguments
        .iter()
        .map(OsString::from)
        .chain(options.arguments.iter().cloned())
        .collect();
    let mut jvm_options = launch.jvm_options.clone();
    if resources.is_some() && !system_roots.is_empty() {
        jvm_options.push(format!(
            "--add-modules={}",
            system_roots.into_iter().collect::<Vec<_>>().join(",")
        ));
    }
    let mut arguments = LaunchRequest {
        entry_point: EntryPoint {
            main_class: launch.entry_point.main_class.clone(),
            main_module: launch.entry_point.main_module.clone(),
        },
        mode: options.launch_mode,
        jvm_options,
        class_path,
        module_path,
        agents,
        arguments: program_arguments,
    }
    .prepare_with_resources(
        &runtime,
        work_path,
        java_limits(options.limits),
        resources
            .as_ref()
            .map(|resources| resources.data.as_slice()),
    )?;
    let environment = launch_environment(&mut arguments, options.launch_mode);
    Ok(ExecutionPlan {
        environment,
        runtime,
        launch_mode: options.launch_mode,
        arguments,
        integrity,
        authentication,
        windowed: application.windowed(),
        directory,
        sdk_lease: None,
    })
}

/// Moves large bootstrap handoff data into environment chunks for both package and JAR launches.
fn launch_environment(arguments: &mut [OsString], mode: LaunchMode) -> Vec<(OsString, OsString)> {
    let mut environment = Vec::new();
    if mode == LaunchMode::Bootstrap
        && let Some(argument) = arguments.iter_mut().rev().find(|argument| {
            argument
                .to_str()
                .is_some_and(|value| value.starts_with("-Djanex.launch="))
        })
    {
        let value = argument
            .to_str()
            .expect("private launch property is ASCII")
            .strip_prefix("-Djanex.launch=")
            .expect("matched launch property");
        if value.len() > 8_000 {
            for (index, chunk) in value.as_bytes().chunks(8_000).enumerate() {
                environment.push((
                    format!("JANEX_LAUNCH_{index}").into(),
                    std::str::from_utf8(chunk).expect("Base64 is ASCII").into(),
                ));
            }
            *argument = format!("-Djanex.launch=env:{}", environment.len()).into();
        }
    }
    environment
}

/// Lazily decodes roots and materializes each distinct root once for a candidate runtime.
struct Paths<'a> {
    /// Private parent directory for numbered per-root directories.
    directory: &'a Path,
    /// Candidate context controlling resource layers.
    context: &'a Context,
    /// Decoded roots shared between runtime attempts over the same immutable snapshot.
    roots: &'a mut Roots,
    /// Source container and decoded table-page cache.
    blobs: &'a mut BlobStore<Cursor<Vec<u8>>>,
    /// Materialized paths for this candidate only.
    paths: BTreeMap<RootKey, PathBuf>,
    /// Remaining aggregate materialization allowance.
    remaining_bytes: u64,
}

impl Paths<'_> {
    /// Materializes a local or already acquired external root without further network access.
    fn local(&mut self, entry: &PathEntry) -> Result<PathBuf> {
        let key = RootKey::of(entry);
        if let Some(path) = self.paths.get(&key) {
            return Ok(path.clone());
        }
        let root = self.roots.get(entry, self.blobs)?;
        let directory = self.directory.join(self.paths.len().to_string());
        fs::create_dir(&directory)?;
        let tree = root.merge(self.context, self.blobs.reader().limits())?;
        let result = materialize_tree(
            &tree,
            &crate::adapters::jar_name(root)?,
            self.blobs,
            &directory,
            self.remaining_bytes,
        )?;
        self.remaining_bytes -= result.logical_bytes;
        self.paths.insert(key, result.path.clone());
        Ok(result.path)
    }
}

/// Reads an owned bounded snapshot, avoiding later reads from a mutable source file.
fn snapshot(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let limit = max_bytes
        .checked_add(1)
        .ok_or_else(|| invalid("invalid snapshot byte limit"))?;
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("run target must be a regular local file"));
    }
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(invalid("input snapshot byte limit exceeded"));
    }
    Ok(bytes)
}

/// Converts a local file URI without enabling remote file authorities or acquisition.
fn target_path(target: &Path) -> Result<PathBuf> {
    let Some(text) = target.to_str() else {
        return Ok(target.into());
    };
    if text
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
    {
        let uri =
            url::Url::parse(text).map_err(|error| invalid(format!("invalid file URI: {error}")))?;
        if uri.query().is_some()
            || uri.fragment().is_some()
            || uri
                .host_str()
                .is_some_and(|host| !host.eq_ignore_ascii_case("localhost"))
        {
            return Err(invalid(
                "file URI must be local and have no query or fragment",
            ));
        }
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'%'
                && !text
                    .as_bytes()
                    .get(index + 1..index + 3)
                    .is_some_and(|bytes| bytes.iter().all(u8::is_ascii_hexdigit))
            {
                return Err(invalid("invalid percent escape in file URI"));
            }
        }
        return uri
            .to_file_path()
            .map_err(|_| invalid("file URI cannot be represented as a local path"));
    }
    if !target.is_absolute() && url::Url::parse(text).is_ok() {
        return Err(Error::Unsupported(
            "run accepts local files only; remote acquisition is unavailable".into(),
        ));
    }
    Ok(target.into())
}

/// Reads a JAR's selected main class and its minimum class-file Java feature.
/// External manifest paths and native-launcher-only manifest actions are not supported here.
pub(crate) fn jar_entry(
    input: impl Read + std::io::Seek,
    main_class: Option<&str>,
) -> Result<(String, u32)> {
    let mut jar = zip::ZipArchive::new(input)
        .map_err(|e| invalid(format!("invalid application JAR: {e}")))?;
    let mut manifest = Vec::new();
    match jar.by_name("META-INF/MANIFEST.MF") {
        Ok(file) => {
            file.take(1024 * 1024 + 1).read_to_end(&mut manifest)?;
        }
        Err(zip::result::ZipError::FileNotFound) if main_class.is_some() => {}
        Err(error) => {
            return Err(invalid(format!(
                "cannot read application manifest: {error}"
            )));
        }
    }
    if manifest.len() > 1024 * 1024 {
        return Err(invalid("application manifest exceeds byte limit"));
    }
    let manifest = janex_java::manifest::Manifest::parse(&manifest, janex_java::Limits::default())?;
    for name in [
        "Class-Path",
        "Launcher-Agent-Class",
        "Add-Exports",
        "Add-Opens",
        "Enable-Native-Access",
    ] {
        if manifest
            .get(name)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(Error::Unsupported(format!(
                "JAR installation does not support manifest {name}; use a self-contained Janex package"
            )));
        }
    }
    let main = main_class
        .or_else(|| manifest.get("Main-Class"))
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            invalid("application JAR has no Main-Class; specify a main-class installation option")
        })?
        .to_owned();
    let mut header = [0; 8];
    jar.by_name(&format!("{}.class", main.replace('.', "/")))
        .map_err(|_| invalid("application main class is missing from the JAR"))?
        .read_exact(&mut header)?;
    if header[..4] != [0xca, 0xfe, 0xba, 0xbe] {
        return Err(invalid("invalid application main class header"));
    }
    let feature = u32::from(u16::from_be_bytes([header[6], header[7]]))
        .saturating_sub(44)
        .max(8);
    Ok((main, feature))
}

/// Prepares an installed JAR with its saved entry point and ordered runtime dependencies.
/// The caller retains all installation files unchanged until the returned plan finishes executing.
pub(crate) fn prepare_jar(
    options: &RunOptions,
    main: &str,
    feature: u32,
    dependencies: &[PathBuf],
) -> Result<ExecutionPlan> {
    if options.openpgp_trust.is_some() || !options.cms_trust.signers.is_empty() {
        return Err(Error::Trust(
            "Janex signer pins do not authenticate JAR signatures".into(),
        ));
    }
    if options.application.is_some() {
        return Err(invalid(
            "JAR applications have a single installed entry point",
        ));
    }
    let target = fs::canonicalize(&options.target)?;
    let mut class_path = vec![janex_java::runtime::java_path(&target)];
    for dependency in dependencies {
        if !dependency.is_file() {
            return Err(invalid(format!(
                "installed application dependency is missing: {}",
                dependency.display()
            )));
        }
        class_path.push(janex_java::runtime::java_path(dependency));
    }
    for (runtime, sdk_lease) in crate::sdk::application_runtimes(&options.java)? {
        if runtime.feature < feature {
            continue;
        }
        let mut arguments = LaunchRequest {
            entry_point: EntryPoint {
                main_class: Some(main.into()),
                main_module: None,
            },
            mode: options.launch_mode,
            jvm_options: Vec::new(),
            class_path,
            module_path: Vec::new(),
            agents: Vec::new(),
            arguments: options.arguments.clone(),
        }
        .prepare(
            &runtime,
            target.parent().unwrap(),
            java_limits(options.limits),
        )?;
        let environment = launch_environment(&mut arguments, options.launch_mode);
        return Ok(ExecutionPlan {
            runtime,
            launch_mode: options.launch_mode,
            arguments,
            environment,
            integrity: IntegrityReport {
                checksums_verified: 0,
                complete_secure_coverage: false,
            },
            authentication: Authentication::Unsigned,
            windowed: false,
            directory: None,
            sdk_lease,
        });
    }
    Err(invalid(format!(
        "application requires Java {feature} or later"
    )))
}

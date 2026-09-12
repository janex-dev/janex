// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Local snapshot verification, runtime selection, and Java process execution.

use crate::{
    Error, Result,
    authentication::{Authentication, CmsTrust, OpenPgpCertificate},
    error::invalid,
    java::{self, JavaOptions, JavaRuntime},
    materialize::materialize,
};
use janex_format::{
    application::{Application, JavaLaunch, PathEntry, read_applications, select_application},
    binary::Limits,
    blob::{BlobRef, BlobStore},
    condition::Context,
    container::{IntegrityReport, Reader, Verification},
    resource::ResourceRoot,
    signature::{cms::CmsSignature, openpgp::OpenPgpSignature},
};
use std::{
    collections::{BTreeMap, btree_map::Entry},
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
    /// Native local path or local `file:` URI; remote acquisition is unsupported.
    pub target: PathBuf,
    /// Explicit application ID; absent selects the sole application.
    pub application: Option<String>,
    /// Java executable or home override, otherwise JAVA_HOME followed by PATH.
    pub java: JavaOptions,
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
    /// Maximum aggregate logical bytes written across distinct materialized roots.
    pub max_materialized_bytes: u64,
}

impl RunOptions {
    /// Creates options requiring authenticated input, with 512 MiB aggregate limits.
    pub fn new(target: impl Into<PathBuf>) -> Self {
        Self {
            target: target.into(),
            application: None,
            java: JavaOptions::default(),
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

/// A selected Java invocation owning its temporary JARs until dropped.
///
/// Preparation verifies one owned input snapshot. Later changes to the source file do not
/// affect this plan. It is not a persistent verification cache or a publisher trust claim.
#[derive(Debug)]
pub struct ExecutionPlan {
    /// Runtime selected after condition and local-module checks.
    runtime: JavaRuntime,
    /// Complete ordered process arguments, without shell encoding.
    arguments: Vec<OsString>,
    /// Full-snapshot checksum result retained for inspection.
    integrity: IntegrityReport,
    /// Publisher authentication outcome for the immutable snapshot.
    authentication: Authentication,
    /// Whether the application requests a windowless Windows process.
    windowed: bool,
    /// Lifetime owner of every generated Java path entry.
    directory: TempDir,
}

impl ExecutionPlan {
    /// Returns the selected runtime and its probed properties.
    pub fn runtime(&self) -> &JavaRuntime {
        &self.runtime
    }

    /// Returns JVM options, entry point, and program arguments in process order.
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

    /// Returns the temporary directory removed when this plan is dropped.
    pub fn directory(&self) -> &Path {
        self.directory.path()
    }

    /// Creates a direct Java command inheriting the current environment, directory, and streams.
    ///
    /// The caller may customize its I/O. Keep this plan alive until the child process exits;
    /// dropping it removes files that the child may still need to load.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.runtime.executable);
        command
            .args(&self.arguments)
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

/// Verifies a local snapshot and prepares the first compatible Java invocation.
///
/// Explicit runtime selection disables fallback. None and Checksum inputs require
/// `allow_unsigned`; signed inputs require signature support and never fall back to that policy.
/// No application main method or descriptor-supplied agent runs during preparation.
pub fn prepare(options: &RunOptions) -> Result<ExecutionPlan> {
    if options.openpgp_trust.is_some() && !options.cms_trust.signers.is_empty() {
        return Err(invalid(
            "OpenPGP and CMS signer pins are mutually exclusive",
        ));
    }
    let bytes = snapshot(&target_path(&options.target)?, options.max_snapshot_bytes)?;
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
            return Err(janex_format::Error::new(
                janex_format::ErrorKind::Trust,
                "signer pins require a file authenticated by the selected signature format",
            )
            .into());
        }
        Verification::None | Verification::Checksum(_) => {
            return Err(invalid(
                "unsigned local execution requires --allow-unsigned",
            ));
        }
        Verification::OpenPgp(payload) => {
            let trust = options.openpgp_trust.as_ref().ok_or_else(|| {
                janex_format::Error::new(
                    janex_format::ErrorKind::Trust,
                    "OpenPGP authentication requires an explicitly pinned public key",
                )
            })?;
            let signature = OpenPgpSignature::decode(payload, options.limits)?;
            Authentication::OpenPgp(trust.authenticate(
                reader.verification_input(),
                &signature,
                SystemTime::now(),
            )?)
        }
        Verification::Cms(payload) => {
            if options.openpgp_trust.is_some() {
                return Err(janex_format::Error::new(
                    janex_format::ErrorKind::Trust,
                    "OpenPGP key pins require an OpenPGP-authenticated file",
                )
                .into());
            }
            let signature = CmsSignature::decode(payload, options.limits)?;
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
        return Err(janex_format::Error::new(
            janex_format::ErrorKind::Trust,
            "signed execution requires secure checksums covering the complete container",
        )
        .into());
    }
    let applications = read_applications(&mut reader)?;
    let application = select_application(&applications, options.application.as_deref())?;
    let mut blobs = BlobStore::new(reader);
    let mut roots = BTreeMap::new();
    let mut failures = Vec::new();
    for executable in java::candidates(&options.java)? {
        let result = JavaRuntime::probe(&executable).and_then(|runtime| {
            prepare_runtime(
                options,
                application,
                runtime,
                &mut blobs,
                &mut roots,
                integrity,
                authentication.clone(),
            )
        });
        match result {
            Ok(plan) => return Ok(plan),
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

/// Resolves and materializes the configuration evaluated against one candidate runtime.
fn prepare_runtime(
    options: &RunOptions,
    application: &Application,
    runtime: JavaRuntime,
    blobs: &mut BlobStore<Cursor<Vec<u8>>>,
    roots: &mut BTreeMap<BlobRef, ResourceRoot>,
    integrity: IntegrityReport,
    authentication: Authentication,
) -> Result<ExecutionPlan> {
    let context = runtime.context(Some("run"));
    let launch = application
        .evaluate_java(&context)?
        .ok_or_else(|| invalid("application conditions do not match this runtime"))?;
    if runtime.version.feature() < 9
        && (!launch.module_path.is_empty() || launch.entry_point.main_module.is_some())
    {
        return Err(invalid("module launching requires Java 9 or later"));
    }
    let directory = tempfile::Builder::new().prefix("janex-run-").tempdir()?;
    let mut materializer = Paths {
        directory: directory.path(),
        context: &context,
        roots,
        blobs,
        paths: BTreeMap::new(),
        remaining_bytes: options.max_materialized_bytes,
    };
    let class_path = launch
        .class_path
        .iter()
        .map(|entry| materializer.local(entry))
        .collect::<Result<Vec<_>>>()?;
    let mut module_path = Vec::new();
    let mut requirements = Vec::new();
    for entry in &launch.module_path {
        if let Some(requirement) = entry.module_requirement() {
            requirements.push(requirement);
        } else {
            module_path.push(materializer.local(entry)?);
        }
    }
    let modules = runtime.validate_module_path(&module_path)?;
    for (name, version) in requirements {
        let actual = modules
            .get(&name)
            .ok_or_else(|| invalid(format!("required module is unavailable: {name}")))?;
        if let Some(version) = version
            && actual.as_deref() != Some(&version)
        {
            return Err(invalid(format!(
                "required module version is unavailable: {name}@{version}"
            )));
        }
    }
    if let Some(name) = &launch.entry_point.main_module
        && !modules.contains_key(name)
    {
        return Err(invalid(format!("main module is unavailable: {name}")));
    }
    let mut agents = Vec::new();
    for (index, agent) in launch.agents.iter().enumerate() {
        let mut path = materializer.local(&agent.reference)?;
        if path.as_os_str().as_encoded_bytes().contains(&b'=') {
            let alias = directory.path().join(format!("agent-{index}.jar"));
            if alias.as_os_str().as_encoded_bytes().contains(&b'=') {
                return Err(invalid(
                    "Java agent path contains an unrepresentable equals sign",
                ));
            }
            fs::hard_link(&path, &alias)?;
            path = alias;
        }
        let mut argument = OsString::from("-javaagent:");
        argument.push(path);
        if !agent.option.is_empty() {
            argument.push("=");
            argument.push(&agent.option);
        }
        agents.push(argument);
    }
    let arguments = arguments(
        &runtime,
        &launch,
        class_path,
        module_path,
        agents,
        &directory,
        &options.arguments,
    )?;
    Ok(ExecutionPlan {
        runtime,
        arguments,
        integrity,
        authentication,
        windowed: application.windowed(),
        directory,
    })
}

/// Lazily decodes roots and materializes each distinct root once for a candidate runtime.
struct Paths<'a> {
    /// Private parent directory for numbered per-root directories.
    directory: &'a Path,
    /// Candidate context controlling resource layers.
    context: &'a Context,
    /// Decoded roots shared between runtime attempts over the same immutable snapshot.
    roots: &'a mut BTreeMap<BlobRef, ResourceRoot>,
    /// Source container and decoded table-page cache.
    blobs: &'a mut BlobStore<Cursor<Vec<u8>>>,
    /// Materialized paths for this candidate only.
    paths: BTreeMap<BlobRef, PathBuf>,
    /// Remaining aggregate materialization allowance.
    remaining_bytes: u64,
}

impl Paths<'_> {
    /// Resolves a local reference without consulting external providers.
    fn local(&mut self, entry: &PathEntry) -> Result<PathBuf> {
        let PathEntry::Local(reference) = entry else {
            return Err(Error::Unsupported("external Java path entries require a resolver; supply local dependencies when packing".into()));
        };
        if let Some(path) = self.paths.get(reference) {
            return Ok(path.clone());
        }
        let root = match self.roots.entry(*reference) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let bytes = self.blobs.resolve(*reference)?;
                entry.insert(ResourceRoot::decode(&bytes, self.blobs)?)
            }
        };
        let directory = self.directory.join(self.paths.len().to_string());
        fs::create_dir(&directory)?;
        let result = materialize(
            root,
            self.context,
            self.blobs,
            &directory,
            self.remaining_bytes,
        )?;
        self.remaining_bytes -= result.logical_bytes;
        self.paths.insert(*reference, result.path.clone());
        Ok(result.path)
    }
}

/// Assembles native arguments without interpreting whitespace or shell metacharacters.
fn arguments(
    runtime: &JavaRuntime,
    launch: &JavaLaunch,
    mut class_path: Vec<PathBuf>,
    module_path: Vec<PathBuf>,
    agents: Vec<OsString>,
    directory: &TempDir,
    user_arguments: &[OsString],
) -> Result<Vec<OsString>> {
    let mut arguments = Vec::new();
    if runtime.version.feature() >= 9 {
        arguments.push("--disable-@files".into());
    }
    arguments.extend(launch.jvm_options.iter().map(OsString::from));
    if class_path.is_empty() {
        let empty = directory.path().join("empty-classpath");
        fs::create_dir(&empty)?;
        class_path.push(empty);
    }
    arguments.push("-cp".into());
    arguments.push(java::join_path(&class_path)?);
    if !module_path.is_empty() {
        arguments.push("--module-path".into());
        arguments.push(java::join_path(&module_path)?);
    }
    arguments.extend(agents);
    if let Some(module) = &launch.entry_point.main_module {
        let entry = launch
            .entry_point
            .main_class
            .as_ref()
            .map_or_else(|| module.clone(), |class| format!("{module}/{class}"));
        arguments.push("--module".into());
        arguments.push(entry.into());
    } else {
        arguments.push(
            launch
                .entry_point
                .main_class
                .as_ref()
                .expect("validated classpath entry point")
                .into(),
        );
    }
    arguments.extend(launch.arguments.iter().map(OsString::from));
    arguments.extend_from_slice(user_arguments);
    if arguments
        .iter()
        .any(|argument| argument.as_encoded_bytes().contains(&0))
    {
        return Err(invalid("Java process arguments must not contain NUL"));
    }
    Ok(arguments)
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

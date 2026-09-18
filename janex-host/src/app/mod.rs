// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Maven application installation, cached execution, and native command dispatch.

mod request;
mod run;
pub use request::{AppRequest, DependencyMode, JarOptions};

use crate::{
    Result,
    dependency::DependencyOptions,
    error::invalid,
    persistence::{from_cbor, publish, to_cbor},
    run::RunOptions,
};
use janex_format::{
    binary::Limits,
    cbor::Value,
    checksum::{Algorithm, Checksum},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::ExitStatus,
};
use url::Url;

/// One immutable installed application release, independent of disposable caches.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installation {
    /// Exact installation identifier, prefixed with `app-`.
    pub id: String,
    /// Concrete artifact coordinates and command name.
    pub application: AppRequest,
    /// SHA-256 of the original, unmodified artifact.
    pub sha256: String,
    /// Original artifact URL.
    pub source: String,
    /// Installed JAR entry point and ordered dependency closure; absent for Janex containers.
    pub jar: Option<JarLaunch>,
}

/// Installation-time launch data, read without consulting POMs during startup.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JarLaunch {
    /// Binary name of the selected application entry point.
    pub main_class: String,
    /// Minimum Java feature required by the entry-point class file.
    pub java_feature: u32,
    /// Runtime dependencies in classpath order; the application JAR precedes these entries.
    pub dependencies: Vec<InstalledDependency>,
}

/// An immutable dependency owned by an application installation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledDependency {
    /// Canonical concrete Maven package identity.
    pub purl: String,
    /// Original download URL.
    pub source: String,
    /// SHA-256 of the unmodified dependency bytes.
    pub sha256: String,
    /// Path relative to the application installation directory.
    pub file: PathBuf,
}

/// A saved version requirement and its selected installed release.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selection {
    /// User requirement, including its repository and command name.
    pub request: AppRequest,
    /// Current installation satisfying this requirement.
    pub installation: String,
    /// Whether updates must retain the current installation.
    pub pinned: bool,
}

/// A command selection that follows a request or fixes an exact installation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandSelection {
    /// Requirement used when no installation ID is fixed.
    pub request: AppRequest,
    /// Fixed installation ID, or `None` to follow the saved request's updates.
    pub installation: Option<String>,
}

/// Application registry snapshot; SDK records are stored separately.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppStatus {
    /// Internal registry schema version.
    pub schema: u32,
    /// All installed versions, including inactive ones.
    pub installations: Vec<Installation>,
    /// Update requirements and pins.
    pub selections: Vec<Selection>,
    /// Active command entry points indexed by command name.
    pub commands: BTreeMap<String, CommandSelection>,
}

impl AppStatus {
    /// Resolves active commands within this snapshot, retaining command-name order.
    pub fn active_commands(&self) -> Result<BTreeMap<&str, &Installation>> {
        self.commands
            .iter()
            .map(|(name, selection)| Ok((name.as_str(), command_installation(self, selection)?)))
            .collect()
    }
}

impl Default for AppStatus {
    fn default() -> Self {
        Self {
            schema: 1,
            installations: Vec::new(),
            selections: Vec::new(),
            commands: BTreeMap::new(),
        }
    }
}

/// Application management scoped to an absolute Janex home.
pub struct AppManager {
    /// User directory containing persistent installations, state, and disposable caches.
    root: PathBuf,
}

impl AppManager {
    /// Uses the configured Janex home without creating it.
    pub fn user() -> Result<Self> {
        Self::new(janex_platform::janex_home()?)
    }

    /// Uses an absolute home without changing process environment variables.
    pub fn new(root: PathBuf) -> Result<Self> {
        if !root.is_absolute() {
            return Err(invalid("application home must be absolute"));
        }
        Ok(Self { root })
    }

    /// Reads a consistent installation and command snapshot without network access.
    pub fn status(&self) -> Result<AppStatus> {
        let _lock = self.lock(false)?;
        self.read()
    }

    /// Installs or reuses a requested release and creates its command if no default exists.
    /// Downloads and validates before publication; existing versions and command defaults are retained.
    /// `launcher` is the native Janex executable copied or linked as the command entry point.
    pub fn install(
        &self,
        request: &AppRequest,
        pin: bool,
        options: &DependencyOptions,
        launcher: &Path,
    ) -> Result<Installation> {
        request.validate()?;
        if options.offline && options.refresh {
            return Err(invalid("offline and refresh are mutually exclusive"));
        }
        {
            let _lock = self.lock(false)?;
            let state = self.read()?;
            self.check_command(&state, request)?;
            if !options.refresh
                && let Some(selection) = state.selections.iter().find(|s| s.request == *request)
            {
                let installed = installation(&state, &selection.installation)?.clone();
                if !self.artifact_path(&installed).is_file() {
                    return Err(invalid(
                        "installed application is missing; uninstall and reinstall it",
                    ));
                }
                drop(_lock);
                // Re-read after acquiring the write lock: another process may update or remove the binding.
                let _lock = self.lock(true)?;
                let mut state = self.read()?;
                let installed = resolve(&state, &request.target())?.clone();
                self.check_command(&state, request)?;
                self.ensure_entry(&request.command, launcher)?;
                let mut changed = false;
                if pin
                    && let Some(selection) = state
                        .selections
                        .iter_mut()
                        .find(|s| s.request == *request && !s.pinned)
                {
                    selection.pinned = true;
                    changed = true;
                }
                if !state.commands.contains_key(&request.command) {
                    state.commands.insert(
                        request.command.clone(),
                        CommandSelection {
                            request: request.clone(),
                            installation: None,
                        },
                    );
                    changed = true;
                }
                if changed {
                    self.save(&state)?;
                }
                return Ok(installed);
            }
        }
        let exact = exact_request(request, |url| self.fetch(url, options, 1024 * 1024))?;
        let url = exact.url(false)?;
        let bytes = self.fetch(&url, options, options.max_bytes)?;
        let mut jar = validate_artifact(&bytes, &exact)?;
        let sha256 = digest(&bytes)?;
        let repository = Url::parse(&exact.repository).unwrap();
        let dependencies =
            runtime_dependencies(&exact, |url| self.fetch(url, options, 4 * 1024 * 1024))?;
        fs::create_dir_all(self.root.join("apps"))?;
        let stage = tempfile::tempdir_in(self.root.join("apps"))?;
        write_artifact(&stage.path().join(exact.filename()), &bytes)?;
        for dependency in dependencies {
            let url = dependency.url(&repository);
            let bytes = self.fetch(&url, options, options.max_bytes)?;
            zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(|e| {
                invalid(format!(
                    "invalid dependency JAR {}: {e}",
                    dependency.filename()
                ))
            })?;
            let sha256 = digest(&bytes)?;
            let file = PathBuf::from("lib")
                .join(&sha256)
                .join(dependency.filename());
            write_artifact(&stage.path().join(&file), &bytes)?;
            jar.as_mut()
                .unwrap()
                .dependencies
                .push(InstalledDependency {
                    purl: dependency.purl(&repository),
                    source: url.into(),
                    sha256,
                    file,
                });
        }
        let id = format!(
            "app-{}",
            digest(
                &serde_json::to_vec(&(&exact, &sha256, &jar))
                    .map_err(|e| invalid(e.to_string()))?
            )?
        );
        let installed = Installation {
            id,
            application: exact,
            sha256,
            source: url.into(),
            jar,
        };
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        self.check_command(&state, request)?;
        if options.refresh
            && let Some(selection) = state
                .selections
                .iter()
                .find(|s| s.request == *request && s.pinned)
        {
            return Ok(installation(&state, &selection.installation)?.clone());
        }
        let directory = self.root.join("apps").join(&installed.id);
        let artifact = self.artifact_path(&installed);
        if artifact.exists() {
            if digest(&read_bounded(&artifact, options.max_bytes)?)? != installed.sha256 {
                return Err(invalid(
                    "existing application content does not match its identity",
                ));
            }
        } else {
            fs::rename(stage.path(), &directory)?;
        }
        fs::create_dir_all(self.root.join("state/app-leases"))?;
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lease_path(&installed.id))?;
        if !state.installations.iter().any(|i| i.id == installed.id) {
            state.installations.push(installed.clone());
        }
        if let Some(selection) = state.selections.iter_mut().find(|s| s.request == *request) {
            selection.installation = installed.id.clone();
            selection.pinned |= pin;
        } else {
            state.selections.push(Selection {
                request: request.clone(),
                installation: installed.id.clone(),
                pinned: pin,
            });
        }
        let new_command = !state.commands.contains_key(&request.command);
        self.ensure_entry(&request.command, launcher)?;
        state
            .commands
            .entry(request.command.clone())
            .or_insert_with(|| CommandSelection {
                request: request.clone(),
                installation: None,
            });
        if let Err(error) = self.save(&state) {
            if new_command {
                let _ = fs::remove_file(self.entry_path(&request.command));
            }
            return Err(error);
        }
        Ok(installed)
    }

    /// Refreshes one saved requirement unless pinned, retaining all previous releases.
    pub fn update(
        &self,
        request: &AppRequest,
        options: &DependencyOptions,
        launcher: &Path,
    ) -> Result<Installation> {
        let state = self.status()?;
        let selection = state
            .selections
            .iter()
            .find(|s| s.request == *request)
            .ok_or_else(|| invalid("no saved application requirement matches this update"))?;
        if selection.pinned {
            return Ok(installation(&state, &selection.installation)?.clone());
        }
        let mut options = options.clone();
        options.refresh = !options.offline;
        self.install(request, false, &options, launcher)
    }

    /// Changes the update policy of an existing request without downloading.
    pub fn set_pin(&self, request: &AppRequest, pinned: bool) -> Result<()> {
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        state
            .selections
            .iter_mut()
            .find(|s| s.request == *request)
            .ok_or_else(|| invalid("no saved application requirement matches this pin request"))?
            .pinned = pinned;
        self.save(&state)
    }

    /// Resolves a saved selector or an exact installation ID without downloading.
    pub fn resolve(&self, target: &str) -> Result<Installation> {
        Ok(resolve(&self.status()?, target)?.clone())
    }

    /// Returns the persistent directory containing the selected application artifact.
    pub fn home(&self, target: &str) -> Result<PathBuf> {
        Ok(self.root.join("apps").join(self.resolve(target)?.id))
    }

    /// Selects the release exposed by its command; selectors follow updates and IDs remain fixed.
    pub fn set_default(&self, target: &str, launcher: &Path) -> Result<Installation> {
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        let installed = resolve(&state, target)?.clone();
        let fixed = target == installed.id;
        let request = if fixed {
            installed.application.clone()
        } else {
            AppRequest::parse(target)?
        };
        self.check_command(&state, &request)?;
        self.ensure_entry(&request.command, launcher)?;
        state.commands.insert(
            request.command.clone(),
            CommandSelection {
                request,
                installation: fixed.then(|| installed.id.clone()),
            },
        );
        self.save(&state)?;
        Ok(installed)
    }

    /// Removes an application's command selection while retaining its installed releases.
    /// If the native entry cannot be removed, the registry is left unchanged.
    pub fn clear_default(&self, target: &str) -> Result<()> {
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        let request = resolve(&state, target)?.application.clone();
        let command = request.command.clone();
        if let Some(current) = state.commands.get(&command)
            && !current.request.same_product(&request)
        {
            return Err(invalid("command belongs to another application"));
        }
        let removed = state.commands.remove(&command).is_some();
        if removed {
            remove_file(&self.entry_path(&command))?;
        }
        self.save(&state)
    }

    /// Removes one exact release, rejecting a running application. Other releases are retained.
    /// Removing the active command's release removes that command; it does not select an older release.
    pub fn uninstall(&self, target: &str) -> Result<Installation> {
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        if !target.starts_with("app-") && AppRequest::parse(target)?.version.is_none() {
            return Err(invalid(
                "application uninstall requires an exact version or installation ID",
            ));
        }
        let installed = resolve(&state, target)?.clone();
        let lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.lease_path(&installed.id))?;
        fs4::FileExt::try_lock(&lease)
            .map_err(|e| invalid(format!("application is in use or cannot be locked: {e}")))?;
        let commands = state
            .commands
            .iter()
            .filter_map(|(name, selection)| {
                command_installation(&state, selection)
                    .ok()
                    .filter(|i| i.id == installed.id)
                    .map(|_| name.clone())
            })
            .collect::<Vec<_>>();
        for name in &commands {
            state.commands.remove(name);
        }
        state.selections.retain(|s| s.installation != installed.id);
        state.installations.retain(|i| i.id != installed.id);
        for name in commands {
            remove_file(&self.entry_path(&name))?;
        }
        self.save(&state)?;
        fs::remove_dir_all(self.root.join("apps").join(&installed.id))?;
        Ok(installed)
    }

    /// Executes a selector or registered command, preferring an installed release.
    /// Installed releases retain a lease until Java exits and run independently of the download cache.
    /// Uninstalled PURLs run from the shared cache without changing installations or command defaults.
    /// Signed containers retain run-time trust checks in both cases.
    pub fn execute(
        &self,
        target: &str,
        command: bool,
        mut options: RunOptions,
    ) -> Result<ExitStatus> {
        let lock = self.lock(false)?;
        let state = self.read()?;
        let installed = if command {
            let selection = state
                .commands
                .get(target)
                .ok_or_else(|| invalid("application command is not registered"))?;
            command_installation(&state, selection)?
        } else if target.starts_with("app-") {
            installation(&state, target)?
        } else {
            let request =
                AppRequest::parse_with_repository(target, &options.dependencies.maven_repository)?;
            match find_request(&state, &request)? {
                Some(installed) => installed,
                None => {
                    drop(lock);
                    return self.execute_cached(&request, options);
                }
            }
        };
        let _lease = fs::File::open(self.lease_path(&installed.id))?;
        fs4::FileExt::lock_shared(&_lease)?;
        options.target = self.artifact_path(installed);
        drop(lock);
        if command {
            options.invocation = "command".into();
        }
        options.allow_unsigned = true;
        if let Some(jar) = &installed.jar {
            let directory = options.target.parent().unwrap();
            let class_path = jar
                .dependencies
                .iter()
                .map(|d| directory.join(&d.file))
                .collect::<Vec<_>>();
            crate::run::prepare_jar(&options, &jar.main_class, jar.java_feature, &class_path)?
                .execute()
        } else {
            crate::run::prepare(&options)?.execute()
        }
    }

    /// Returns whether this executable is a registered native application entry under this home.
    pub fn entry_name(&self, executable: &Path) -> Result<Option<String>> {
        let Some(parent) = executable.parent() else {
            return Ok(None);
        };
        if !self.root.join("bin").is_dir()
            || parent.canonicalize()? != self.root.join("bin").canonicalize()?
        {
            return Ok(None);
        }
        let name = if cfg!(windows) {
            executable.file_stem()
        } else {
            executable.file_name()
        };
        let Some(name) = name.and_then(|s| s.to_str()) else {
            return Ok(None);
        };
        if name == "janex" {
            return Ok(None);
        }
        request::command_name(name)?;
        Ok(Some(name.into()))
    }

    /// Returns the managed artifact filename, without resolving cache paths.
    fn artifact_path(&self, installed: &Installation) -> PathBuf {
        self.root
            .join("apps")
            .join(&installed.id)
            .join(installed.application.filename())
    }

    /// Returns the persistent lease inode, never removed while a process may hold it.
    fn lease_path(&self, id: &str) -> PathBuf {
        self.root
            .join("state/app-leases")
            .join(format!("{id}.lock"))
    }

    /// Returns the platform-native command filename.
    fn entry_path(&self, name: &str) -> PathBuf {
        self.root.join("bin").join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        })
    }

    /// Rejects replacement of another product's command or an unmanaged file.
    fn check_command(&self, state: &AppStatus, request: &AppRequest) -> Result<()> {
        if let Some(current) = state.commands.get(&request.command) {
            if !current.request.same_product(request) {
                return Err(invalid(
                    "application command is owned by another product; choose a command qualifier",
                ));
            }
        } else if self.entry_path(&request.command).symlink_metadata().is_ok() {
            return Err(invalid(
                "application command path already exists and is not managed",
            ));
        }
        Ok(())
    }

    /// Publishes a native entry without overwriting an existing file; hard links avoid duplicate binaries.
    fn ensure_entry(&self, name: &str, launcher: &Path) -> Result<()> {
        let path = self.entry_path(name);
        if path.is_file() {
            return Ok(());
        }
        fs::create_dir_all(path.parent().unwrap())?;
        if fs::hard_link(launcher, &path).is_err() {
            let temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
            fs::copy(launcher, temporary.path())?;
            temporary.as_file().sync_all()?;
            temporary.persist_noclobber(path).map_err(|e| e.error)?;
        }
        Ok(())
    }

    /// Acquires a persistent registry lock; read-only queries on an unused home create nothing.
    fn lock(&self, exclusive: bool) -> Result<Option<fs::File>> {
        let path = self.root.join("state/apps.lock");
        if exclusive {
            fs::create_dir_all(path.parent().unwrap())?;
        }
        let file = match fs::OpenOptions::new()
            .read(true)
            .write(exclusive)
            .create(exclusive)
            .truncate(false)
            .open(path)
        {
            Ok(file) => file,
            Err(e) if !exclusive && e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if exclusive {
            fs4::FileExt::lock(&file)?;
        } else {
            fs4::FileExt::lock_shared(&file)?;
        }
        Ok(Some(file))
    }

    /// Decodes the bounded CBOR registry and validates paths before using them.
    fn read(&self) -> Result<AppStatus> {
        let bytes = match read_bounded(&self.root.join("state/apps.cbor"), 16 * 1024 * 1024) {
            Ok(bytes) => bytes,
            Err(crate::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AppStatus::default());
            }
            Err(e) => return Err(e),
        };
        let value = Value::from_bytes(&bytes, Limits::default())?;
        let state: AppStatus = serde_json::from_value(from_cbor(&value)?)
            .map_err(|e| invalid(format!("invalid application registry: {e}")))?;
        if state.schema != 1 {
            return Err(invalid("unsupported application registry schema"));
        }
        for installed in &state.installations {
            installed.application.validate()?;
            if installed.application.version.is_none()
                || !valid_id(&installed.id)
                || installed.sha256.len() != 64
                || !installed.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid("invalid application installation identity"));
            }
            if (installed.application.kind == "jar") != installed.jar.is_some() {
                return Err(invalid(
                    "application launch metadata does not match its artifact type",
                ));
            }
            if let Some(jar) = &installed.jar {
                for dependency in &jar.dependencies {
                    if dependency.file.as_os_str().is_empty()
                        || dependency
                            .file
                            .components()
                            .any(|c| !matches!(c, std::path::Component::Normal(_)))
                    {
                        return Err(invalid(
                            "application dependency path escapes its installation",
                        ));
                    }
                }
            }
        }
        for selection in &state.selections {
            selection.request.validate()?;
            let actual = installation(&state, &selection.installation)?;
            if selection
                .request
                .exact(actual.application.version.as_deref().unwrap())?
                != actual.application
            {
                return Err(invalid("application binding mismatch"));
            }
        }
        for (name, selection) in &state.commands {
            request::command_name(name)?;
            selection.request.validate()?;
            if *name != selection.request.command
                || command_installation(&state, selection)?.application.command != *name
            {
                return Err(invalid("application command mismatch"));
            }
        }
        Ok(state)
    }

    /// Atomically replaces the registry after complete artifacts and command entries are published.
    fn save(&self, state: &AppStatus) -> Result<()> {
        let value = to_cbor(serde_json::to_value(state).map_err(|e| invalid(e.to_string()))?)?;
        if value.as_bytes().len() > 16 * 1024 * 1024 {
            return Err(invalid("application registry is too large"));
        }
        publish(&self.root.join("state/apps.cbor"), value.as_bytes())
    }

    /// Reads local repositories directly or uses the shared HTTPS content cache.
    fn fetch(&self, url: &Url, options: &DependencyOptions, limit: u64) -> Result<Vec<u8>> {
        if url.scheme() == "file" {
            return read_bounded(
                &url.to_file_path()
                    .map_err(|_| invalid("invalid local repository path"))?,
                limit,
            );
        }
        let mut options = options.clone();
        options.max_bytes = limit;
        if options.cache_directory.is_none() {
            options.cache_directory = Some(self.root.join("cache/dependencies"));
        }
        Ok(crate::dependency::artifact(url.as_str(), &options)?.bytes)
    }
}

/// Resolves an exact installation identity from a consistent registry snapshot.
fn installation<'a>(state: &'a AppStatus, id: &str) -> Result<&'a Installation> {
    state
        .installations
        .iter()
        .find(|i| i.id == id)
        .ok_or_else(|| invalid("application installation is missing"))
}

/// Resolves a request binding before attempting an unambiguous concrete release match.
fn resolve<'a>(state: &'a AppStatus, target: &str) -> Result<&'a Installation> {
    if target.starts_with("app-") {
        return installation(state, target);
    }
    let request = AppRequest::parse(target)?;
    find_request(state, &request)?.ok_or_else(|| invalid("application is not installed"))
}

/// Finds a saved binding or a single concrete release; ambiguity remains an error.
fn find_request<'a>(
    state: &'a AppStatus,
    request: &AppRequest,
) -> Result<Option<&'a Installation>> {
    if let Some(selection) = state.selections.iter().find(|s| s.request == *request) {
        return installation(state, &selection.installation).map(Some);
    }
    let mut matches = state
        .installations
        .iter()
        .filter(|i| i.application == *request);
    let found = matches.next();
    if matches.next().is_some() {
        return Err(invalid(
            "ambiguous application release; select an installation ID",
        ));
    }
    Ok(found)
}

/// Resolves an omitted version through the repository's explicit release pointer.
fn exact_request(
    request: &AppRequest,
    fetch: impl FnOnce(&Url) -> Result<Vec<u8>>,
) -> Result<AppRequest> {
    if request.version.is_some() {
        return Ok(request.clone());
    }
    request.exact(&release(&fetch(&request.url(true)?)?, request)?)
}

/// Resolves the same runtime graph for installed and cached JAR applications.
fn runtime_dependencies(
    request: &AppRequest,
    fetch: impl FnMut(&Url) -> Result<Vec<u8>>,
) -> Result<Vec<crate::maven::Artifact>> {
    if request.kind != "jar" || request.jar.dependencies == DependencyMode::None {
        return Ok(Vec::new());
    }
    crate::maven::runtime_dependencies(
        &crate::maven::Artifact {
            group: request.group.clone(),
            name: request.artifact.clone(),
            version: request.version.clone().unwrap(),
            extension: "jar".into(),
            classifier: request.classifier.clone(),
        },
        &Url::parse(&request.repository).unwrap(),
        fetch,
    )
}

/// Resolves a command's fixed release or movable version request.
fn command_installation<'a>(
    state: &'a AppStatus,
    selection: &CommandSelection,
) -> Result<&'a Installation> {
    match &selection.installation {
        Some(id) => installation(state, id),
        None => resolve(state, &selection.request.target()),
    }
}

/// Validates the identifier before using it as a filesystem component.
fn valid_id(value: &str) -> bool {
    value.strip_prefix("app-").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Computes a canonical SHA-256 identity.
fn digest(bytes: &[u8]) -> Result<String> {
    Ok(Checksum::compute(Algorithm::Sha256, bytes)?
        .digest()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Reads a bounded local artifact or registry without following archive entries.
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    if limit == u64::MAX {
        return Err(invalid("invalid application byte limit"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("application file exceeds byte limit"));
    }
    Ok(bytes)
}

/// Removes an entry if present, preserving errors other than absence.
fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Reads the repository's explicit release pointer without imposing SDK or SemVer ordering.
fn release(bytes: &[u8], request: &AppRequest) -> Result<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("Maven metadata is not UTF-8"))?;
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
            ..Default::default()
        },
    )
    .map_err(|e| invalid(format!("invalid Maven metadata: {e}")))?;
    let root = document.root_element();
    let child = |name| {
        root.children()
            .find(|n| n.has_tag_name(name))
            .and_then(|n| n.text())
    };
    if !root.has_tag_name("metadata")
        || child("groupId") != Some(&request.group)
        || child("artifactId") != Some(&request.artifact)
    {
        return Err(invalid("Maven metadata has unexpected coordinates"));
    }
    let version = root
        .children()
        .find(|n| n.has_tag_name("versioning"))
        .and_then(|n| n.children().find(|n| n.has_tag_name("release")))
        .and_then(|n| n.text())
        .ok_or_else(|| {
            invalid("Maven metadata has no release version; specify an exact version")
        })?;
    request.exact(version)?;
    Ok(version.into())
}

/// Checks executable metadata without running application code or resolving POM dependencies.
fn validate_artifact(bytes: &[u8], request: &AppRequest) -> Result<Option<JarLaunch>> {
    if request.kind == "jar" {
        let (main_class, java_feature) = crate::run::jar_entry(
            std::io::Cursor::new(bytes),
            request.jar.main_class.as_deref(),
        )?;
        Ok(Some(JarLaunch {
            main_class,
            java_feature,
            dependencies: Vec::new(),
        }))
    } else {
        let mut reader = janex_format::container::Reader::open_auto(
            std::io::Cursor::new(bytes),
            Limits::default(),
        )?;
        reader.verify_checksums()?;
        let applications = janex_format::application::read_applications(&mut reader)?;
        janex_format::application::select_application(&applications, None)?;
        Ok(None)
    }
}

/// Writes an owned installation artifact before publishing the directory.
fn write_artifact(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

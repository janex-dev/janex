// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Atomic SDK registration and persistent installation ownership.

use super::{AvailableSdk, CatalogOptions, SdkRequest, archive, catalog, hex};
use crate::{Result, error::invalid};
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
    path::{Component, Path, PathBuf},
};

/// One complete managed SDK or a registered external SDK home.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installation {
    /// Stable content and variant identity; usable as an exact command target.
    pub id: String,
    /// Complete product release and platform variant of this installation.
    pub sdk: SdkRequest,
    /// Bundled Java release, separate from the product version; absent for portable tools.
    pub java_version: Option<String>,
    /// Home relative to the managed tree, or an absolute external home.
    pub home: PathBuf,
    /// Whether Janex owns and may remove the SDK tree.
    pub managed: bool,
    /// Original download URL, or external registration path.
    pub source: String,
    /// SHA-256 of the downloaded archive; absent for external installations.
    pub sha256: Option<String>,
}

/// A user requirement bound to one completed installation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selection {
    /// Requested version series or exact release and platform variant.
    pub request: SdkRequest,
    /// Installation currently satisfying this request.
    pub installation: String,
    /// Prevents update from resolving a different build for this request.
    pub pinned: bool,
}

/// One consistent registry snapshot for command and IDE presentation.
#[derive(Clone, Debug, Serialize)]
pub struct SdkStatus {
    /// Completed managed and external installations.
    pub installations: Vec<Installation>,
    /// Persisted version requirements and update policies.
    pub selections: Vec<Selection>,
    /// Resolved defaults keyed by family/platform, or family for portable tools.
    pub defaults: BTreeMap<String, Installation>,
}

/// An independent default selection for a tool family and target platform.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct DefaultSelection {
    /// Requirement whose saved binding may advance on update.
    request: SdkRequest,
    /// Exact installation when selected by ID.
    id: Option<String>,
}

/// Persistent registry, atomically replaced as a single bounded CBOR value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Registry {
    /// Internal schema version, independent of the Janex container version.
    schema: u32,
    /// Completed installation records.
    pub(super) installations: Vec<Installation>,
    /// Persisted requests and their current resolutions.
    pub(super) selections: Vec<Selection>,
    /// Independent global defaults; resolving them never contacts the network.
    pub(super) defaults: BTreeMap<String, DefaultSelection>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            schema: 1,
            installations: Vec::new(),
            selections: Vec::new(),
            defaults: BTreeMap::new(),
        }
    }
}

/// Native SDK operations scoped to one absolute Janex home.
#[derive(Clone, Debug)]
pub struct SdkManager {
    /// Persistent root, shared with dependency caching but not owned by a cache cleaner.
    pub(super) root: PathBuf,
}

impl SdkManager {
    /// Uses `JANEX_HOME`, or the platform user's `.janex` directory, without creating it.
    pub fn user() -> Result<Self> {
        Self::new(janex_platform::janex_home()?)
    }

    /// Uses an explicit absolute home without reading or modifying process environment variables.
    pub fn new(root: PathBuf) -> Result<Self> {
        if !root.is_absolute() {
            return Err(invalid("SDK home must be an absolute path"));
        }
        Ok(Self { root })
    }

    /// Lists completed installations without creating state or contacting a server.
    pub fn list(&self) -> Result<Vec<Installation>> {
        Ok(self.read()?.installations)
    }

    /// Returns installations, requirements, and the default from the same registry snapshot.
    pub fn status(&self) -> Result<SdkStatus> {
        let state = self.read()?;
        let defaults = state
            .defaults
            .keys()
            .map(|family| {
                Ok((
                    family.clone(),
                    self.default_at_key(&state, family)?.unwrap(),
                ))
            })
            .collect::<Result<_>>()?;
        Ok(SdkStatus {
            installations: state.installations,
            selections: state.selections,
            defaults,
        })
    }

    /// Returns persisted requirements and their current installation bindings.
    pub fn selections(&self) -> Result<Vec<Selection>> {
        Ok(self.read()?.selections)
    }

    /// Changes whether update may move a saved requirement to a different installation.
    pub fn set_pin(&self, request: &SdkRequest, pinned: bool) -> Result<()> {
        request.validate()?;
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        let selection = state
            .selections
            .iter_mut()
            .find(|s| s.request == *request)
            .ok_or_else(|| invalid("no saved SDK requirement matches this pin request"))?;
        selection.pinned = pinned;
        self.save(&state)
    }

    /// Queries matching GA packages for the requested native or explicit platform variant.
    pub fn available(
        &self,
        request: &SdkRequest,
        options: &CatalogOptions,
    ) -> Result<Vec<AvailableSdk>> {
        catalog::available(&self.root, request, options)
    }

    /// Installs or reuses an SDK and records its requirement, retaining all previous versions.
    /// Does not change the default selection. A failed download or extraction publishes no record.
    pub fn install(
        &self,
        request: &SdkRequest,
        pin: bool,
        options: &CatalogOptions,
    ) -> Result<Installation> {
        request.validate()?;
        if options.offline && options.refresh {
            return Err(invalid("offline and refresh are mutually exclusive"));
        }
        if !options.refresh {
            let _lock = self.lock(false)?;
            let mut state = self.read()?;
            if let Some(index) = state.selections.iter().position(|s| s.request == *request) {
                let selected = state.selections[index].installation.clone();
                let installed = state
                    .installations
                    .iter()
                    .find(|i| i.id == selected)
                    .unwrap()
                    .clone();
                if !self
                    .home(&installed)?
                    .join("bin")
                    .join(request.executable())
                    .is_file()
                {
                    return Err(invalid(
                        "installed SDK is missing; uninstall and reinstall it",
                    ));
                }
                if pin && !state.selections[index].pinned {
                    drop(_lock);
                    let _lock = self.lock(true)?;
                    state = self.read()?;
                    if let Some(selection) = state
                        .selections
                        .iter_mut()
                        .find(|s| s.request == *request && s.installation == installed.id)
                    {
                        selection.pinned = true;
                        self.save(&state)?;
                    } else {
                        return Err(invalid(
                            "SDK selection changed concurrently; retry installation",
                        ));
                    }
                }
                return Ok(installed);
            }
        }
        if options.offline {
            let _lock = self.lock(true)?;
            let mut state = self.read()?;
            let installed = self.resolve_request(&state, request)?;
            if !self
                .home(&installed)?
                .join("bin")
                .join(request.executable())
                .is_file()
            {
                return Err(invalid("installed SDK home is missing"));
            }
            bind(&mut state, request.clone(), &installed.id, pin);
            self.save(&state)?;
            return Ok(installed);
        }
        let packages = self.available(request, options)?;
        let package = packages
            .first()
            .ok_or_else(|| invalid("no matching SDK archive is available"))?;
        let artifact = catalog::artifact(&self.root, package, options)?;
        let exact = request.with_version(&package.version);
        if artifact.checksum.algorithm() == Algorithm::Sha256 {
            let _lock = self.lock(true)?;
            let mut state = self.read()?;
            let digest = hex(artifact.checksum.digest());
            if let Some(existing) = state
                .installations
                .iter()
                .find(|i| i.sdk == exact && i.sha256.as_ref() == Some(&digest))
                .cloned()
            {
                if !self
                    .home(&existing)?
                    .join("bin")
                    .join(request.executable())
                    .is_file()
                {
                    return Err(invalid("installed SDK home is missing"));
                }
                bind(&mut state, request.clone(), &existing.id, pin);
                self.save(&state)?;
                return Ok(existing);
            }
        }
        fs::create_dir_all(self.root.join("tmp"))?;
        let stage = tempfile::Builder::new()
            .prefix("sdk-")
            .tempdir_in(self.root.join("tmp"))?;
        let download = stage.path().join(&package.filename);
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&download)?;
        catalog::download_archive(&artifact.url, &mut file, options)?;
        file.sync_all()?;
        let digest = archive::verify(&mut file, &artifact.checksum)?;
        self.commit_archive(
            file,
            &package.archive_type,
            exact,
            request,
            pin,
            &artifact.url,
            &digest,
            options.max_extracted_bytes,
            stage.path(),
        )
    }

    /// Extracts verified bytes, then publishes the tree and its registry reference under a lock.
    #[allow(clippy::too_many_arguments)]
    fn commit_archive(
        &self,
        file: fs::File,
        format: &str,
        exact: SdkRequest,
        requested: &SdkRequest,
        pin: bool,
        source: &str,
        digest: &str,
        limit: u64,
        staging: &Path,
    ) -> Result<Installation> {
        let identity = format!(
            "{}\n{digest}\n{}",
            serde_json::to_string(&exact).map_err(|e| invalid(e.to_string()))?,
            exact.default_key()
        );
        let id = hex(Checksum::compute(Algorithm::Sha256, identity.as_bytes())?.digest());
        let tree = staging.join("tree");
        fs::create_dir(&tree)?;
        archive::extract(file, format, &tree, limit)?;
        let home = archive::find_home(&tree, &exact)?
            .strip_prefix(&tree)
            .unwrap()
            .to_owned();
        let installation = Installation {
            id,
            java_version: if exact.java().is_some() {
                Some(archive::java_version(&tree.join(&home))?)
            } else {
                None
            },
            sdk: exact,
            home,
            managed: true,
            source: source.into(),
            sha256: Some(digest.into()),
        };
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        if let Some(existing) = state.installations.iter().find(|i| i.id == installation.id) {
            if !self
                .home(existing)?
                .join("bin")
                .join(existing.sdk.executable())
                .is_file()
            {
                return Err(invalid("existing SDK tree is missing"));
            }
        } else {
            let directory = self.root.join("sdks").join(installation.sdk.family());
            fs::create_dir_all(&directory)?;
            let destination = directory.join(&installation.id);
            if destination.exists() {
                return Err(invalid("unregistered SDK tree occupies the destination"));
            }
            // Publish the lease file before making the installation visible.
            self.create_lease(&installation.id)?;
            fs::rename(&tree, destination)?;
            state.installations.push(installation.clone());
        }
        bind(&mut state, requested.clone(), &installation.id, pin);
        self.save(&state)?;
        Ok(installation)
    }

    /// Re-resolves a saved requirement, retaining the old tree and respecting its pin.
    pub fn update(&self, request: &SdkRequest, options: &CatalogOptions) -> Result<Installation> {
        let state = self.read()?;
        let selected = state
            .selections
            .iter()
            .find(|s| s.request == *request)
            .ok_or_else(|| invalid("no installed SDK request matches this update"))?;
        if selected.pinned {
            return Ok(state
                .installations
                .into_iter()
                .find(|i| i.id == selected.installation)
                .unwrap());
        }
        let mut options = options.clone();
        options.refresh = true;
        self.install(request, false, &options)
    }

    /// Registers an existing SDK home without copying or owning its contents. Java registration
    /// reads release metadata; portable tools are identified from their core library and variant files.
    pub fn register(&self, request: &SdkRequest, path: &Path) -> Result<Installation> {
        request.validate()?;
        let path = path.canonicalize()?;
        let (home, sdk) = if let Some(java_request) = request.java() {
            let home = archive::find_home(&path, request)?;
            let mut java = java_request.clone();
            java.version = if java.descriptor()?.is_nik() {
                archive::nik_version(&home)?
            } else {
                archive::java_version(&home)?
            };
            java.validate()?;
            (home, java)
        } else {
            super::tools::external_home(&path, request)?
        };
        if !request.accepts(&sdk) {
            return Err(invalid(
                "external SDK does not establish the requested release or build",
            ));
        }
        let identity = serde_json::to_vec(&(&home, &sdk)).map_err(|e| invalid(e.to_string()))?;
        let id = hex(Checksum::compute(Algorithm::Sha256, identity.as_slice())?.digest());
        let installation = Installation {
            id,
            java_version: if sdk.java().is_some() {
                Some(archive::java_version(&home)?)
            } else {
                None
            },
            sdk,
            home,
            managed: false,
            source: path
                .to_str()
                .ok_or_else(|| invalid("external SDK path is not Unicode"))?
                .into(),
            sha256: None,
        };
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        self.create_lease(&installation.id)?;
        if !state.installations.iter().any(|i| i.id == installation.id) {
            state.installations.push(installation.clone());
        }
        bind(&mut state, request.clone(), &installation.id, true);
        self.save(&state)?;
        Ok(installation)
    }

    /// Selects an installed requirement or installation ID as the global default without downloading.
    pub fn set_default(&self, target: &str) -> Result<Installation> {
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        let (installation, request) = self.resolve_in(&state, target)?;
        state.defaults.insert(
            request.default_key(),
            DefaultSelection {
                request,
                id: (target == installation.id).then(|| installation.id.clone()),
            },
        );
        self.save(&state)?;
        Ok(installation)
    }

    /// Clears one family's global default while retaining all installed SDKs.
    pub fn clear_default(&self, family: &str) -> Result<()> {
        validate_family(family)?;
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        state
            .defaults
            .remove(&default_key(family, &super::SdkPlatform::native()?));
        self.save(&state)
    }

    /// Returns the global default installation, if configured, without inspecting project files.
    pub fn default_installation(&self, family: &str) -> Result<Option<Installation>> {
        validate_family(family)?;
        let state = self.read()?;
        self.default_in(&state, family)
    }

    /// Returns a default for an explicit target platform without downloading or applying emulation fallback.
    pub fn default_for(
        &self,
        family: &str,
        platform: &super::SdkPlatform,
    ) -> Result<Option<Installation>> {
        validate_family(family)?;
        platform.validate()?;
        self.default_at_key(&self.read()?, &default_key(family, platform))
    }

    /// Clears only the default for the requested family and target platform.
    pub fn clear_default_for(&self, family: &str, platform: &super::SdkPlatform) -> Result<()> {
        validate_family(family)?;
        platform.validate()?;
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        state.defaults.remove(&default_key(family, platform));
        self.save(&state)
    }

    /// Resolves the native-platform default, keeping portable tools independent of architecture.
    pub(super) fn default_in(
        &self,
        state: &Registry,
        family: &str,
    ) -> Result<Option<Installation>> {
        self.default_at_key(state, &default_key(family, &super::SdkPlatform::native()?))
    }

    /// Resolves an exact default ID or its movable request in one platform slot.
    fn default_at_key(&self, state: &Registry, key: &str) -> Result<Option<Installation>> {
        let Some(default) = state.defaults.get(key) else {
            return Ok(None);
        };
        if default.request.default_key() != key {
            return Err(invalid("default SDK target mismatch"));
        }
        if let Some(id) = &default.id {
            return state
                .installations
                .iter()
                .find(|i| i.id == *id && default.request.accepts(&i.sdk))
                .cloned()
                .map(Some)
                .ok_or_else(|| invalid("default SDK installation is missing"));
        }
        self.resolve_request(state, &default.request).map(Some)
    }

    /// Resolves an installed target. Multiple matching concrete builds require an exact ID for removal.
    pub fn resolve(&self, target: &str) -> Result<Installation> {
        Ok(self.resolve_in(&self.read()?, target)?.0)
    }

    /// Returns the absolute usable SDK home for an installation record.
    pub fn home(&self, installation: &Installation) -> Result<PathBuf> {
        validate_installation(installation)?;
        Ok(if installation.managed {
            self.root
                .join("sdks")
                .join(installation.sdk.family())
                .join(&installation.id)
                .join(&installation.home)
        } else {
            installation.home.clone()
        })
    }

    /// Removes one exact installation, rejecting the default or a live Janex lease.
    /// External registration removal never deletes the external directory. If tree cleanup fails,
    /// the record has already been removed and the unreferenced tree remains on disk.
    pub fn uninstall(&self, target: &str) -> Result<Installation> {
        let _lock = self.lock(true)?;
        let mut state = self.read()?;
        let installation = if let Some(i) = state.installations.iter().find(|i| i.id == target) {
            i.clone()
        } else {
            let request = SdkRequest::parse(target)?;
            if !request.version().contains('.') && !request.version().contains('+') {
                return Err(invalid(
                    "uninstall requires an exact version or installation ID",
                ));
            }
            let matches: Vec<_> = state
                .installations
                .iter()
                .filter(|i| {
                    matches_request(&i.sdk, &request)
                        && (request.java().is_some() || request.version() == i.sdk.version())
                })
                .collect();
            if matches.len() != 1 {
                return Err(invalid(
                    "uninstall target is missing or ambiguous; use the installation ID",
                ));
            }
            matches[0].clone()
        };
        if state.defaults.keys().any(|key| {
            self.default_at_key(&state, key)
                .ok()
                .flatten()
                .is_some_and(|d| d.id == installation.id)
        }) {
            return Err(invalid(
                "SDK is the global default; select another version or clear the default first",
            ));
        }
        let lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.lease_path(&installation.id))?;
        fs4::FileExt::try_lock(&lease)
            .map_err(|_| invalid("SDK is in use by another Janex operation"))?;
        state
            .selections
            .retain(|s| s.installation != installation.id);
        state.installations.retain(|i| i.id != installation.id);
        self.save(&state)?;
        if installation.managed {
            let parent = self
                .root
                .join("sdks")
                .join(installation.sdk.family())
                .canonicalize()?;
            let tree = parent.join(&installation.id);
            let metadata = fs::symlink_metadata(&tree)?;
            if metadata.file_type().is_symlink() || !tree.canonicalize()?.starts_with(&parent) {
                return Err(invalid("SDK tree is not confined to managed storage"));
            }
            fs::remove_dir_all(tree)?;
        }
        Ok(installation)
    }

    /// Resolves an exact ID or a Java requirement against one registry snapshot.
    pub(super) fn resolve_in(
        &self,
        state: &Registry,
        target: &str,
    ) -> Result<(Installation, SdkRequest)> {
        if let Some(i) = state.installations.iter().find(|i| i.id == target) {
            return Ok((i.clone(), i.sdk.clone()));
        }
        let request = SdkRequest::parse(target)?;
        Ok((self.resolve_request(state, &request)?, request))
    }

    /// Applies a stored binding first, then selects the newest matching installed build.
    pub(super) fn resolve_request(
        &self,
        state: &Registry,
        request: &SdkRequest,
    ) -> Result<Installation> {
        if let Some(selection) = state.selections.iter().find(|s| s.request == *request) {
            return Ok(state
                .installations
                .iter()
                .find(|i| i.id == selection.installation)
                .unwrap()
                .clone());
        }
        state
            .installations
            .iter()
            .filter(|i| matches_request(&i.sdk, request))
            .max_by(|a, b| a.sdk.compare(&b.sdk).then(a.id.cmp(&b.id)))
            .cloned()
            .ok_or_else(|| invalid(format!("SDK is not installed: {}", request.target())))
    }

    /// Reads and validates a bounded registry snapshot; missing state means an empty installation set.
    pub(super) fn read(&self) -> Result<Registry> {
        let file = match fs::File::open(self.root.join("state/sdks.cbor")) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        let value = Value::from_bytes(
            &bytes,
            Limits {
                max_bytes: 16 * 1024 * 1024,
                max_elements: 500_000,
                max_depth: 16,
            },
        )?;
        let state: Registry = serde_json::from_value(from_cbor(&value)?)
            .map_err(|e| invalid(format!("invalid SDK registry: {e}")))?;
        if state.schema != 1 {
            return Err(invalid("unsupported SDK registry schema"));
        }
        let mut ids = std::collections::BTreeSet::new();
        for i in &state.installations {
            validate_installation(i)?;
            if !ids.insert(&i.id) {
                return Err(invalid("duplicate SDK installation ID"));
            }
        }
        for (index, s) in state.selections.iter().enumerate() {
            s.request.validate()?;
            let installed = state
                .installations
                .iter()
                .find(|i| i.id == s.installation)
                .ok_or_else(|| invalid("dangling SDK selection"))?;
            if !matches_request(&installed.sdk, &s.request)
                || state.selections[..index]
                    .iter()
                    .any(|other| other.request == s.request)
            {
                return Err(invalid("invalid SDK selection binding"));
            }
        }
        for (family, default) in &state.defaults {
            default.request.validate()?;
            self.default_at_key(&state, family)?;
        }
        Ok(state)
    }

    /// Serializes internal state using the existing deterministic CBOR codec.
    fn save(&self, state: &Registry) -> Result<()> {
        let json = serde_json::to_value(state).map_err(|e| invalid(e.to_string()))?;
        let value = to_cbor(json)?;
        if value.as_bytes().len() > 16 * 1024 * 1024 {
            return Err(invalid("SDK registry size limit exceeded"));
        }
        publish(&self.root.join("state/sdks.cbor"), value.as_bytes())
    }

    /// Acquires the registry lock; read-only access to an unused home creates nothing.
    pub(super) fn lock(&self, exclusive: bool) -> Result<Option<fs::File>> {
        let path = self.root.join("state/sdk.lock");
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

    /// Returns the stable per-installation lease file; lock files are never removed or replaced.
    pub(super) fn lease_path(&self, id: &str) -> PathBuf {
        self.root
            .join("state/sdk-leases")
            .join(format!("{id}.lock"))
    }

    /// Creates a lock inode before publishing its installation.
    fn create_lease(&self, id: &str) -> Result<()> {
        let path = self.lease_path(id);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        Ok(())
    }

    /// Holds a shared lease until the returned file is dropped.
    pub(super) fn lease(&self, id: &str) -> Result<fs::File> {
        let file = fs::File::open(self.lease_path(id))?;
        fs4::FileExt::lock_shared(&file)?;
        Ok(file)
    }
}

/// Tests both the numeric requirement and every explicitly selected variant.
fn matches_request(actual: &SdkRequest, requested: &SdkRequest) -> bool {
    requested.accepts(actual)
}

/// Rejects unknown families before consulting or modifying defaults.
fn validate_family(family: &str) -> Result<()> {
    if matches!(family, "java" | "gradle" | "maven") {
        Ok(())
    } else {
        Err(invalid("unknown SDK family"))
    }
}

/// Inserts or replaces the resolution for one requirement without changing the global default.
fn bind(state: &mut Registry, request: SdkRequest, id: &str, pinned: bool) {
    state.selections.retain(|s| s.request != request);
    state.selections.push(Selection {
        request,
        installation: id.into(),
        pinned,
    });
}

/// Builds a family default key using a native or explicitly selected platform.
fn default_key(family: &str, platform: &super::SdkPlatform) -> String {
    if family == "java" {
        format!("java/{}", platform.key())
    } else {
        family.into()
    }
}

/// Validates path confinement before consuming registry data.
fn validate_installation(i: &Installation) -> Result<()> {
    i.sdk.validate()?;
    if let Some(java) = i.sdk.java() {
        super::numeric_version(&java.version)?;
    } else {
        super::request::tool_version(i.sdk.version())?;
    }
    if i.id.len() != 64
        || !i
            .id
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(invalid("invalid SDK installation ID"));
    }
    if i.managed {
        if !i.sha256.as_ref().is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        }) {
            return Err(invalid(
                "managed SDK requires an exact SHA-256 archive identity",
            ));
        }
        if i.home
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(invalid("invalid relative SDK home"));
        }
    } else if !i.home.is_absolute() {
        return Err(invalid("external SDK home must be absolute"));
    }
    Ok(())
}

/// Atomically replaces a metadata file after syncing its complete contents.
pub(super) fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| invalid("metadata path has no parent"))?;
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Converts the registry's JSON-compatible schema to deterministic CBOR with text keys.
fn to_cbor(value: serde_json::Value) -> Result<Value> {
    use serde_json::Value as Json;
    Ok(match value {
        Json::Null => Value::null(),
        Json::Bool(v) => Value::boolean(v),
        Json::String(v) => Value::text(&v),
        Json::Number(v) => Value::uint(
            v.as_u64()
                .ok_or_else(|| invalid("invalid registry number"))?,
        ),
        Json::Array(v) => Value::array(v.into_iter().map(to_cbor).collect::<Result<Vec<_>>>()?),
        Json::Object(v) => Value::map(
            v.into_iter()
                .map(|(k, v)| Ok((Value::text(&k), to_cbor(v)?)))
                .collect::<Result<Vec<_>>>()?,
        )?,
    })
}

/// Decodes only the CBOR types used by the internal registry schema.
fn from_cbor(value: &Value) -> Result<serde_json::Value> {
    use serde_json::Value as Json;
    Ok(match value.as_bytes()[0] >> 5 {
        0 => Json::from(value.as_u64()?),
        3 => Json::from(value.as_text()?),
        4 => Json::Array(
            value
                .as_array()?
                .iter()
                .map(from_cbor)
                .collect::<Result<_>>()?,
        ),
        5 => {
            let mut map = serde_json::Map::new();
            for (key, value) in value.as_map()? {
                if map
                    .insert(key.as_text()?.into(), from_cbor(&value)?)
                    .is_some()
                {
                    return Err(invalid("duplicate SDK registry key"));
                }
            }
            Json::Object(map)
        }
        _ if value.is_null() => Json::Null,
        _ => Json::Bool(value.as_bool()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Seek;
    use zip::{ZipWriter, write::SimpleFileOptions};

    /// Creates a small, structurally valid SDK archive without executable test code.
    fn fixture(
        manager: &SdkManager,
        requested: &SdkRequest,
        version: &str,
        pin: bool,
    ) -> Installation {
        fs::create_dir_all(manager.root.join("tmp")).unwrap();
        let stage = tempfile::tempdir_in(manager.root.join("tmp")).unwrap();
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(stage.path().join("jdk.zip"))
            .unwrap();
        let mut writer = ZipWriter::new(&mut file);
        let options = SimpleFileOptions::default();
        if requested.java().is_none() {
            let (directory, core) = if requested.family() == "gradle" {
                (
                    format!("gradle-{version}"),
                    format!("gradle-core-{version}.jar"),
                )
            } else {
                (
                    format!("apache-maven-{version}"),
                    format!("maven-core-{version}.jar"),
                )
            };
            if requested.family() == "gradle" && requested.variant == "all" {
                for path in [
                    format!("{directory}/docs/index.html"),
                    format!("{directory}/src/source.txt"),
                ] {
                    writer.start_file(path, options).unwrap();
                    writer.write_all(b"fixture").unwrap();
                }
            }
            for path in [
                format!("{directory}/bin/{}", requested.executable()),
                format!("{directory}/lib/{core}"),
            ] {
                writer.start_file(path, options).unwrap();
                writer.write_all(b"fixture").unwrap();
            }
        } else {
            writer.start_file("jdk/release", options).unwrap();
            write!(
                writer,
                "JAVA_VERSION=\"{}\"\nOS_ARCH=\"{}\"\n",
                if requested.java().unwrap().descriptor().unwrap().is_nik() {
                    "21.0.8"
                } else {
                    version.split('+').next().unwrap()
                },
                requested.platform.as_ref().unwrap().arch
            )
            .unwrap();
            for name in [
                requested.platform.as_ref().unwrap().executable("java"),
                requested.platform.as_ref().unwrap().executable("javac"),
            ] {
                writer
                    .start_file(format!("jdk/bin/{name}"), options)
                    .unwrap();
                writer.write_all(b"fixture").unwrap();
            }
        }
        writer.finish().unwrap();
        file.rewind().unwrap();
        let expected = Checksum::compute(Algorithm::Sha256, &mut file).unwrap();
        let digest = archive::verify(&mut file, &expected).unwrap();
        let exact = requested.with_version(version);
        manager
            .commit_archive(
                file,
                "zip",
                exact,
                requested,
                pin,
                "https://example.org/jdk.zip",
                &digest,
                1024 * 1024,
                stage.path(),
            )
            .unwrap()
    }

    #[test]
    fn versions_coexist_and_default_follows_its_request() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SdkManager::new(temp.path().join("home")).unwrap();
        assert!(manager.list().unwrap().is_empty());
        assert!(!manager.root.exists());
        let request = SdkRequest::parse("bellsoft/liberica-jdk@21").unwrap();
        let old = fixture(&manager, &request, "21.0.8+12", false);
        assert!(manager.default_installation("java").unwrap().is_none());
        manager.set_default("bellsoft/liberica-jdk@21").unwrap();
        let new = fixture(&manager, &request, "21.0.9+10", false);
        assert_eq!(manager.list().unwrap().len(), 2);
        assert_eq!(
            manager.default_installation("java").unwrap().unwrap().id,
            new.id
        );
        assert!(manager.home(&old).unwrap().exists());
        assert!(
            manager
                .uninstall(&new.id)
                .unwrap_err()
                .to_string()
                .contains("default")
        );
        assert!(manager.uninstall("bellsoft/liberica-jdk@21").is_err());
        manager.set_default(&old.id).unwrap();
        assert_eq!(
            manager.default_installation("java").unwrap().unwrap().id,
            old.id
        );
        manager.uninstall(&new.id).unwrap();
        assert!(manager.home(&old).unwrap().exists());
        manager.clear_default("java").unwrap();
        manager.uninstall(&old.id).unwrap();
        assert!(manager.list().unwrap().is_empty());
    }

    #[test]
    fn running_lease_blocks_removal_and_pin_prevents_network_updates() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SdkManager::new(temp.path().to_owned()).unwrap();
        let request = SdkRequest::parse("bellsoft/liberica-jdk@21").unwrap();
        let installed = fixture(&manager, &request, "21.0.8+12", true);
        let offline = CatalogOptions {
            offline: true,
            ..Default::default()
        };
        assert_eq!(
            manager.install(&request, false, &offline).unwrap().id,
            installed.id
        );
        assert_eq!(manager.update(&request, &offline).unwrap().id, installed.id);
        let execution = manager.execution(Some(&installed.id), None).unwrap();
        assert!(
            manager
                .uninstall(&installed.id)
                .unwrap_err()
                .to_string()
                .contains("in use")
        );
        drop(execution);
        manager.uninstall(&installed.id).unwrap();
    }

    #[test]
    fn external_unregister_preserves_files_and_project_pins_exact_id() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SdkManager::new(temp.path().join("home")).unwrap();
        let request = SdkRequest::parse("bellsoft/liberica-jdk@21").unwrap();
        let installed = fixture(&manager, &request, "21.0.8+12", true);
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let path = manager.use_project(&installed.id, &project, true).unwrap();
        assert_eq!(
            super::super::project_java(&project).unwrap(),
            Some(installed.id.clone())
        );
        assert!(fs::read_to_string(path).unwrap().contains(&installed.id));
        let mut state = manager.read().unwrap();
        state.installations[0].home = manager.home(&installed).unwrap();
        state.installations[0].managed = false;
        state.installations[0].sha256 = None;
        manager.save(&state).unwrap();
        manager.uninstall(&installed.id).unwrap();
        assert!(manager.home(&installed).unwrap().join("release").exists());
    }

    #[test]
    fn corrupt_or_escaping_registry_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SdkManager::new(temp.path().to_owned()).unwrap();
        let request = SdkRequest::parse("bellsoft/liberica-jdk@21").unwrap();
        fixture(&manager, &request, "21.0.8+12", true);
        let mut state = manager.read().unwrap();
        state.installations[0].home = "../outside".into();
        manager.save(&state).unwrap();
        assert!(manager.list().is_err());
        fs::write(manager.root.join("state/sdks.cbor"), [0xff]).unwrap();
        assert!(manager.list().is_err());
    }

    #[test]
    fn tool_defaults_updates_and_leases_are_independent() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SdkManager::new(temp.path().join("home")).unwrap();
        let java = fixture(
            &manager,
            &SdkRequest::parse("bellsoft/liberica-jdk@21").unwrap(),
            "21.0.8+12",
            true,
        );
        let gradle_request = SdkRequest::parse("gradle@8").unwrap();
        let old = fixture(&manager, &gradle_request, "8.14.2", false);
        let maven_request = SdkRequest::parse("maven@3.9").unwrap();
        let maven = fixture(&manager, &maven_request, "3.9.9", true);
        manager.set_default(&java.id).unwrap();
        manager.set_default("gradle@8").unwrap();
        manager.set_default("maven@3.9").unwrap();
        let new = fixture(&manager, &gradle_request, "8.14.3", false);
        let status = manager.status().unwrap();
        assert_eq!(status.defaults[&java.sdk.default_key()].id, java.id);
        assert_eq!(status.defaults["maven"].id, maven.id);
        assert_eq!(status.defaults["gradle"].id, new.id);
        assert_eq!(status.installations.len(), 4);
        assert!(manager.home(&old).unwrap().exists());
        assert!(manager.uninstall("maven@3.9").is_err());
        let offline = CatalogOptions {
            offline: true,
            ..Default::default()
        };
        assert_eq!(
            manager.update(&maven_request, &offline).unwrap().id,
            maven.id
        );
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        manager.use_project(&java.id, &project, true).unwrap();
        manager.use_project("gradle@8", &project, false).unwrap();
        let path = manager.use_project(&maven.id, &project, true).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(
            content.contains(&java.id)
                && content.contains(&maven.id)
                && content.contains("gradle@8")
        );
        let execution = manager
            .execution_with(
                Some(&java.id),
                Some(&new.id),
                Some(&maven.id),
                Some(&project),
            )
            .unwrap();
        assert_eq!(execution.homes().len(), 3);
        manager.clear_default("gradle").unwrap();
        assert!(
            manager
                .uninstall(&new.id)
                .unwrap_err()
                .to_string()
                .contains("in use")
        );
        assert!(manager.default_installation("java").unwrap().is_some());
        assert!(manager.default_installation("maven").unwrap().is_some());
        drop(execution);
        manager.uninstall(&new.id).unwrap();
        assert!(manager.home(&old).unwrap().is_dir());
    }
    #[test]
    fn platform_defaults_updates_and_project_selectors_remain_independent() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SdkManager::new(temp.path().join("home")).unwrap();
        let x64 = SdkRequest::parse("bellsoft/liberica-jdk@21[arch=x86-64,variant=full]").unwrap();
        let arm = SdkRequest::parse("bellsoft/liberica-jdk@21[arch=aarch64,variant=full]").unwrap();
        let standard = SdkRequest::parse("bellsoft/liberica-jdk@21[arch=x86-64]").unwrap();
        let old_x64 = fixture(&manager, &x64, "21.0.8+12", false);
        let old_arm = fixture(&manager, &arm, "21.0.8+12", false);
        let plain = fixture(&manager, &standard, "21.0.8+12", false);
        assert_ne!(old_x64.id, old_arm.id);
        assert_ne!(old_x64.id, plain.id);
        manager.set_default(&x64.target()).unwrap();
        manager.set_default(&arm.target()).unwrap();
        manager.set_pin(&arm, true).unwrap();
        let new_x64 = fixture(&manager, &x64, "21.0.9+10", false);
        assert_eq!(
            manager
                .default_for("java", x64.platform.as_ref().unwrap())
                .unwrap()
                .unwrap()
                .id,
            new_x64.id
        );
        assert_eq!(
            manager
                .default_for("java", arm.platform.as_ref().unwrap())
                .unwrap()
                .unwrap()
                .id,
            old_arm.id
        );
        assert_eq!(
            manager
                .update(
                    &arm,
                    &CatalogOptions {
                        offline: true,
                        ..Default::default()
                    }
                )
                .unwrap()
                .id,
            old_arm.id
        );
        assert_eq!(manager.list().unwrap().len(), 4);
        assert!(manager.uninstall(&old_arm.id).is_err());
        assert!(manager.uninstall(&new_x64.id).is_err());
        manager
            .clear_default_for("java", arm.platform.as_ref().unwrap())
            .unwrap();
        manager.uninstall(&old_arm.id).unwrap();
        assert_eq!(manager.resolve(&x64.target()).unwrap().id, new_x64.id);
        assert!(manager.resolve(&arm.target()).is_err());
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        manager
            .use_project(
                "bellsoft/liberica-jdk@21[arch=x86-64,variant=full]",
                &project,
                false,
            )
            .unwrap();
        let text = fs::read_to_string(project.join(".janex-toolchains.toml")).unwrap();
        assert!(text.contains("arch=x86-64,variant=full"));
        assert!(!text.contains("os="));
        let native = SdkRequest::parse("bellsoft/liberica-jdk@21").unwrap();
        fixture(&manager, &native, "21.0.9+10", false);
        manager
            .use_project("bellsoft/liberica-jdk@21", &project, false)
            .unwrap();
        let text = fs::read_to_string(project.join(".janex-toolchains.toml")).unwrap();
        assert_eq!(text.trim(), "java = \"bellsoft/liberica-jdk@21\"");
    }

    #[test]
    fn foreign_platform_archives_can_be_managed_without_execution() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SdkManager::new(temp.path().join("home")).unwrap();
        let os = if cfg!(windows) { "linux" } else { "windows" };
        let request =
            SdkRequest::parse(&format!("bellsoft/liberica-jdk@21[os={os},arch=aarch64]")).unwrap();
        let installed = fixture(&manager, &request, "21.0.8+12", false);
        assert_eq!(installed.java_version.as_deref(), Some("21.0.8"));
        assert!(
            manager
                .home(&installed)
                .unwrap()
                .join("bin")
                .join(request.executable())
                .is_file()
        );
        manager.set_default(&request.target()).unwrap();
        assert!(manager.default_installation("java").unwrap().is_none());
        assert!(request.check_host().is_err());
    }
    #[test]
    fn nik_installations_keep_product_and_runtime_versions_separate() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SdkManager::new(temp.path().join("home")).unwrap();
        let request = SdkRequest::parse("bellsoft/liberica-nik@24").unwrap();
        let installed = fixture(&manager, &request, "24.0.2+1", false);
        assert_eq!(installed.sdk.version(), "24.0.2+1");
        assert_eq!(installed.java_version.as_deref(), Some("21.0.8"));
        assert_eq!(
            manager.resolve("bellsoft/liberica-nik@24").unwrap().id,
            installed.id
        );
        assert!(manager.resolve("bellsoft/liberica-jdk@21").is_err());
        assert!(manager.resolve("bellsoft/liberica-nik@21").is_err());
    }
    #[test]
    fn gradle_variants_coexist_and_keep_independent_update_bindings() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SdkManager::new(temp.path().join("home")).unwrap();
        let bin = SdkRequest::parse("gradle@9").unwrap();
        let all = SdkRequest::parse("gradle@9[variant=all]").unwrap();
        let first_bin = fixture(&manager, &bin, "9.0.0", false);
        let first_all = fixture(&manager, &all, "9.0.0", false);
        assert_ne!(first_bin.id, first_all.id);
        assert_eq!(manager.resolve("gradle@9").unwrap().id, first_bin.id);
        manager.set_default("gradle@9[variant=all]").unwrap();
        manager.set_pin(&all, true).unwrap();
        let next_bin = fixture(&manager, &bin, "9.1.0", false);
        assert_eq!(
            manager.default_installation("gradle").unwrap().unwrap().id,
            first_all.id
        );
        assert_eq!(
            manager
                .update(
                    &all,
                    &CatalogOptions {
                        offline: true,
                        ..Default::default()
                    }
                )
                .unwrap()
                .id,
            first_all.id
        );
        assert_eq!(manager.resolve("gradle@9").unwrap().id, next_bin.id);
        manager.set_pin(&all, false).unwrap();
        let next_all = fixture(&manager, &all, "9.1.0", false);
        assert_eq!(
            manager.default_installation("gradle").unwrap().unwrap().id,
            next_all.id
        );
        assert_eq!(manager.list().unwrap().len(), 4);
        assert_eq!(manager.selections().unwrap().len(), 2);
        assert_eq!(
            manager.uninstall("gradle@9.0.0[variant=all]").unwrap().id,
            first_all.id
        );
        assert!(manager.home(&first_bin).unwrap().exists());
        assert_eq!(
            manager.resolve("gradle@9[variant=all]").unwrap().id,
            next_all.id
        );
    }
}

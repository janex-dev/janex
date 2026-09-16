// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Local input packaging with shared blobs, lossless transforms, and atomic output.

use crate::{
    Result,
    authentication::{CmsSigner, OpenPgpSigner},
    error::invalid,
    import::{ImportOptions, ImportedRoot, import_path},
};
use janex_format::{
    application::{Application, EntryPoint, PathEntry},
    blob::{BlobRef, PoolBuilder},
    cbor::Value,
    checksum::{Algorithm, Checksum},
    classfile,
    container::{APPLICATION, BLOB_POOL, Writer},
    content::{Content, Source, Transform},
    data_pool::DataPoolBuilder,
    resource::{Directory, DirectoryEntry, Layer, ResourceRoot},
    version::JavaRange,
};
use janex_signature::cms;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

/// Local inputs and Java launch data for one new application package.
#[derive(Clone, Debug)]
pub struct PackOptions {
    /// Primary directory or JAR, placed first on the selected Java path.
    pub source: PathBuf,
    /// Destination file, which must not already exist.
    pub output: PathBuf,
    /// Additional classpath inputs in order.
    pub class_path: Vec<PathBuf>,
    /// Additional module-path inputs in order.
    pub module_path: Vec<PathBuf>,
    /// External declarations appended after the local classpath inputs, without downloading them.
    /// Entries must be `PathEntry::External`.
    pub external_class_path: Vec<PathEntry>,
    /// External declarations appended after local module-path inputs, including virtual requirements.
    /// Entries must be `PathEntry::External`.
    pub external_module_path: Vec<PathEntry>,
    /// Explicit binary main-class name, overriding inferred names.
    pub main_class: Option<String>,
    /// Main module; when present, the primary input is placed on the module path.
    pub main_module: Option<String>,
    /// File-local application ID, defaulting to `main`.
    pub application: String,
    /// Complete JVM arguments in order, without shell splitting.
    pub jvm_options: Vec<String>,
    /// Preset program arguments, including empty strings.
    pub arguments: Vec<String>,
    /// Optional `vers:jep322` runtime constraint.
    pub java_version: Option<String>,
    /// Import bounds; the aggregate byte limit also applies across all inputs.
    pub import: ImportOptions,
    /// Zstandard level for blobs and tables, defaulting to 3.
    pub compression_level: i32,
    /// Compare a CLASSFILE-transformed candidate and retain it only when the complete file is smaller.
    pub transform_classfiles: bool,
    /// Append the portable Java launcher as a JAR tail for `java -jar` execution.
    pub with_launcher: bool,
    /// Native PE or ELF launcher to prepend with an application and trust policy.
    pub native_launcher: Option<PathBuf>,
    /// Invocation strategy recorded in the native launcher; defaults to bootstrap.
    pub native_launch_mode: crate::run::LaunchMode,
    /// Optional publisher signer; absent uses Checksum verification.
    pub signer: Option<PackSigner>,
}

/// Selects the single publisher-signature mechanism used by a package.
#[derive(Clone, Debug)]
pub enum PackSigner {
    /// One CMS signer certificate and its matching private key.
    Cms(Arc<CmsSigner>),
    /// One certified, unlocked OpenPGP primary key or signing subkey.
    OpenPgp(Arc<OpenPgpSigner>),
}

impl PackOptions {
    /// Creates options for a classpath package with default limits and compression.
    pub fn new(source: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            output: output.into(),
            class_path: Vec::new(),
            module_path: Vec::new(),
            external_class_path: Vec::new(),
            external_module_path: Vec::new(),
            main_class: None,
            main_module: None,
            application: "main".into(),
            jvm_options: Vec::new(),
            arguments: Vec::new(),
            java_version: None,
            import: ImportOptions::default(),
            compression_level: 3,
            transform_classfiles: true,
            with_launcher: false,
            native_launcher: None,
            native_launch_mode: crate::run::LaunchMode::Bootstrap,
            signer: None,
        }
    }
}

/// Statistics for a completely written package.
#[derive(Clone, Copy, Debug)]
pub struct PackReport {
    /// Number of imported roots, including the primary input.
    pub resource_roots: usize,
    /// Complete output file length in bytes.
    pub file_bytes: u64,
    /// Number of file entries using a CLASSFILE transform, including shared references.
    pub classfiles_transformed: usize,
}

/// Packages local inputs into a new Janex 0.1 file with SHA-256 integrity coverage.
///
/// Inputs are imported before creating output files and must remain unchanged during import.
/// Existing destinations, including symbolic links, are rejected. Temporary files use the
/// destination directory; the selected result is synchronized before publication without
/// replacement. Failures before publication leave no destination file.
///
/// Each input has its own root and pool. Identical file bytes within a root share a blob;
/// file entries use XXH3-64 checksums; checksums alone never establish equality.
/// The smaller complete container wins when
/// comparing transformed and ordinary class files. Ties retain ordinary class files.
pub fn pack(options: &PackOptions) -> Result<PackReport> {
    if options.with_launcher && options.signer.is_some() {
        return Err(invalid(
            "the standalone Java launcher does not yet support signed packages",
        ));
    }
    match fs::symlink_metadata(&options.output) {
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "output already exists",
            )
            .into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if options.application.is_empty() {
        return Err(invalid("application ID must not be empty"));
    }
    if let Some(version) = &options.java_version {
        JavaRange::parse(version)?;
    }
    let count = 1usize
        .checked_add(options.class_path.len())
        .and_then(|count| count.checked_add(options.module_path.len()))
        .ok_or_else(|| invalid("input count overflow"))?;
    options.import.limits.elements(count as u64)?;
    let mut inputs = Vec::new();
    let mut remaining = options.import.max_total_bytes;
    for path in std::iter::once(&options.source)
        .chain(&options.class_path)
        .chain(&options.module_path)
    {
        let mut import = options.import;
        import.max_total_bytes = remaining;
        let input = import_path(path, import)?;
        for directory in input.layers.iter().flat_map(|layer| &layer.directories) {
            for entry in &directory.entries {
                if let DirectoryEntry::File { content, .. } = entry
                    && let Source::Inline(bytes) = &content.source
                {
                    remaining = remaining
                        .checked_sub(bytes.len() as u64)
                        .ok_or_else(|| invalid("aggregate package input limit exceeded"))?;
                }
            }
        }
        inputs.push(input);
    }
    let entry_point = infer_entry_point(&inputs[0], options)?;
    let header = options
        .native_launcher
        .as_deref()
        .map(|path| {
            crate::native_launcher::header(
                path,
                &options.application,
                options.native_launch_mode,
                options.signer.as_ref(),
            )
        })
        .transpose()?
        .unwrap_or_default();
    let parent = options
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut selected = tempfile::NamedTempFile::new_in(parent)?;
    let mut transformed = write_package(
        selected.as_file_mut(),
        &inputs,
        options,
        &entry_point,
        &header,
        false,
    )?;
    let mut size = selected.as_file().metadata()?.len();
    if options.transform_classfiles && has_classfiles(&inputs) {
        let mut candidate = tempfile::NamedTempFile::new_in(parent)?;
        let count = write_package(
            candidate.as_file_mut(),
            &inputs,
            options,
            &entry_point,
            &header,
            true,
        )?;
        let candidate_size = candidate.as_file().metadata()?.len();
        if candidate_size < size {
            selected = candidate;
            transformed = count;
            size = candidate_size;
        }
    }
    #[cfg(unix)]
    if options.native_launcher.is_some() {
        use std::os::unix::fs::PermissionsExt;
        selected
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    selected.as_file().sync_all()?;
    selected
        .persist_noclobber(&options.output)
        .map_err(|error| error.error)?;
    Ok(PackReport {
        resource_roots: inputs.len(),
        file_bytes: size,
        classfiles_transformed: transformed,
    })
}

/// Determines the entry point from explicit options, a main manifest, or a module descriptor.
fn infer_entry_point(input: &ImportedRoot, options: &PackOptions) -> Result<EntryPoint> {
    let mut main_class = options.main_class.clone().or_else(|| {
        input
            .manifest
            .as_ref()
            .and_then(|manifest| manifest.get("Main-Class"))
            .map(str::to_owned)
    });
    if main_class.is_none() && options.main_module.is_none() {
        for directory in input
            .layers
            .iter()
            .flat_map(|layer| &layer.directories)
            .filter(|directory| directory.path.is_empty())
        {
            for entry in &directory.entries {
                if let DirectoryEntry::File { name, content, .. } = entry
                    && name == "module-info.class"
                {
                    let bytes = imported_bytes(content)?;
                    if let Some(module) = classfile::inspect(bytes, options.import.limits)?.module
                        && let Some(name) = module.main_class
                    {
                        if main_class
                            .as_ref()
                            .is_some_and(|previous| previous != &name)
                        {
                            return Err(invalid(
                                "module main class varies by resource layer; supply --main-class or --main-module",
                            ));
                        }
                        main_class = Some(name);
                    }
                }
            }
        }
    }
    let entry = EntryPoint {
        main_class,
        main_module: options.main_module.clone(),
    };
    if entry.main_class.is_none() && entry.main_module.is_none() {
        return Err(invalid(
            "cannot determine entry point; supply --main-class or --main-module",
        ));
    }
    entry.to_value()?;
    Ok(entry)
}

/// Writes one complete candidate and returns its transformed file-entry count.
fn write_package(
    mut output: impl Write,
    inputs: &[ImportedRoot],
    options: &PackOptions,
    entry: &EntryPoint,
    header: &[u8],
    transform: bool,
) -> Result<usize> {
    output.write_all(header)?;
    let mut writer = Writer::new(output)?;
    let mut roots = Vec::new();
    let mut transformed = 0;
    for (index, input) in inputs.iter().enumerate() {
        let pool_id = index as u64 + 1;
        let (pool, root, count) = build_pool(input, pool_id, options, transform)?;
        transformed += count;
        roots.push(root);
        writer.write_section(pool_id, BLOB_POOL, &pool.bytes, Some(pool.type_info))?;
    }
    let mut class_path = Vec::new();
    let mut module_path = Vec::new();
    if options.main_module.is_some() {
        module_path.push(local(roots[0])?);
    } else {
        class_path.push(local(roots[0])?);
    }
    for root in &roots[1..1 + options.class_path.len()] {
        class_path.push(local(*root)?);
    }
    for root in &roots[1 + options.class_path.len()..] {
        module_path.push(local(*root)?);
    }
    for (entries, output, modular) in [
        (&options.external_class_path, &mut class_path, false),
        (&options.external_module_path, &mut module_path, true),
    ] {
        for entry in entries {
            if !matches!(entry, PathEntry::External { .. }) {
                return Err(invalid(
                    "external path declarations must use external references",
                ));
            }
            output.push(entry.to_value(modular)?);
        }
    }
    let mut launch = vec![
        (Value::uint(1), entry.to_value()?),
        (Value::uint(2), Value::array(module_path)),
        (Value::uint(3), Value::array(class_path)),
        (
            Value::uint(5),
            Value::array(
                options
                    .jvm_options
                    .iter()
                    .map(|argument| Value::text(argument)),
            ),
        ),
        (
            Value::uint(7),
            Value::array(
                options
                    .arguments
                    .iter()
                    .map(|argument| Value::text(argument)),
            ),
        ),
    ];
    if let Some(version) = &options.java_version {
        launch.push((
            Value::uint(0),
            Value::map([(
                Value::uint(5),
                Value::map([
                    (Value::uint(0), Value::text("janex.java")),
                    (
                        Value::uint(1),
                        Value::map([(Value::uint(0), Value::text(version))])?,
                    ),
                ])?,
            )])?,
        ));
    }
    let application = Application::from_values(
        Value::map([
            (Value::uint(0), Value::text(&options.application)),
            (Value::uint(1), Value::text("janex.java")),
        ])?,
        Value::map([(
            Value::uint(0),
            Value::map([(Value::uint(0), Value::map(launch)?)])?,
        )])?,
        options.import.limits,
    )?;
    writer.write_section(
        0,
        APPLICATION,
        &application.encode()?,
        Some(application.type_info().clone()),
    )?;
    let empty_region = Value::map([(Value::uint(0), Value::uint(0))])?;
    let tail: &[u8] = if options.with_launcher {
        include_bytes!("../../janex-bootstrap/build/libs/janex-bootstrap.jar")
    } else {
        &[]
    };
    let tail_region = if tail.is_empty() {
        empty_region.clone()
    } else {
        Value::map([
            (Value::uint(0), Value::uint(tail.len() as u64)),
            (
                Value::uint(1),
                Value::bytes(&Checksum::compute(Algorithm::Sha256, tail)?.encode()),
            ),
        ])?
    };
    let metadata = Value::map([
        (
            Value::uint(1),
            if header.is_empty() {
                empty_region.clone()
            } else {
                Value::map([
                    (Value::uint(0), Value::uint(header.len() as u64)),
                    (
                        Value::uint(1),
                        Value::bytes(&Checksum::compute(Algorithm::Sha256, header)?.encode()),
                    ),
                ])?
            },
        ),
        (Value::uint(2), tail_region),
    ])?;
    match &options.signer {
        Some(PackSigner::Cms(signer)) => {
            writer.finish_with(metadata, 3, |input| {
                Ok::<_, crate::Error>(cms::sign(input, &[signer], SystemTime::now())?)
            })?;
        }
        Some(PackSigner::OpenPgp(signer)) => {
            writer.finish_with(metadata, 2, |input| {
                Ok::<_, crate::Error>(signer.sign(input, SystemTime::now())?)
            })?;
        }
        None => {
            let mut output = writer.finish(metadata)?;
            output.write_all(tail)?;
        }
    }
    Ok(transformed)
}

/// Tracks a content digest's exact original bytes and reusable content descriptor.
struct SharedFile<'a> {
    /// Borrowed input bytes used to distinguish even digest collisions.
    bytes: &'a [u8],
    /// Previously written blob and its optional transform.
    content: Content,
}

/// Builds one resource pool, keeping every file reference and string index stable.
fn build_pool(
    input: &ImportedRoot,
    pool_id: u64,
    options: &PackOptions,
    transform: bool,
) -> Result<(janex_format::blob::BuiltPool, BlobRef, usize)> {
    let mut pool = PoolBuilder::new();
    let mut strings = DataPoolBuilder::new();
    let mut shared = BTreeMap::<Vec<u8>, Vec<SharedFile<'_>>>::new();
    let mut layers = Vec::new();
    let mut transformed = 0;
    for layer in &input.layers {
        let mut directories = Vec::new();
        for directory in &layer.directories {
            let mut entries = Vec::new();
            for entry in &directory.entries {
                if let DirectoryEntry::File {
                    name,
                    content,
                    metadata,
                } = entry
                {
                    let bytes = imported_bytes(content)?;
                    let digest = Checksum::compute(Algorithm::Xxh3_64, bytes)?;
                    let content = if bytes.is_empty() {
                        Content::inline(Vec::new())
                    } else if let Some(previous) = shared
                        .get(digest.digest())
                        .and_then(|files| files.iter().find(|file| file.bytes == bytes))
                    {
                        previous.content.clone()
                    } else {
                        let encoded = if transform
                            && name.ends_with(".class")
                            && bytes.starts_with(b"\xca\xfe\xba\xbe")
                        {
                            match classfile::transform(bytes, &mut strings, options.import.limits) {
                                Ok(value) => value,
                                Err(error)
                                    if matches!(
                                        error.kind(),
                                        janex_format::ErrorKind::Invalid
                                            | janex_format::ErrorKind::Unsupported
                                    ) =>
                                {
                                    None
                                }
                                Err(error) => return Err(error.into()),
                            }
                        } else {
                            None
                        };
                        let mut content = Content::blob(BlobRef {
                            pool: pool_id,
                            index: pool.push(
                                encoded.as_deref().unwrap_or(bytes),
                                options.compression_level,
                            )?,
                        });
                        if encoded.is_some() {
                            content.transforms.push(Transform {
                                input_size: bytes.len() as u64,
                                method: 1,
                                properties: Value::empty_map(),
                            });
                        }
                        shared
                            .entry(digest.digest().to_vec())
                            .or_default()
                            .push(SharedFile {
                                bytes,
                                content: content.clone(),
                            });
                        content
                    };
                    transformed += usize::from(!content.transforms.is_empty());
                    let mut fields = metadata.as_map()?;
                    fields.retain(|(key, _)| key.as_u64().ok() != Some(0));
                    fields.push((Value::uint(0), Value::bytes(&digest.encode())));
                    entries.push(DirectoryEntry::File {
                        name: name.clone(),
                        content,
                        metadata: Value::map(fields)?,
                    });
                } else {
                    entries.push(entry.clone());
                }
            }
            directories.push(Directory {
                path: directory.path.clone(),
                metadata: directory.metadata.clone(),
                entries,
            });
        }
        layers.push(Layer {
            condition: layer.condition.clone(),
            directories,
        });
    }
    let data_pool = BlobRef {
        pool: pool_id,
        index: pool.len() as u64,
    };
    let mut root = ResourceRoot {
        data_pool,
        data: strings.finish(),
        layers,
        metadata: Value::map([(
            Value::text("janex.java.jar_name"),
            Value::text(&input.jar_name),
        )])?,
    };
    let root_bytes = root.encode(options.import.limits)?;
    let strings = root.data.encode()?;
    options.import.limits.bytes(strings.len() as u64)?;
    pool.push(&strings, options.compression_level)?;
    let root = BlobRef {
        pool: pool_id,
        index: pool.push(&root_bytes, options.compression_level)?,
    };
    Ok((
        pool.finish(8, options.compression_level)?,
        root,
        transformed,
    ))
}

/// Borrows bytes from the untransformed inline representation produced by import.
fn imported_bytes(content: &Content) -> Result<&[u8]> {
    match &content.source {
        Source::Inline(bytes) if content.transforms.is_empty() => Ok(bytes),
        _ => Err(invalid("imported file is not untransformed inline content")),
    }
}

/// Encodes a local Java path entry.
fn local(root: BlobRef) -> Result<Value> {
    Ok(PathEntry::Local(root).to_value(false)?)
}

/// Returns whether a transform candidate can differ from ordinary packaging.
fn has_classfiles(inputs: &[ImportedRoot]) -> bool {
    inputs
        .iter()
        .flat_map(|input| &input.layers)
        .flat_map(|layer| &layer.directories)
        .flat_map(|directory| &directory.entries)
        .any(|entry| matches!(entry, DirectoryEntry::File { name, .. } if name.ends_with(".class")))
}

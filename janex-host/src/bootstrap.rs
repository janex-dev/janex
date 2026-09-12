// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Host-selected classpath and module resources referencing one authenticated snapshot.

use crate::{Result, adapters::java_limits, error::invalid};
use janex_format::{
    application::PathEntry,
    blob::{BlobRef, BlobStore, Entry},
    condition::Context,
    content::{Content, Source},
    resource::{Node, ResourceRoot, ResourceTree},
    strings::StringPool,
};
use janex_java::manifest::Manifest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Cursor,
    path::Path,
};

/// Prepared index and the logical resource allowance it consumes.
pub(crate) struct Resources {
    /// Private index embedded in the bootstrap JAR.
    pub(crate) data: Vec<u8>,
    /// Aggregate logical bytes across selected classpath and module roots.
    pub(crate) logical_bytes: u64,
}

/// Builds private launch data; ordinary resource payloads remain in the snapshot.
pub(crate) fn prepare(
    entries: &[PathEntry],
    modules: &[PathEntry],
    context: &Context,
    blobs: &mut BlobStore<Cursor<Vec<u8>>>,
    roots: &mut BTreeMap<BlobRef, ResourceRoot>,
    directory: &Path,
    max_bytes: u64,
) -> Result<Resources> {
    let snapshot = directory.join("snapshot.janex");
    fs::write(&snapshot, blobs.reader().get_ref().get_ref())?;
    let mut offset = blobs.reader().range().start + 8;
    let mut sections = BTreeMap::new();
    for section in blobs.reader().sections() {
        sections.insert(section.id(), offset + 8);
        offset += section.length();
    }
    let mut builder = Builder {
        blobs,
        sections,
        sources: Vec::new(),
        ids: BTreeMap::new(),
        sizes: Vec::new(),
        pools: Vec::new(),
        pool_ids: BTreeMap::new(),
        remaining: max_bytes,
        index_bytes: 0,
    };
    let mut root_data = Vec::new();
    let requirements: Vec<_> = modules
        .iter()
        .filter_map(PathEntry::module_requirement)
        .collect();
    number(&mut root_data, requirements.len() as u64)?;
    for (name, version) in requirements {
        string(&mut root_data, &name)?;
        string(&mut root_data, version.as_deref().unwrap_or(""))?;
    }
    let selected: Vec<_> = entries
        .iter()
        .map(|entry| (entry, false))
        .chain(
            modules
                .iter()
                .filter(|entry| entry.module_requirement().is_none())
                .map(|entry| (entry, true)),
        )
        .collect();
    number(&mut root_data, selected.len() as u64)?;
    for (entry, module) in selected {
        let PathEntry::Local(reference) = entry else {
            return Err(invalid("bootstrap paths require resolved local resources"));
        };
        if !roots.contains_key(reference) {
            let bytes = builder.blobs.resolve(*reference)?;
            roots.insert(*reference, ResourceRoot::decode(&bytes, builder.blobs)?);
        }
        let root = &roots[reference];
        string(&mut root_data, root.jar_name()?)?;
        root_data.push(u8::from(module));
        let tree = root.merge(context, builder.blobs.reader().limits())?;
        let mut children = BTreeMap::<&str, Vec<&str>>::new();
        for (path, _) in tree.entries().filter(|(p, _)| !p.is_empty()) {
            children
                .entry(path.rsplit_once('/').map_or("", |(p, _)| p))
                .or_default()
                .push(path);
        }
        let mut files = vec![(String::new(), String::new())];
        expand(
            &tree,
            &children,
            "",
            "",
            &mut BTreeSet::new(),
            &mut files,
            builder.blobs.reader().limits(),
        )?;
        number(&mut root_data, files.len() as u64)?;
        for (name, canonical) in files {
            string(&mut root_data, &name)?;
            let node = tree.get(&canonical).expect("expanded node exists");
            match node {
                Node::Directory(_) => root_data.extend_from_slice(&(-1i32).to_be_bytes()),
                Node::File {
                    content,
                    metadata: _,
                } => {
                    if name.eq_ignore_ascii_case("META-INF/MANIFEST.MF") {
                        let data = tree.read_file(&canonical, builder.blobs)?;
                        let data =
                            Manifest::parse(&data, java_limits(builder.blobs.reader().limits()))?
                                .for_runtime();
                        builder.file(&Content::inline(data), root, &mut root_data)?;
                    } else {
                        builder.file(content, root, &mut root_data)?;
                    }
                }
                Node::SymbolicLink { .. } => unreachable!("expanded symbolic link"),
            }
            let empty_metadata = janex_format::cbor::Value::empty_map();
            let metadata = match node {
                Node::Directory(metadata) => metadata.unwrap_or(&empty_metadata),
                Node::File { metadata, .. } => metadata,
                Node::SymbolicLink { .. } => unreachable!(),
            };
            let mut flags = 0u8;
            for key in 2..=5 {
                if metadata.get(key)?.is_some() {
                    flags |= 1 << (key - 2);
                }
            }
            root_data.push(flags);
            for key in 2..=4 {
                if let Some(value) = metadata.get(key)? {
                    root_data.extend(value.as_i128()?.to_be_bytes());
                }
            }
            if let Some(mode) = metadata.get(5)? {
                number(&mut root_data, mode.as_u64()?)?;
            }
            builder
                .blobs
                .reader()
                .limits()
                .bytes(builder.index_bytes + root_data.len() as u64)?;
        }
    }
    let limits = builder.blobs.reader().limits();
    let mut output = b"JNXRES01".to_vec();
    number(&mut output, limits.max_bytes.min(i32::MAX as u64))?;
    number(&mut output, limits.max_elements.min(i32::MAX as u64))?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        units(&mut output, snapshot.as_os_str().encode_wide())?;
    }
    #[cfg(not(windows))]
    string(
        &mut output,
        snapshot
            .to_str()
            .ok_or_else(|| invalid("bootstrap snapshot path must be Unicode"))?,
    )?;
    number(&mut output, builder.sources.len() as u64)?;
    for source in builder.sources {
        output.extend(source);
    }
    number(&mut output, builder.pools.len() as u64)?;
    for pool in builder.pools {
        output.extend(pool);
    }
    output.extend(root_data);
    limits.bytes(output.len() as u64)?;
    Ok(Resources {
        data: output,
        logical_bytes: max_bytes - builder.remaining,
    })
}

/// Accumulates bounded, topologically ordered source descriptors and shared string pools.
struct Builder<'a> {
    /// Source reader over the verified snapshot.
    blobs: &'a mut BlobStore<Cursor<Vec<u8>>>,
    /// Absolute pool payload offsets in that snapshot.
    sections: BTreeMap<u64, u64>,
    /// Serialized sources; extents refer only to earlier sources.
    sources: Vec<Vec<u8>>,
    /// Shared blob source identities.
    ids: BTreeMap<BlobRef, usize>,
    /// Decoded source lengths.
    sizes: Vec<u64>,
    /// Serialized string pools shared by transforms.
    pools: Vec<Vec<u8>>,
    /// Pool identities independent of the referring root.
    pool_ids: BTreeMap<BlobRef, usize>,
    /// Serialized source and pool bytes retained so far.
    index_bytes: u64,
    /// Remaining aggregate expanded logical byte allowance.
    remaining: u64,
}

impl Builder<'_> {
    /// Registers a source descriptor after checking collection and byte limits.
    fn add(&mut self, data: Vec<u8>, size: u64) -> Result<usize> {
        let limits = self.blobs.reader().limits();
        limits.bytes(size)?;
        self.index_bytes += data.len() as u64;
        limits.bytes(self.index_bytes)?;
        limits.elements(self.sources.len() as u64 + 1)?;
        let id = self.sources.len();
        self.sources.push(data);
        self.sizes.push(size);
        Ok(id)
    }

    /// Stores inline or Host-decoded bytes in the private index.
    fn inline(&mut self, bytes: &[u8]) -> Result<usize> {
        let mut data = vec![0];
        number(&mut data, bytes.len() as u64)?;
        data.extend_from_slice(bytes);
        self.add(data, bytes.len() as u64)
    }

    /// Describes a blob without decoding ordinary independently compressed payloads.
    fn blob(&mut self, reference: BlobRef) -> Result<usize> {
        if let Some(id) = self.ids.get(&reference) {
            return Ok(*id);
        }
        let id = match self.blobs.entry(reference)? {
            Entry::Stored { offset, encoding } => {
                let mut dictionary = false;
                for filter in &encoding.filters {
                    dictionary |= filter.properties.get(0)?.is_some();
                }
                if dictionary {
                    // The portable decoder does not implement external dictionaries.
                    let bytes = self.blobs.resolve(reference)?;
                    self.inline(&bytes)?
                } else {
                    let mut data = vec![1];
                    data.extend_from_slice(
                        &(self.sections[&reference.pool] + offset).to_be_bytes(),
                    );
                    number(&mut data, encoding.stored_size)?;
                    number(&mut data, encoding.filters.len() as u64)?;
                    for filter in encoding.filters.iter().rev() {
                        number(&mut data, filter.input_size)?;
                    }
                    self.add(data, encoding.decoded_size())?
                }
            }
            Entry::Extents(extents) => {
                let mut data = vec![2];
                number(&mut data, extents.len() as u64)?;
                let mut size = 0u64;
                for extent in extents {
                    let source = BlobRef {
                        pool: reference.pool,
                        index: extent.stored_blob_index,
                    };
                    if !matches!(self.blobs.entry(source)?, Entry::Stored { .. }) {
                        return Err(invalid("extent source must be a Stored entry"));
                    }
                    let id = self.blob(source)?;
                    if extent
                        .decoded_offset
                        .checked_add(extent.decoded_length)
                        .is_none_or(|end| end > self.sizes[id])
                    {
                        return Err(invalid("extent exceeds decoded source"));
                    }
                    size = size
                        .checked_add(extent.decoded_length)
                        .ok_or_else(|| invalid("extent length overflow"))?;
                    number(&mut data, id as u64)?;
                    number(&mut data, extent.decoded_offset)?;
                    number(&mut data, extent.decoded_length)?;
                }
                self.add(data, size)?
            }
            Entry::Unknown(_) => return Err(invalid("unsupported blob entry")),
        };
        self.ids.insert(reference, id);
        Ok(id)
    }

    /// Registers the selected transform pool once, including explicit file overrides.
    fn pool(&mut self, reference: BlobRef, root: &ResourceRoot) -> Result<usize> {
        if let Some(id) = self.pool_ids.get(&reference) {
            return Ok(*id);
        }
        let explicit;
        let pool = if reference == root.string_pool {
            &root.strings
        } else {
            explicit = StringPool::decode(
                &self.blobs.resolve(reference)?,
                self.blobs.reader().limits(),
            )?;
            &explicit
        };
        let mut data = Vec::new();
        number(&mut data, pool.len() as u64)?;
        for i in 0..pool.len() {
            string(&mut data, pool.get(i as u64)?)?;
        }
        self.index_bytes += data.len() as u64;
        self.blobs.reader().limits().bytes(self.index_bytes)?;
        let id = self.pools.len();
        self.pools.push(data);
        self.pool_ids.insert(reference, id);
        Ok(id)
    }

    /// Serializes logical transforms and charges their final size to the launch budget.
    fn file(&mut self, content: &Content, root: &ResourceRoot, output: &mut Vec<u8>) -> Result<()> {
        let id = match &content.source {
            Source::Inline(bytes) => self.inline(bytes)?,
            Source::Blob(reference) => self.blob(*reference)?,
        };
        let size = content
            .transforms
            .first()
            .map_or(self.sizes[id], |t| t.input_size);
        self.remaining = self
            .remaining
            .checked_sub(size)
            .ok_or_else(|| invalid("materialized resource byte limit exceeded"))?;
        number(output, id as u64)?;
        number(output, content.transforms.len() as u64)?;
        for transform in content.transforms.iter().rev() {
            let reference = transform
                .properties
                .get(0)?
                .map(|v| BlobRef::from_value(&v))
                .transpose()?
                .unwrap_or(root.string_pool);
            number(output, transform.input_size)?;
            number(output, self.pool(reference, root)? as u64)?;
        }
        Ok(())
    }
}

/// Expands directory links while retaining classpath names and rejecting cycles.
fn expand(
    tree: &ResourceTree<'_>,
    children: &BTreeMap<&str, Vec<&str>>,
    source: &str,
    output: &str,
    active: &mut BTreeSet<String>,
    files: &mut Vec<(String, String)>,
    limits: janex_format::binary::Limits,
) -> Result<()> {
    if active.len() > limits.max_depth || !active.insert(source.into()) {
        return Err(invalid("symbolic-link directory cycle or expansion limit"));
    }
    for path in children.get(source).into_iter().flatten() {
        let name = path.rsplit('/').next().expect("child name");
        let name = if output.is_empty() {
            name.to_owned()
        } else {
            format!("{output}/{name}")
        };
        limits.bytes(name.len() as u64)?;
        let (canonical, node) = tree.resolve(path)?;
        if matches!(node, Node::File { .. }) && crate::materialize::jar_signature(&name) {
            continue;
        }
        let directory = matches!(node, Node::Directory(_));
        files.push((
            if directory {
                format!("{name}/")
            } else {
                name.clone()
            },
            canonical.into(),
        ));
        limits.elements(files.len() as u64)?;
        if directory {
            expand(tree, children, canonical, &name, active, files, limits)?;
        }
    }
    active.remove(source);
    Ok(())
}

/// Writes a nonnegative Java integer.
fn number(output: &mut Vec<u8>, value: u64) -> Result<()> {
    output.extend_from_slice(
        &i32::try_from(value)
            .map_err(|_| invalid("bootstrap index exceeds Java size limit"))?
            .to_be_bytes(),
    );
    Ok(())
}
/// Writes a string without charset-dependent conversion.
fn string(output: &mut Vec<u8>, text: &str) -> Result<()> {
    units(output, text.encode_utf16())
}
/// Writes exact UTF-16 units, including native Windows path surrogates.
fn units(output: &mut Vec<u8>, units: impl Iterator<Item = u16> + Clone) -> Result<()> {
    number(output, units.clone().count() as u64)?;
    for unit in units {
        output.extend_from_slice(&unit.to_be_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use janex_format::{
        binary::{Limits, write_sized, write_vuint},
        blob::{Encoding, Filter},
        cbor::Value,
        classfile,
        condition::Condition,
        container::{BLOB_POOL, Reader, Writer},
        content::Transform,
        resource::{Directory, DirectoryEntry, Layer},
    };
    use janex_java::{
        launch::{EntryPoint, LaunchMode, LaunchRequest},
        runtime::{JavaOptions, JavaRuntime, candidates},
    };
    use std::{io::Write, process::Command};

    /// Creates a regular entry with no optional checksum.
    fn file(name: &str, content: Content) -> DirectoryEntry {
        DirectoryEntry::File {
            name: name.into(),
            content,
            metadata: Value::empty_map(),
        }
    }

    /// Appends a manually described stored blob, independently of PoolBuilder.
    fn stored(data: &mut Vec<u8>, page: &mut Vec<u8>, bytes: &[u8], filters: Vec<Filter>) {
        let mut payload = Vec::new();
        write_vuint(&mut payload, data.len() as u64).unwrap();
        Encoding {
            stored_size: bytes.len() as u64,
            filters,
        }
        .write(&mut payload)
        .unwrap();
        page.push(0);
        write_sized(page, &payload).unwrap();
        data.extend(bytes);
    }

    #[test]
    fn indexes_extents_dictionary_links_and_file_pool_overrides_without_eager_payload_reads() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Main.java"),
            r#"
import java.io.*;
public class Main {
    static byte[] read(String name) throws Exception {
        try (InputStream input = Main.class.getClassLoader().getResource(name).openStream();
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[512]; int n;
            while ((n = input.read(buffer)) != -1) output.write(buffer, 0, n);
            return output.toByteArray();
        }
    }
    public static void main(String[] args) throws Exception {
        java.nio.file.Files.write(java.nio.file.Paths.get(args[0]), read("Main.class"));
        System.out.println(new String(read("extent.txt"), "UTF-8"));
        System.out.println(new String(read("alias/data.txt"), "UTF-8"));
        java.nio.file.Path folder = java.nio.file.Paths.get(Main.class.getClassLoader().getResource("alias/").toURI());
        java.util.Map<String, Object> attributes = java.nio.file.Files.readAttributes(folder, "janex:*");
        if (!new java.math.BigInteger("-170141183460469231731687303715884105728").equals(attributes.get("creationTimeNanos"))) throw new AssertionError();
        if (!new java.math.BigInteger("170141183460469231731687303715884105727").equals(attributes.get("lastAccessTimeNanos"))) throw new AssertionError();
        if (!Integer.valueOf(493).equals(attributes.get("permissions"))) throw new AssertionError();
        if (((java.nio.file.attribute.FileTime) attributes.get("lastModifiedTime")).to(java.util.concurrent.TimeUnit.NANOSECONDS) != 1234567890123L) throw new AssertionError();
        try { read("unused.txt"); throw new AssertionError("invalid Zstd accepted"); }
        catch (IOException expected) { System.out.println("lazy-error"); }
    }
}
"#,
        )
        .unwrap();
        let compiled = Command::new("javac")
            .current_dir(temp.path())
            .args(["--release", "8", "Main.java"])
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let original = fs::read(temp.path().join("Main.class")).unwrap();
        let mut strings = StringPool::new();
        let transformed = classfile::transform(&original, &mut strings, Limits::default())
            .unwrap()
            .unwrap();
        let dictionary = b"a shared dictionary containing a repeated resource message";
        let plain = b"a repeated resource message";
        let mut encoder =
            zstd::stream::Encoder::with_dictionary(Vec::new(), 3, dictionary).unwrap();
        encoder.write_all(plain).unwrap();
        let compressed = encoder.finish().unwrap();
        let reference = |index| BlobRef { pool: 1, index };
        let mut data = Vec::new();
        let mut page = Vec::new();
        stored(&mut data, &mut page, dictionary, vec![]); // 0
        stored(&mut data, &mut page, &strings.encode().unwrap(), vec![]); // 1
        stored(&mut data, &mut page, &transformed, vec![]); // 2
        stored(
            &mut data,
            &mut page,
            &compressed,
            vec![Filter {
                input_size: plain.len() as u64,
                method: 1,
                properties: Value::map([(Value::uint(0), reference(0).to_value())]).unwrap(),
            }],
        ); // 3
        let mut extent = Vec::new();
        for value in [2, 0, 0, 1, 0, 1, 7] {
            write_vuint(&mut extent, value).unwrap();
        }
        page.push(1);
        write_sized(&mut page, &extent).unwrap(); // 4
        stored(
            &mut data,
            &mut page,
            b"not a Zstd frame",
            vec![Filter {
                input_size: 10,
                method: 1,
                properties: Value::empty_map(),
            }],
        ); // 5: a valid descriptor with deliberately invalid, unused payload bytes.
        let info = Value::map([
            (Value::uint(0), Value::uint(6)),
            (Value::uint(1), Value::uint(8)),
            (
                Value::uint(2),
                Value::array([Value::array([
                    Value::uint(data.len() as u64),
                    Encoding {
                        stored_size: page.len() as u64,
                        filters: vec![],
                    }
                    .to_value()
                    .unwrap(),
                ])]),
            ),
        ])
        .unwrap();
        let mut section = b"BLOBPOOL".to_vec();
        section.extend(data);
        section.extend(page);
        let mut writer = Writer::new(Vec::new()).unwrap();
        writer
            .write_section(1, BLOB_POOL, &section, Some(info))
            .unwrap();
        let empty = Value::map([(Value::uint(0), Value::uint(0))]).unwrap();
        let bytes = writer
            .finish(Value::map([(Value::uint(1), empty.clone()), (Value::uint(2), empty)]).unwrap())
            .unwrap();
        let mut reader = Reader::open_auto(Cursor::new(bytes), Limits::default()).unwrap();
        assert!(reader.verify_checksums().unwrap().complete_secure_coverage);
        let mut blobs = BlobStore::new(reader);
        let mut entries = vec![
            file(
                "Main.class",
                Content {
                    source: Source::Blob(reference(2)),
                    transforms: vec![Transform {
                        input_size: original.len() as u64,
                        method: 1,
                        properties: Value::map([(Value::uint(0), reference(1).to_value())])
                            .unwrap(),
                    }],
                },
            ),
            DirectoryEntry::SymbolicLink {
                name: "alias".into(),
                target: "folder".into(),
                metadata: Value::empty_map(),
            },
            file("extent.txt", Content::blob(reference(4))),
            file("unused.txt", Content::blob(reference(5))),
        ];
        entries.sort_by(|a, b| a.name().cmp(b.name()));
        let root = ResourceRoot {
            string_pool: reference(99),
            strings: StringPool::new(),
            metadata: Value::empty_map(),
            layers: vec![Layer {
                condition: Condition::unconditional(),
                directories: vec![
                    Directory {
                        path: "".into(),
                        metadata: Value::empty_map(),
                        entries,
                    },
                    Directory {
                        path: "folder".into(),
                        metadata: Value::map([
                            (Value::uint(2), Value::integer(i128::MIN)),
                            (Value::uint(3), Value::integer(1234567890123)),
                            (Value::uint(4), Value::integer(i128::MAX)),
                            (Value::uint(5), Value::uint(0o755)),
                        ])
                        .unwrap(),
                        entries: vec![file("data.txt", Content::blob(reference(3)))],
                    },
                ],
            }],
        };
        let mut roots = BTreeMap::from([(reference(98), root)]);
        let context = Context {
            os: "linux".into(),
            arch: "x86-64".into(),
            invocation: None,
            runtime: None,
        };
        let launch = temp.path().join("launch");
        fs::create_dir(&launch).unwrap();
        let resources = prepare(
            &[PathEntry::Local(reference(98))],
            &[],
            &context,
            &mut blobs,
            &mut roots,
            &launch,
            1024 * 1024,
        )
        .unwrap();
        let runtime = JavaRuntime::probe(
            &candidates(&JavaOptions {
                java: Some("java".into()),
                java_home: None,
            })
            .unwrap()[0],
        )
        .unwrap();
        let restored = temp.path().join("restored.class");
        let request = LaunchRequest {
            entry_point: EntryPoint {
                main_class: Some("Main".into()),
                main_module: None,
            },
            mode: LaunchMode::Bootstrap,
            jvm_options: vec![],
            class_path: vec![],
            module_path: vec![],
            agents: vec![],
            arguments: vec![restored.as_os_str().into()],
        };
        let arguments = request
            .prepare_with_resources(
                &runtime,
                &launch,
                java_limits(Limits::default()),
                Some(&resources.data),
            )
            .unwrap();
        let output = Command::new(&runtime.executable)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout)
                .unwrap()
                .replace("\r\n", "\n"),
            "a shared\na repeated resource message\nlazy-error\n"
        );
        assert_eq!(fs::read(restored).unwrap(), original);
    }
}

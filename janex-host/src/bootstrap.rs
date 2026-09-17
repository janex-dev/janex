// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Compact resource-root handoff over verified files without launch-time copies.

use crate::{Result, error::invalid};
use janex_format::{application::PathEntry, binary::Limits, condition::Context};
use std::path::Path;

/// Root references interpreted in the application JVM.
pub(crate) struct Resources {
    /// Compact private handoff; contains no expanded resource index.
    pub(crate) data: Vec<u8>,
}

/// Encodes original paths without reading file contents or expanding resource roots.
pub(crate) fn prepare(
    entries: &[PathEntry],
    modules: &[PathEntry],
    context: &Context,
    limits: Limits,
    roots: &crate::roots::Roots,
    source: &Path,
    max_bytes: u64,
) -> Result<Resources> {
    let mut output = b"JNXROOT2".to_vec();
    number(&mut output, limits.max_bytes.min(i32::MAX as u64))?;
    number(&mut output, limits.max_elements.min(i32::MAX as u64))?;
    number(&mut output, limits.max_depth as u64)?;
    output.extend(max_bytes.to_be_bytes());
    path(&mut output, source)?;
    string(&mut output, &context.os)?;
    string(&mut output, &context.arch)?;
    string(&mut output, context.invocation.as_deref().unwrap_or(""))?;
    let runtime = context.runtime.as_ref();
    let version = runtime
        .and_then(|runtime| runtime.java_version.as_ref())
        .map(|version| version.to_string());
    string(&mut output, version.as_deref().unwrap_or("8"))?;
    string(
        &mut output,
        runtime.map_or("", |runtime| runtime.vendor.as_str()),
    )?;
    let requirements: Vec<_> = modules
        .iter()
        .filter_map(PathEntry::module_requirement)
        .collect();
    number(&mut output, requirements.len() as u64)?;
    for (name, version) in requirements {
        string(&mut output, &name)?;
        string(&mut output, version.as_deref().unwrap_or(""))?;
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
    number(&mut output, selected.len() as u64)?;
    for (entry, module) in selected {
        output.push(u8::from(module));
        match entry {
            PathEntry::Local(reference) => {
                output.push(0);
                output.extend(reference.pool.to_be_bytes());
                output.extend(reference.index.to_be_bytes());
            }
            PathEntry::External { .. } => {
                let key = crate::roots::RootKey::of(entry);
                let archive = roots
                    .archives
                    .get(&key)
                    .ok_or_else(|| invalid("external dependency has not been acquired"))?;
                output.push(1);
                string(&mut output, &archive.jar_name)?;
                path(&mut output, &archive.path)?;
            }
        }
        limits.bytes(output.len() as u64)?;
    }
    Ok(Resources { data: output })
}

/// Encodes a path without losing Windows UTF-16 code units.
fn path(output: &mut Vec<u8>, value: &Path) -> Result<()> {
    let value = janex_java::runtime::java_path(value);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        units(output, value.as_os_str().encode_wide())
    }
    #[cfg(not(windows))]
    string(
        output,
        value
            .to_str()
            .ok_or_else(|| invalid("bootstrap paths must be Unicode"))?,
    )
}

/// Encodes a nonnegative Java integer count.
fn number(output: &mut Vec<u8>, value: u64) -> Result<()> {
    output.extend(identifier(value)?.to_be_bytes());
    Ok(())
}

/// Bounds identities and counts to Java's array-index range.
fn identifier(value: u64) -> Result<u32> {
    i32::try_from(value)
        .map(|value| value as u32)
        .map_err(|_| invalid("handoff count exceeds Java range"))
}

/// Encodes Unicode text as counted UTF-16 units.
fn string(output: &mut Vec<u8>, text: &str) -> Result<()> {
    units(output, text.encode_utf16())
}

/// Encodes exact counted UTF-16 units.
fn units(output: &mut Vec<u8>, units: impl Iterator<Item = u16> + Clone) -> Result<()> {
    number(output, units.clone().count() as u64)?;
    for unit in units {
        output.extend(unit.to_be_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::java_limits;
    use janex_format::{
        binary::{Limits, write_sized, write_vuint},
        blob::{BlobStore, Encoding, Filter},
        cbor::Value,
        classfile,
        condition::Condition,
        container::{BLOB_POOL, Reader, Writer},
        content::Transform,
        resource::{Directory, DirectoryEntry, Layer},
    };
    use janex_format::{
        blob::BlobRef,
        content::{Content, Source},
        data_pool::DataPool,
        resource::ResourceRoot,
    };
    use janex_java::{
        launch::{EntryPoint, LaunchMode, LaunchRequest},
        runtime::{JavaOptions, JavaRuntime, candidates},
    };
    use std::{
        fs,
        io::{Cursor, Write},
        process::Command,
    };

    /// Checks source and pool identities at the Java integer boundary without allocation.
    #[test]
    fn identifiers_fit_nonnegative_java_integers() {
        assert_eq!(identifier(0).unwrap(), 0);
        assert_eq!(identifier(i32::MAX as u64).unwrap(), i32::MAX as u32);
        for value in [i32::MAX as u64 + 1, u32::MAX as u64, u64::MAX] {
            assert!(identifier(value).is_err());
        }
    }

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
        if (!"selected".equals(new String(read("context.txt"), "UTF-8"))) throw new AssertionError("Host context lost");
        System.out.println(new String(read("extent.txt"), "UTF-8"));
        System.out.println(new String(read("alias/data.txt"), "UTF-8"));
        java.nio.file.Path folder = java.nio.file.Paths.get(Main.class.getClassLoader().getResource("alias/").toURI());
        java.util.Map<String, Object> attributes = java.nio.file.Files.readAttributes(folder, "janex:*");
        if (!java.time.Instant.MIN.equals(attributes.get("creationTimeInstant"))) throw new AssertionError();
        if (!java.time.Instant.MAX.equals(attributes.get("lastAccessTimeInstant"))) throw new AssertionError();
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
        let mut strings = janex_format::data_pool::DataPoolBuilder::new();
        let transformed = classfile::transform(&original, &mut strings, Limits::default())
            .unwrap()
            .unwrap();
        strings.intern(b"\xff\xc0\x80\xed\xa0\x80");
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
        let mut root = ResourceRoot {
            data_pool: reference(6),
            data: DataPool::new(),
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
                            (
                                Value::uint(2),
                                Value::integer(janex_format::resource::MIN_TIMESTAMP_NANOS),
                            ),
                            (Value::uint(3), Value::integer(1234567890123)),
                            (
                                Value::uint(4),
                                Value::integer(janex_format::resource::MAX_TIMESTAMP_NANOS),
                            ),
                            (Value::uint(5), Value::uint(0o755)),
                        ])
                        .unwrap(),
                        entries: vec![file("data.txt", Content::blob(reference(3)))],
                    },
                ],
            }],
        };
        root.layers.push(Layer {
            condition: Condition::from_value(
                Value::map([
                    (Value::uint(1), Value::text("linux")),
                    (Value::uint(2), Value::text("aarch64")),
                    (Value::uint(4), Value::text("open")),
                ])
                .unwrap(),
            )
            .unwrap(),
            directories: vec![Directory {
                path: String::new(),
                metadata: Value::empty_map(),
                entries: vec![file("context.txt", Content::inline(b"selected".to_vec()))],
            }],
        });
        let encoded_root = root.encode(Limits::default()).unwrap();
        stored(&mut data, &mut page, &root.data.encode().unwrap(), vec![]);
        stored(&mut data, &mut page, &encoded_root, vec![]);
        let info = Value::map([
            (Value::uint(0), Value::uint(8)),
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
        let blobs = BlobStore::new(reader);
        let roots = crate::roots::Roots::default();
        let context = Context {
            os: "linux".into(),
            arch: "aarch64".into(),
            invocation: Some("open".into()),
            runtime: None,
        };
        let launch = temp.path().join("launch");
        fs::create_dir(&launch).unwrap();
        let source = temp.path().join("source.janex");
        fs::write(&source, blobs.reader().get_ref().get_ref()).unwrap();
        let resources = prepare(
            &[PathEntry::Local(reference(7))],
            &[],
            &context,
            blobs.reader().limits(),
            &roots,
            &source,
            1024 * 1024,
        )
        .unwrap();
        assert!(resources.data.len() < 2048);
        assert!(
            roots.entries.is_empty(),
            "resource roots must remain unexpanded in the Host"
        );
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

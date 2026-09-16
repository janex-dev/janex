// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Read-only container inspection using the format library's public parsing APIs.

use clap::Args;
use janex_format::{
    application::read_applications,
    binary::Limits,
    blob::{BlobRef, BlobStore, Encoding, Entry},
    cbor::Value,
    checksum::Checksum,
    container::{self, Reader, Verification},
    content::{Content, Source},
    resource::{DirectoryEntry, ResourceRoot},
};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, Write},
    path::PathBuf,
};

/// Selects optional structural detail without evaluating a launch environment.
#[derive(Args)]
pub(crate) struct InspectArgs {
    /// Local Janex file, including a native prefix or appended JAR launcher.
    file: PathBuf,
    /// Show section ranges, types, and recorded metadata.
    #[arg(long)]
    sections: bool,
    /// Show every pool, table page, and logical blob entry without decoding file payloads.
    #[arg(long)]
    blobs: bool,
    /// Show all declared local resource roots and layers, including inactive branches.
    #[arg(long)]
    resources: bool,
    /// Include root and explicit CLASSFILE data pools; implies --resources.
    #[arg(long)]
    data_pools: bool,
    /// Emit a versioned JSON document with exact decimal strings for 64-bit values.
    #[arg(long)]
    json: bool,
    /// Verify recorded container checksums; does not authenticate signatures or check every file.
    #[arg(long)]
    verify: bool,
    /// Explicit trailing byte length when automatic standalone/JAR detection is unsuitable.
    #[arg(long, value_name = "BYTES")]
    external_tail_length: Option<u64>,
}

/// Encodes bytes without loss, locale dependence, or terminal control sequences.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}

/// Projects CBOR without losing integer precision, key types, or unsupported extension values.
fn cbor(value: &Value) -> janex_format::Result<Json> {
    Ok(match value.as_bytes()[0] >> 5 {
        0 | 1 => json!({"integer": value.as_i128()?.to_string()}),
        2 => json!({"bytes_hex": hex(value.as_byte_string()?)}),
        3 => json!(value.as_text()?),
        4 => Json::Array(
            value
                .as_array()?
                .iter()
                .map(cbor)
                .collect::<janex_format::Result<_>>()?,
        ),
        5 => json!({"map": value.as_map()?.iter().map(|(key, value)| {
            Ok(json!([cbor(key)?, cbor(value)?]))
        }).collect::<janex_format::Result<Vec<_>>>()?}),
        _ if value.is_null() => Json::Null,
        _ if matches!(value.as_bytes()[0], 0xf4 | 0xf5) => json!(value.as_bool()?),
        _ => json!({"cbor_hex": hex(value.as_bytes())}),
    })
}

/// Describes a recorded digest without implying that it has been checked.
fn checksum(value: Option<&Checksum>) -> Json {
    value.map_or(Json::Null, |value| {
        json!({
            "algorithm": format!("{:?}", value.algorithm()),
            "algorithm_id": value.algorithm() as u8,
            "digest_hex": hex(value.digest()),
        })
    })
}

/// Uses decimal strings for opaque unsigned identifiers, safe for JavaScript consumers.
fn reference(value: BlobRef) -> Json {
    json!({"pool": value.pool.to_string(), "index": value.index.to_string()})
}

/// Describes stored and decoded lengths and preserves all filter properties.
fn encoding(value: &Encoding) -> janex_format::Result<Json> {
    Ok(json!({
        "stored_size": value.stored_size.to_string(),
        "decoded_size": value.decoded_size().to_string(),
        "filters": value.filters.iter().map(|filter| Ok(json!({
            "method": filter.method,
            "input_size": filter.input_size.to_string(),
            "properties": cbor(&filter.properties)?,
        }))).collect::<janex_format::Result<Vec<_>>>()?,
    }))
}

/// Describes a file's source and transforms without reading or restoring its data.
fn content(value: &Content) -> janex_format::Result<Json> {
    Ok(json!({
        "source": match &value.source {
            Source::Inline(bytes) => json!({"kind": "inline", "size": bytes.len().to_string()}),
            Source::Blob(blob) => json!({"kind": "blob", "reference": reference(*blob)}),
        },
        "transforms": value.transforms.iter().map(|transform| Ok(json!({
            "method": transform.method,
            "input_size": transform.input_size.to_string(),
            "properties": cbor(&transform.properties)?,
        }))).collect::<janex_format::Result<Vec<_>>>()?,
    }))
}

/// Projects every unmerged directory and layer, preserving tombstones, links, and metadata.
fn resource(root: &ResourceRoot, blob: BlobRef) -> janex_format::Result<Json> {
    let mut layers = Vec::new();
    for layer in &root.layers {
        let mut directories = Vec::new();
        for directory in &layer.directories {
            let mut entries = Vec::new();
            for entry in &directory.entries {
                entries.push(match entry {
                    DirectoryEntry::File { name, content: value, metadata } => json!({
                        "kind": "file", "name": name, "content": content(value)?, "metadata": cbor(metadata)?,
                    }),
                    DirectoryEntry::SymbolicLink { name, target, metadata } => json!({
                        "kind": "symbolic_link", "name": name, "target": target, "metadata": cbor(metadata)?,
                    }),
                    DirectoryEntry::Tombstone { name } => json!({"kind": "tombstone", "name": name}),
                });
            }
            directories.push(json!({"path": directory.path, "metadata": cbor(&directory.metadata)?, "entries": entries}));
        }
        layers
            .push(json!({"condition": cbor(layer.condition.value())?, "directories": directories}));
    }
    Ok(json!({
        "reference": reference(blob), "name": root.jar_name()?, "data_pool": reference(root.data_pool),
        "metadata": cbor(&root.metadata)?, "layers": layers,
    }))
}

/// Inspects a local file; stdout is written only after requested parsing and verification succeed.
pub(crate) fn run(args: InspectArgs) -> janex_host::Result<i32> {
    let file = File::open(&args.file)?;
    let physical_size = file.metadata()?.len();
    let limits = Limits::default();
    let mut reader = if let Some(length) = args.external_tail_length {
        Reader::open(file, length, limits)?
    } else {
        Reader::open_auto(file, limits)?
    };
    let range = reader.range();
    let verification_type = match reader.verification() {
        Verification::None => "none",
        Verification::Checksum(_) => "checksum",
        Verification::Cms(_) => "cms",
        Verification::OpenPgp(_) => "openpgp",
    };
    let integrity = if args.verify {
        let result = reader.verify_checksums()?;
        json!({"status": "passed", "content_checksums_verified": result.checksums_verified,
            "complete_secure_coverage": result.complete_secure_coverage})
    } else {
        json!({"status": "not_checked"})
    };
    let sections: Vec<_> = reader.sections().cloned().collect();
    let applications = read_applications(&mut reader)?;
    let application_sections: Vec<_> = sections
        .iter()
        .filter(|section| section.kind() == container::APPLICATION)
        .collect();
    let mut report = json!({
        "schema_version": 1, "format_version": "0.1",
        "physical_size": physical_size.to_string(),
        "janex_range": {"offset": range.start.to_string(), "length": (range.end - range.start).to_string()},
        "external_prefix_size": range.start.to_string(),
        "external_tail_size": (physical_size - range.end).to_string(),
        "section_count": sections.len(),
        "blob_pool_count": sections.iter().filter(|section| section.kind() == container::BLOB_POOL).count(),
        "verification": {"type": verification_type, "container_checksums": integrity,
            "signature": if matches!(verification_type, "cms" | "openpgp") { "not_checked" } else { "not_present" }},
        "metadata": cbor(reader.metadata())?,
        "applications": applications.iter().zip(&application_sections).map(|(application, section)| Ok(json!({
            "id": application.id(), "type": application.application_type(), "section_id": section.id().to_string(),
            "metadata": cbor(application.value())?, "type_info": cbor(application.type_info())?,
            "resource_roots": application.resource_references().into_iter().map(reference).collect::<Vec<_>>(),
        }))).collect::<janex_format::Result<Vec<_>>>()?,
    });
    if args.sections {
        let mut values = Vec::new();
        for section in &sections {
            let range = reader.section_range(section.id())?;
            values.push(json!({
                "id": section.id().to_string(), "type_id": section.kind().to_string(),
                "kind": match section.kind() { container::BLOB_POOL => "blob_pool", container::APPLICATION => "application", container::PADDING => "padding", _ => "unknown" },
                "offset": range.start.to_string(), "length": section.length().to_string(),
                "checksum": checksum(section.checksum()), "metadata": cbor(section.value())?,
            }));
        }
        report["sections"] = Json::Array(values);
    }
    let mut blobs = BlobStore::new(reader);
    if args.blobs {
        let mut pools = Vec::new();
        for section in sections
            .iter()
            .filter(|section| section.kind() == container::BLOB_POOL)
        {
            let info = blobs.pool_info(section.id())?;
            limits.elements(info.count)?;
            blobs.validate_pool(section.id())?;
            let pages = info.pages.iter().enumerate().map(|(index, page)| Ok(json!({
                "index": index, "first_blob_index": ((index as u64) << info.page_entry_shift).to_string(),
                "offset": (info.payload_offset + page.offset).to_string(),
                "encoding": encoding(&page.encoding)?, "checksum": checksum(page.checksum.as_ref()),
                "checksum_status": if page.checksum.is_some() { "passed" } else { "not_present" },
            }))).collect::<janex_format::Result<Vec<_>>>()?;
            let mut entries = Vec::new();
            for index in 0..info.count {
                let value = match blobs.entry(BlobRef {
                    pool: section.id(),
                    index,
                })? {
                    Entry::Stored {
                        offset,
                        encoding: value,
                    } => json!({
                        "index": index.to_string(), "kind": "stored", "offset": (info.payload_offset + offset).to_string(),
                        "encoding": encoding(&value)?,
                    }),
                    Entry::Extents(extents) => json!({
                        "index": index.to_string(), "kind": "extents",
                        "extents": extents.iter().map(|extent| json!({
                            "stored_blob_index": extent.stored_blob_index.to_string(),
                            "decoded_offset": extent.decoded_offset.to_string(),
                            "decoded_length": extent.decoded_length.to_string(),
                        })).collect::<Vec<_>>(),
                    }),
                    Entry::Unknown(tag) => {
                        json!({"index": index.to_string(), "kind": "unknown", "tag": tag})
                    }
                };
                entries.push(value);
            }
            pools.push(json!({"id": section.id().to_string(), "count": info.count.to_string(),
                "payload_offset": info.payload_offset.to_string(), "page_entry_shift": info.page_entry_shift,
                "pages": pages, "entries": entries}));
        }
        report["blob_pools"] = Json::Array(pools);
    }
    if args.resources || args.data_pools {
        let references: BTreeSet<_> = applications
            .iter()
            .flat_map(|application| application.resource_references())
            .collect();
        limits.elements(references.len() as u64)?;
        let mut roots = Vec::new();
        let mut string_references = BTreeSet::new();
        for reference in references {
            let root = ResourceRoot::decode(&blobs.resolve(reference)?, &mut blobs)?;
            if args.data_pools {
                string_references.insert(root.data_pool);
                for layer in &root.layers {
                    for directory in &layer.directories {
                        for entry in &directory.entries {
                            if let DirectoryEntry::File { content, .. } = entry {
                                for transform in &content.transforms {
                                    if let Some(pool) = transform.properties.get(0)? {
                                        string_references.insert(BlobRef::from_value(&pool)?);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            roots.push(resource(&root, reference)?);
        }
        report["resource_roots"] = Json::Array(roots);
        if args.data_pools {
            let mut pools = Vec::new();
            for reference in string_references {
                let strings =
                    janex_format::data_pool::DataPool::decode(&blobs.resolve(reference)?, limits)?;
                let values = (0..strings.len())
                    .map(|index| {
                        strings
                            .get(index)
                            .map(|bytes| json!({"bytes_hex": hex(bytes)}))
                    })
                    .collect::<janex_format::Result<Vec<_>>>()?;
                pools.push(json!({"reference": self::reference(reference), "entries": values}));
            }
            report["data_pools"] = Json::Array(pools);
        }
    }
    let mut output = io::BufWriter::new(io::stdout().lock());
    if args.json {
        serde_json::to_writer_pretty(&mut output, &report).map_err(io::Error::other)?;
        writeln!(output)?;
    } else {
        print_text(&mut output, &report)?;
    }
    output.flush()?;
    Ok(0)
}

/// Prints scalar JSON strings without their quotes for generated numeric and enum fields.
fn scalar(value: &Json) -> &str {
    value.as_str().unwrap_or("-")
}

/// Renders optional structural detail while retaining escaping for all input-controlled names.
fn print_text(output: &mut impl Write, report: &Json) -> io::Result<()> {
    writeln!(
        output,
        "Janex {}: {} bytes",
        scalar(&report["format_version"]),
        scalar(&report["physical_size"])
    )?;
    writeln!(
        output,
        "  Container: offset={}, length={}, prefix={}, tail={}",
        scalar(&report["janex_range"]["offset"]),
        scalar(&report["janex_range"]["length"]),
        scalar(&report["external_prefix_size"]),
        scalar(&report["external_tail_size"])
    )?;
    writeln!(
        output,
        "  Sections: {}; blob pools: {}",
        report["section_count"], report["blob_pool_count"]
    )?;
    writeln!(
        output,
        "  Verification: {}; container checksums: {}; signature: {}",
        scalar(&report["verification"]["type"]),
        scalar(&report["verification"]["container_checksums"]["status"]),
        scalar(&report["verification"]["signature"])
    )?;
    if report["verification"]["container_checksums"]["status"] == "passed" {
        writeln!(
            output,
            "  Content checksums verified: {}; complete secure coverage: {}",
            report["verification"]["container_checksums"]["content_checksums_verified"],
            report["verification"]["container_checksums"]["complete_secure_coverage"]
        )?;
    }
    for application in report["applications"]
        .as_array()
        .expect("report applications")
    {
        writeln!(
            output,
            "  Application {}: type={}, section={}",
            application["id"],
            application["type"],
            scalar(&application["section_id"])
        )?;
    }
    if let Some(sections) = report["sections"].as_array() {
        for section in sections {
            writeln!(
                output,
                "Section {}: {} (type={}), offset={}, length={}, checksum={}",
                scalar(&section["id"]),
                scalar(&section["kind"]),
                scalar(&section["type_id"]),
                scalar(&section["offset"]),
                scalar(&section["length"]),
                section["checksum"]
            )?;
        }
    }
    if let Some(pools) = report["blob_pools"].as_array() {
        for pool in pools {
            writeln!(
                output,
                "Pool {}: {} blobs; payload offset={}",
                scalar(&pool["id"]),
                scalar(&pool["count"]),
                scalar(&pool["payload_offset"])
            )?;
            for page in pool["pages"].as_array().expect("pool pages") {
                writeln!(
                    output,
                    "  Page {}: offset={}, encoding={}, checksum={}",
                    page["index"],
                    scalar(&page["offset"]),
                    page["encoding"],
                    scalar(&page["checksum_status"])
                )?;
            }
            for entry in pool["entries"].as_array().expect("pool entries") {
                writeln!(output, "  Blob {}: {}", scalar(&entry["index"]), entry)?;
            }
        }
    }
    if let Some(roots) = report["resource_roots"].as_array() {
        for root in roots {
            writeln!(
                output,
                "Resource root {}: reference={}, data pool={}",
                root["name"], root["reference"], root["data_pool"]
            )?;
            for (index, layer) in root["layers"]
                .as_array()
                .expect("resource layers")
                .iter()
                .enumerate()
            {
                writeln!(output, "  Layer {index}: condition={}", layer["condition"])?;
                for directory in layer["directories"].as_array().expect("layer directories") {
                    writeln!(output, "    Directory {}", directory["path"])?;
                    for entry in directory["entries"].as_array().expect("directory entries") {
                        writeln!(output, "      {}", entry)?;
                    }
                }
            }
        }
    }
    if let Some(pools) = report["data_pools"].as_array() {
        for pool in pools {
            writeln!(output, "Data pool {}", pool["reference"])?;
            for (index, value) in pool["entries"]
                .as_array()
                .expect("pool entries")
                .iter()
                .enumerate()
            {
                writeln!(output, "  {index}: {value}")?;
            }
        }
    }
    Ok(())
}

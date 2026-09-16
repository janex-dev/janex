// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! ZIP64 and ordinary ZIP wire vectors shared with the independent Java reader.

use janex_format::{
    binary::Limits,
    cbor::Value,
    container::{Reader, Writer},
};
use janex_host::{
    pack::{PackOptions, pack},
    run::{RunOptions, prepare},
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
};

/// Writes a little-endian fixed-width field.
fn field(bytes: &mut [u8], offset: usize, width: usize, value: u64) {
    bytes[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
}

/// Creates an ordinary EOCD with an exact terminal comment.
fn end(count: u16, size: u32, offset: u32, comment: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 22];
    field(&mut bytes, 0, 4, 0x06054b50);
    field(&mut bytes, 8, 2, count.into());
    field(&mut bytes, 10, 2, count.into());
    field(&mut bytes, 12, 4, size.into());
    field(&mut bytes, 16, 4, offset.into());
    field(&mut bytes, 20, 2, comment.len() as u64);
    bytes.extend(comment);
    bytes
}

/// Wraps a directory with ZIP64 records, including a variable-length extensible-data sector.
fn finish64(
    mut bytes: Vec<u8>,
    count: u64,
    offset: u64,
    extension: &[u8],
    placeholders: bool,
) -> Vec<u8> {
    let position = bytes.len() as u64;
    let size = position - offset;
    let mut record = vec![0; 56];
    field(&mut record, 0, 4, 0x06064b50);
    field(&mut record, 4, 8, 44 + extension.len() as u64);
    field(&mut record, 12, 2, 45);
    field(&mut record, 14, 2, 45);
    field(&mut record, 24, 8, count);
    field(&mut record, 32, 8, count);
    field(&mut record, 40, 8, size);
    field(&mut record, 48, 8, offset);
    bytes.extend(record);
    bytes.extend(extension);
    let mut locator = vec![0; 20];
    field(&mut locator, 0, 4, 0x07064b50);
    field(&mut locator, 8, 8, position);
    field(&mut locator, 16, 4, 1);
    bytes.extend(locator);
    bytes.extend(if placeholders {
        end(u16::MAX, u32::MAX, u32::MAX, b"ZIP64 comment")
    } else {
        end(count as u16, size as u32, offset as u32, b"ZIP64 comment")
    });
    bytes
}

/// Creates a stored member whose central and local fields independently exercise ZIP64 extras.
/// Descriptor modes select absent, unsigned 64-bit, and signed 64-bit data descriptors.
fn member(descriptor: u8, extension: &[u8], placeholders: bool) -> Vec<u8> {
    let mut local = vec![0; 30];
    field(&mut local, 0, 4, 0x04034b50);
    field(&mut local, 4, 2, 45);
    field(&mut local, 6, 2, if descriptor == 0 { 0 } else { 8 });
    field(
        &mut local,
        14,
        4,
        if descriptor == 0 { 0x3610a686 } else { 0 },
    );
    field(&mut local, 18, 4, u32::MAX.into());
    field(&mut local, 22, 4, u32::MAX.into());
    field(&mut local, 26, 2, 5);
    field(&mut local, 28, 2, 20);
    local.extend(b"a.txt");
    local.extend([1, 0, 16, 0]);
    local.extend(5u64.to_le_bytes());
    local.extend(5u64.to_le_bytes());
    local.extend(b"hello");
    if descriptor != 0 {
        if descriptor == 2 {
            local.extend(b"PK\x07\x08");
        }
        local.extend(0x3610a686u32.to_le_bytes());
        local.extend(5u64.to_le_bytes());
        local.extend(5u64.to_le_bytes());
    }
    let offset = local.len() as u64;
    let mut central = vec![0; 46];
    field(&mut central, 0, 4, 0x02014b50);
    field(&mut central, 4, 2, (3 << 8) | 45);
    field(&mut central, 6, 2, 45);
    field(&mut central, 8, 2, if descriptor == 0 { 0 } else { 8 });
    field(&mut central, 16, 4, 0x3610a686);
    field(&mut central, 20, 4, u32::MAX.into());
    field(&mut central, 24, 4, u32::MAX.into());
    field(&mut central, 28, 2, 5);
    field(&mut central, 30, 2, 32);
    field(&mut central, 34, 2, 65535);
    field(&mut central, 38, 4, 0o100644 << 16);
    field(&mut central, 42, 4, u32::MAX.into());
    central.extend(b"a.txt");
    central.extend([1, 0, 28, 0]);
    central.extend(5u64.to_le_bytes());
    central.extend(5u64.to_le_bytes());
    central.extend(0u64.to_le_bytes());
    central.extend(0u32.to_le_bytes());
    local.extend(central);
    finish64(local, 1, offset, extension, placeholders)
}

/// Writes one length-prefixed byte string.
fn bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend((value.len() as u32).to_be_bytes());
    output.extend(value);
}

/// Records native framing acceptance without conflating it with integrity verification.
fn container_vector(output: &mut Vec<u8>, value: &[u8]) {
    output.extend([
        0,
        u8::from(Reader::open_auto(Cursor::new(value), Limits::default()).is_ok()),
    ]);
    bytes(output, value);
    increment(output);
}

/// Records a successfully imported native archive, including Unix metadata.
fn archive_vector(output: &mut Vec<u8>, value: &[u8]) {
    let entries =
        janex_java::jar::read(value, janex_java::Limits::default(), 512 * 1024 * 1024).unwrap();
    output.extend([1, 1]);
    bytes(output, value);
    output.extend((entries.len() as u32).to_be_bytes());
    for entry in entries {
        bytes(output, entry.name.as_bytes());
        bytes(output, &entry.content);
        output.extend(entry.unix_mode.map_or(-1, |mode| mode as i32).to_be_bytes());
    }
    increment(output);
}

/// Increments the vector count in the stream header.
fn increment(output: &mut [u8]) {
    let count = u32::from_be_bytes(output[..4].try_into().unwrap()) + 1;
    output[..4].copy_from_slice(&count.to_be_bytes());
}

#[test]
fn java_matches_zip64_import_and_native_tail_boundary_validation() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let jar = project.join("janex-bootstrap/build/libs/janex-bootstrap.jar");
    let compile = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&jar)
        .arg("-d")
        .arg(temp.path())
        .arg(
            project
                .join("janex-reader/src/testFixtures/java/org/glavo/janex/reader/ArchiveTest.java"),
        )
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let janex = Writer::new(Vec::new())
        .unwrap()
        .finish_with::<janex_format::Error>(Value::empty_map(), 0, |_| Ok(Vec::new()))
        .unwrap();
    let mut vectors = vec![0; 4];
    container_vector(&mut vectors, &janex);
    for extension in [vec![], vec![42; 17], vec![0; 70000]] {
        for placeholders in [false, true] {
            let empty = finish64(Vec::new(), 0, 0, &extension, placeholders);
            archive_vector(&mut vectors, &empty);
            container_vector(&mut vectors, &[&janex[..], &empty].concat());
            for descriptor in 0..3 {
                let archive = member(descriptor, &extension, placeholders);
                archive_vector(&mut vectors, &archive);
                container_vector(
                    &mut vectors,
                    &[b"external header", &janex[..], &archive].concat(),
                );
            }
        }
    }
    let empty = end(0, 0, 0, b"");
    let mut unspecified_mode = member(0, &[], true);
    let central = unspecified_mode
        .windows(4)
        .position(|value| value == b"PK\x01\x02")
        .unwrap();
    field(&mut unspecified_mode, central + 38, 4, 0xffff0000);
    let entries = janex_java::jar::read(
        &unspecified_mode,
        janex_java::Limits::default(),
        512 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(entries[0].unix_mode, None);
    archive_vector(&mut vectors, &unspecified_mode);
    archive_vector(&mut vectors, &empty);
    container_vector(&mut vectors, &[&janex[..], &empty].concat());
    let nested = end(0, 0, 0, &[&janex[..], &empty].concat());
    container_vector(&mut vectors, &[&janex[..], &nested].concat());
    // A real directory crossing the 16-bit count boundary must not truncate its entries.
    let mut large = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..65536 {
        large
            .start_file(
                format!("entry-{index:05}"),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
    }
    let large = large.finish().unwrap().into_inner();
    archive_vector(&mut vectors, &large);
    container_vector(&mut vectors, &[&janex[..], &large].concat());
    let mut false_janex = janex.clone();
    false_janex[0] ^= 1;
    let nested = end(0, 0, 0, &[&false_janex[..], &empty].concat());
    container_vector(&mut vectors, &[&janex[..], &nested].concat());
    for descriptor in 0..3 {
        let archive = member(descriptor, &[], true);
        for i in 0..archive.len() {
            let mut changed = archive.clone();
            changed[i] ^= 1;
            container_vector(&mut vectors, &[&janex[..], &changed].concat());
        }
        for size in 0..archive.len() {
            container_vector(&mut vectors, &[&janex[..], &archive[..size]].concat());
        }
    }
    let fixture = temp.path().join("archives.bin");
    fs::write(&fixture, vectors).unwrap();
    let classpath = std::env::join_paths([temp.path(), jar.as_path()]).unwrap();
    let mut runtimes = vec![PathBuf::from("java")];
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        runtimes.push(Path::new(&home).join("bin").join(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
    }
    for java in runtimes {
        let result = Command::new(java)
            .arg("-cp")
            .arg(&classpath)
            .arg("org.glavo.janex.reader.ArchiveTest")
            .arg(&fixture)
            .arg(temp.path().join("snapshot.janex"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn standalone_api_launches_an_executable_zip64_tail() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/build/libs/janex-bootstrap.jar");
    let harness = temp.path().join("harness");
    let compile = Command::new("javac")
        .args(["--release", "8", "-cp"])
        .arg(&bootstrap)
        .arg("-d")
        .arg(&harness)
        .arg(project.join(
            "janex-bootstrap/src/testFixtures/java/org/glavo/janex/bootstrap/StandaloneTest.java",
        ))
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let classpath = std::env::join_paths([harness.as_path(), bootstrap.as_path()]).unwrap();
    fs::write(temp.path().join("Main.java"), "public class Main { public static void main(String[] args) { System.out.println(\"zip64-ok\"); } }").unwrap();
    let result = Command::new("javac")
        .current_dir(temp.path())
        .args(["--release", "8", "-d", "classes", "Main.java"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let source = temp.path().join("source.janex");
    let mut options = PackOptions::new(temp.path().join("classes"), &source);
    options.main_class = Some("Main".into());
    pack(&options).unwrap();
    let mut jar = fs::read(project.join("janex-bootstrap/build/libs/janex-bootstrap.jar")).unwrap();
    let end = jar.len() - 22;
    assert_eq!(&jar[end..end + 4], b"PK\x05\x06");
    let count = u16::from_le_bytes(jar[end + 10..end + 12].try_into().unwrap()) as u64;
    let offset = u32::from_le_bytes(jar[end + 16..end + 20].try_into().unwrap()) as u64;
    jar.truncate(end);
    let tail = finish64(jar, count, offset, &vec![0; 70000], true);
    let mut reader =
        Reader::open_auto(fs::File::open(&source).unwrap(), Limits::default()).unwrap();
    let mut writer = Writer::new(Vec::new()).unwrap();
    for section in reader.sections().cloned().collect::<Vec<_>>() {
        writer
            .write_section(
                section.id(),
                section.kind(),
                &reader.read_section(section.id()).unwrap(),
                section.type_info().unwrap(),
            )
            .unwrap();
    }
    let region = Value::map([
        (Value::uint(0), Value::uint(tail.len() as u64)),
        (
            Value::uint(1),
            Value::bytes(
                &janex_format::checksum::Checksum::compute(
                    janex_format::checksum::Algorithm::Sha256,
                    tail.as_slice(),
                )
                .unwrap()
                .encode(),
            ),
        ),
    ])
    .unwrap();
    let mut metadata = reader.metadata().as_map().unwrap();
    metadata.retain(|(key, _)| !matches!(key.as_u64(), Ok(0 | 2)));
    metadata.push((Value::uint(2), region));
    let mut output = writer.finish(Value::map(metadata).unwrap()).unwrap();
    output.extend(tail);
    let target = temp.path().join("zip64.janex");
    fs::write(&target, output).unwrap();
    let mut run = RunOptions::new(&target);
    run.allow_unsigned = true;
    assert!(prepare(&run).unwrap().execute().unwrap().success());
    let mut runtimes = vec![PathBuf::from("java")];
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        runtimes.push(Path::new(&home).join("bin").join(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
    }
    for java in runtimes {
        let output = Command::new(&java)
            .arg("-cp")
            .arg(&classpath)
            .arg("org.glavo.janex.bootstrap.StandaloneTest")
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{java:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "zip64-ok");
    }
}

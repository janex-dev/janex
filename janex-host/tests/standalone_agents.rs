// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Native and independent Java agent preparation, invocation order, and failure isolation.

#[path = "support/http.rs"]
mod http;

use janex_format::{
    application::Application,
    binary::Limits,
    blob::{BlobRef, BlobStore, PoolBuilder},
    cbor::{self, Value},
    checksum::{Algorithm, Checksum},
    container::{APPLICATION, BLOB_POOL, Reader, Writer},
    resource::{DirectoryEntry, ResourceRoot},
};
use janex_host::{
    pack::{PackOptions, pack},
    run::{RunOptions, prepare},
};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use zip::{ZipWriter, write::SimpleFileOptions};

/// Replaces an integer-keyed configuration field.
fn replace(value: Value, key: u64, replacement: Value) -> Value {
    let mut entries = value.as_map().unwrap();
    entries.retain(|(candidate, _)| candidate.as_u64().unwrap() != key);
    entries.push((Value::uint(key), replacement));
    Value::map(entries).unwrap()
}

/// Rewrites application bytes and their enclosing checksums while retaining the executable tail.
fn configure(source: &Path, target: &Path, change: impl Fn(Value) -> Value) {
    let original = fs::read(source).unwrap();
    let mut reader = Reader::open_auto(Cursor::new(&original), Limits::default()).unwrap();
    let tail = &original[reader.range().end as usize..];
    let mut writer = Writer::new(Vec::new()).unwrap();
    for section in reader.sections().cloned().collect::<Vec<_>>() {
        let info = section.type_info().unwrap();
        let mut bytes = reader.read_section(section.id()).unwrap();
        if section.kind() == APPLICATION {
            let application =
                Application::decode(&bytes, info.clone().unwrap(), Limits::default()).unwrap();
            let descriptor = application.value().required(0).unwrap();
            let config = change(descriptor.required(0).unwrap());
            let value = replace(
                application.value().clone(),
                0,
                replace(descriptor, 0, config),
            );
            bytes = b"JANEXAPP".to_vec();
            cbor::write_sized(&mut bytes, &value).unwrap();
        }
        writer
            .write_section(section.id(), section.kind(), &bytes, info)
            .unwrap();
    }
    let mut metadata = reader.metadata().as_map().unwrap();
    metadata.retain(|(key, _)| key.as_u64().unwrap() != 0);
    let mut bytes = writer.finish(Value::map(metadata).unwrap()).unwrap();
    bytes.extend(tail);
    fs::write(target, bytes).unwrap();
}

/// Compiles one fixture with Java 8 class-file compatibility.
fn compile(directory: &Path, source: &str, output: &str) {
    let result = Command::new("javac")
        .current_dir(directory)
        .args(["--release", "8", "-d", output, source])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

/// Creates a ZIP64 agent JAR with instrumentation capabilities and an ordinary resource.
fn jar(directory: &Path, main: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().large_file(true);
    zip.start_file("META-INF/MANIFEST.MF", options).unwrap();
    write!(zip, "Manifest-Version: 1.0\r\nPremain-Class: {main}\r\nCan-Retransform-Classes: true\r\nCan-Redefine-Classes: true\r\nClass-Path: forbidden.jar\r\n\r\n").unwrap();
    let mut files: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    for file in files {
        zip.start_file(file.file_name().unwrap().to_str().unwrap(), options)
            .unwrap();
        zip.write_all(&fs::read(file).unwrap()).unwrap();
    }
    zip.start_file(format!("{main}.txt"), options).unwrap();
    zip.write_all(b"agent-resource").unwrap();
    zip.finish().unwrap().into_inner()
}

/// Encodes an agent declaration without splitting or normalizing its option string.
fn agent(reference: Value, option: &str) -> Value {
    Value::map([
        (Value::uint(0), reference),
        (Value::uint(1), Value::text(option)),
    ])
    .unwrap()
}

/// Rebuilds the local agent root with valid or corrupted logical-file checksums.
fn agent_hashes(source: &Path, target: &Path, algorithm: Algorithm, corrupt: bool) {
    let original = fs::read(source).unwrap();
    let mut reader = Reader::open_auto(Cursor::new(&original), Limits::default()).unwrap();
    let section = reader
        .sections()
        .find(|section| section.kind() == APPLICATION)
        .unwrap()
        .clone();
    let application = Application::decode(
        &reader.read_section(section.id()).unwrap(),
        section.type_info().unwrap().unwrap(),
        Limits::default(),
    )
    .unwrap();
    let paths = application
        .value()
        .required(0)
        .unwrap()
        .required(0)
        .unwrap()
        .required(3)
        .unwrap()
        .as_array()
        .unwrap();
    let reference = BlobRef::from_value(&paths[1].required(1).unwrap()).unwrap();
    let mut blobs = BlobStore::new(reader);
    let bytes = blobs.resolve(reference).unwrap();
    let mut root = ResourceRoot::decode(&bytes, &mut blobs).unwrap();
    for layer in &mut root.layers {
        for directory in &mut layer.directories {
            for entry in &mut directory.entries {
                if let DirectoryEntry::File {
                    content, metadata, ..
                } = entry
                {
                    let bytes = content.resolve_file(&mut blobs, &root.strings).unwrap();
                    let mut checksum = Checksum::compute(algorithm, bytes.as_slice())
                        .unwrap()
                        .encode();
                    if corrupt {
                        *checksum.last_mut().unwrap() ^= 1;
                    }
                    *metadata = replace(metadata.clone(), 0, Value::bytes(&checksum));
                }
            }
        }
    }
    let encoded_root = root.encode(Limits::default()).unwrap();
    let encoded_strings = root.strings.encode().unwrap();
    let mut reader = Reader::open_auto(Cursor::new(&original), Limits::default()).unwrap();
    let mut writer = Writer::new(Vec::new()).unwrap();
    for section in reader.sections().cloned().collect::<Vec<_>>() {
        if section.kind() == BLOB_POOL {
            let count = section
                .type_info()
                .unwrap()
                .unwrap()
                .required(0)
                .unwrap()
                .as_u64()
                .unwrap();
            let mut builder = PoolBuilder::new();
            for index in 0..count {
                let current = BlobRef {
                    pool: section.id(),
                    index,
                };
                let bytes = if current == reference {
                    encoded_root.clone()
                } else if current == root.string_pool {
                    encoded_strings.clone()
                } else {
                    blobs.resolve(current).unwrap()
                };
                builder.push(&bytes, 3).unwrap();
            }
            let pool = builder.finish(8, 3).unwrap();
            writer
                .write_section(section.id(), BLOB_POOL, &pool.bytes, Some(pool.type_info))
                .unwrap();
        } else {
            writer
                .write_section(
                    section.id(),
                    section.kind(),
                    &reader.read_section(section.id()).unwrap(),
                    section.type_info().unwrap(),
                )
                .unwrap();
        }
    }
    let mut metadata = reader.metadata().as_map().unwrap();
    metadata.retain(|(key, _)| key.as_u64().unwrap() != 0);
    let mut bytes = writer.finish(Value::map(metadata).unwrap()).unwrap();
    bytes.extend(&original[reader.range().end as usize..]);
    fs::write(target, bytes).unwrap();
}

#[test]
fn standalone_agents_preserve_order_options_instrumentation_and_preparation_isolation() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("LocalAgent.java"), r#"
import java.lang.instrument.*;
import java.nio.file.*;
import java.security.ProtectionDomain;
/// Records native premain invocations and observes application transformation.
public class LocalAgent {
    /// Checks the selected agent capabilities and records one invocation.
    public static void premain(String option, Instrumentation instrumentation) throws Exception {
        if (!ClassLoader.getSystemClassLoader().getClass().getName().equals("org.janex.bootstrap.ResourceLoader")) throw new AssertionError("agent ran in parent");
        if (!instrumentation.isRetransformClassesSupported() || !instrumentation.isRedefineClassesSupported()) throw new AssertionError("capabilities lost");
        try (java.io.InputStream input = LocalAgent.class.getResourceAsStream("/LocalAgent.txt")) {
            if (input == null || input.read() != 'a') throw new AssertionError("resource missing");
        }
        String value = option == null || option.isEmpty() ? "<empty>" : option;
        String order = System.getProperty("agent.order", "") + value + "|";
        System.setProperty("agent.order", order);
        Files.write(Paths.get(System.getProperty("agent.marker")), (value + "\n").getBytes("UTF-8"), StandardOpenOption.CREATE, StandardOpenOption.APPEND);
        instrumentation.addTransformer(new ClassFileTransformer() {
            /// Observes the application class without changing its bytecode.
            public byte[] transform(ClassLoader loader, String name, Class<?> type, ProtectionDomain domain, byte[] bytes) {
                if (name.equals("Main")) System.setProperty("agent.transformed", "yes");
                return null;
            }
        }, true);
    }
}
"#).unwrap();
    fs::write(temp.path().join("RemoteAgent.java"), r#"
import java.nio.file.*;
/// Exercises the one-argument premain method in a remote agent root.
public class RemoteAgent {
    /// Appends the remote invocation to the shared order and marker.
    public static void premain(String option) throws Exception {
        if (!option.equals("remote=three words")) throw new AssertionError(option);
        System.setProperty("agent.order", System.getProperty("agent.order", "") + option + "|");
        Files.write(Paths.get(System.getProperty("agent.marker")), (option + "\n").getBytes("UTF-8"), StandardOpenOption.CREATE, StandardOpenOption.APPEND);
    }
}
"#).unwrap();
    fs::write(temp.path().join("Main.java"), r#"
/// Verifies that all selected agents ran before the application entry point.
public class Main {
    /// Checks argument order and the application transformation hook.
    public static void main(String[] args) {
        if (!"one=two words|<empty>|remote=three words|".equals(System.getProperty("agent.order"))) throw new AssertionError(System.getProperty("agent.order"));
        if (!"yes".equals(System.getProperty("agent.transformed"))) throw new AssertionError("transformer not called");
        System.out.println("agents-ok");
    }
}
"#).unwrap();
    compile(temp.path(), "LocalAgent.java", "local");
    compile(temp.path(), "RemoteAgent.java", "remote");
    compile(temp.path(), "Main.java", "main");
    let local = temp.path().join("agent=local.jar");
    fs::write(&local, jar(&temp.path().join("local"), "LocalAgent")).unwrap();
    let remote = jar(&temp.path().join("remote"), "RemoteAgent");
    let server = http::Server::new();
    server.file("/remote.jar", &remote);
    let remote = Value::map([
        (Value::uint(0), Value::uint(1)),
        (
            Value::uint(1),
            Value::text(&format!("{}/remote.jar", server.url)),
        ),
        (
            Value::uint(2),
            Value::bytes(
                &Checksum::compute(Algorithm::Sm3, remote.as_slice())
                    .unwrap()
                    .encode(),
            ),
        ),
    ])
    .unwrap();
    let marker = temp.path().join("marker.txt");
    let source = temp.path().join("source.janex");
    let mut packing = PackOptions::new(temp.path().join("main"), &source);
    packing.main_class = Some("Main".into());
    packing.class_path.push(local);
    packing
        .jvm_options
        .push(format!("-Dagent.marker={}", marker.display()));
    packing.with_launcher = true;
    pack(&packing).unwrap();
    let target = temp.path().join("agents.janex");
    let selected_agents = |config: Value| {
        let entries = config.required(3).unwrap().as_array().unwrap();
        let local = entries[1].clone();
        let selected = vec![
            agent(local.clone(), "one=two words"),
            agent(local, ""),
            agent(remote.clone(), "remote=three words"),
        ];
        // Clearing and appending in nested overlays must preserve the final order.
        let overlay = Value::map([
            (Value::uint(4), Value::null()),
            (
                Value::uint(6),
                Value::array([Value::map([(Value::uint(4), Value::array(selected))]).unwrap()]),
            ),
        ])
        .unwrap();
        let config = replace(config, 3, Value::array([entries[0].clone()]));
        let config = replace(
            config,
            4,
            Value::array([agent(remote.clone(), "must-not-run")]),
        );
        replace(config, 6, Value::array([overlay]))
    };
    configure(&source, &target, selected_agents);
    let cache = temp.path().join("cache");
    let launch_directory = temp.path().join("launch files");
    fs::create_dir(&launch_directory).unwrap();
    let mut options = RunOptions::new(&target);
    options.allow_unsigned = true;
    options.dependencies.cache_directory = Some(cache.clone());
    let prepared = prepare(&options).unwrap();
    assert!(!marker.exists(), "agent executed during native preparation");
    let output = prepared
        .command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "agents-ok");
    let expected = b"one=two words\n<empty>\nremote=three words\n";
    assert_eq!(fs::read(&marker).unwrap(), expected);
    fs::remove_file(&marker).unwrap();
    let mut runtimes = vec![PathBuf::from("java")];
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        runtimes.push(Path::new(&home).join("bin").join(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
    }
    for java in &runtimes {
        let output = Command::new(java)
            .arg(format!("-Djava.io.tmpdir={}", launch_directory.display()))
            .arg(format!("-Djanex.dependencyCache={}", cache.display()))
            .arg("-Djanex.offline=true")
            .arg("-jar")
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{java:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "agents-ok");
        assert_eq!(fs::read(&marker).unwrap(), expected);
        fs::remove_file(&marker).unwrap();
        assert_eq!(fs::read_dir(&launch_directory).unwrap().count(), 0);
    }
    for algorithm in [
        Algorithm::Xxh3_64,
        Algorithm::Xxh3_128,
        Algorithm::Sha256,
        Algorithm::Sha512,
        Algorithm::Sm3,
    ] {
        for corrupt in [false, true] {
            let hashed_source = temp.path().join("hashed-source.janex");
            let hashed = temp.path().join("hashed.janex");
            agent_hashes(&source, &hashed_source, algorithm, corrupt);
            configure(&hashed_source, &hashed, selected_agents);
            let mut native = RunOptions::new(&hashed);
            native.allow_unsigned = true;
            native.dependencies.cache_directory = Some(cache.clone());
            native.dependencies.offline = true;
            assert_eq!(prepare(&native).is_ok(), !corrupt);
            assert!(!marker.exists());
            let output = Command::new("java")
                .arg(format!("-Djanex.dependencyCache={}", cache.display()))
                .arg("-Djanex.offline=true")
                .arg("-jar")
                .arg(&hashed)
                .output()
                .unwrap();
            assert_eq!(
                output.status.success(),
                !corrupt,
                "{algorithm:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if corrupt {
                assert!(!marker.exists());
                assert!(String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"));
            } else {
                assert_eq!(fs::read(&marker).unwrap(), expected);
                fs::remove_file(&marker).unwrap();
            }
        }
    }
    let failed = temp.path().join("failed.janex");
    configure(&source, &failed, |config| {
        let entries = config.required(3).unwrap().as_array().unwrap();
        let missing = replace(
            remote.clone(),
            1,
            Value::text(&format!("{}/missing.jar", server.url)),
        );
        replace(
            config,
            4,
            Value::array([
                agent(entries[1].clone(), "must-not-run"),
                agent(missing, ""),
            ]),
        )
    });
    for java in runtimes {
        let output = Command::new(java)
            .arg(format!("-Djava.io.tmpdir={}", launch_directory.display()))
            .arg(format!("-Djanex.dependencyCache={}", cache.display()))
            .arg("-Djanex.offline=true")
            .arg("-jar")
            .arg(&failed)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            !marker.exists(),
            "agent executed before all dependencies were prepared"
        );
        assert_eq!(fs::read_dir(&launch_directory).unwrap().count(), 0);
    }
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let malformed = temp.path().join("malformed.janex");
    configure(&source, &malformed, |config| {
        let entries = config.required(3).unwrap().as_array().unwrap();
        let condition = Value::map([(Value::uint(1), Value::text("unavailable-test-os"))]).unwrap();
        let invalid = replace(agent(entries[1].clone(), ""), 1, Value::uint(7));
        let overlay = Value::map([
            (Value::uint(0), condition),
            (Value::uint(4), Value::array([invalid])),
        ])
        .unwrap();
        replace(config, 6, Value::array([overlay]))
    });
    let mut malformed_options = RunOptions::new(&malformed);
    malformed_options.allow_unsigned = true;
    assert!(prepare(&malformed_options).is_err());
    let output = Command::new("java")
        .arg("-jar")
        .arg(&malformed)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Expected CBOR text"));
}

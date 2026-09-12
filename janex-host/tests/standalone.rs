// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Cross-language acceptance of Rust-written packages launched entirely through Java.

use janex_format::{binary::Limits, container::Reader};
use janex_host::{
    pack::{PackOptions, pack},
    run::{LaunchMode, RunOptions, prepare},
};
use std::{fs, path::Path, process::Command};

/// Compiles Java fixtures and requires successful tool completion.
fn javac(directory: &Path, args: &[&str]) {
    let result = Command::new("javac")
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn jar_tail_launches_resources_arguments_and_both_native_modes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), r#"
import java.nio.file.*;
public class Main {
    public static void main(String[] args) throws Exception {
        if (!System.getProperty("example.value").equals("two words")) throw new AssertionError();
        if (!System.getProperty("outer.value").equals("inherited")) throw new AssertionError();
        String preset = Boolean.getBoolean("native.direct") ? "ascii" : "\ud83d\ude80";
        if (!args[0].equals(preset) || !args[1].isEmpty() || !args[2].equals("user value")) throw new AssertionError();
        java.net.URL url = Main.class.getResource("/value.txt");
        try (java.io.InputStream input = url.openStream()) {
            if (input.read() != 'o') throw new AssertionError();
        }
        if (url.getProtocol().equals("janex")) {
            Path path = Paths.get(url.toURI());
            if (!new String(Files.readAllBytes(path), "UTF-8").equals("ok")) throw new AssertionError();
            if (!Files.isDirectory(path.getParent())) throw new AssertionError();
        }
        System.out.println("standalone-ok");
    }
}
"#).unwrap();
    javac(
        temp.path(),
        &["--release", "8", "-d", "classes", "Main.java"],
    );
    fs::write(temp.path().join("classes/value.txt"), b"ok").unwrap();
    let output = temp.path().join("application with spaces.janex");
    let mut options = PackOptions::new(temp.path().join("classes"), &output);
    options.main_class = Some("Main".into());
    options.arguments = vec!["\u{1f680}".into(), "".into()];
    options.jvm_options = vec!["-Dexample.value=two words".into()];
    options.java_version = Some("vers:jep322/>=8".into());
    options.with_launcher = true;
    pack(&options).unwrap();
    let mut reader =
        Reader::open_auto(fs::File::open(&output).unwrap(), Limits::default()).unwrap();
    assert!(reader.verify_checksums().unwrap().complete_secure_coverage);
    let mut runtimes = vec![std::path::PathBuf::from("java")];
    let launch_directory = temp.path().join("launch files");
    fs::create_dir(&launch_directory).unwrap();
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        runtimes.push(Path::new(&home).join("bin").join(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
    }
    for java in runtimes {
        let result = Command::new(java)
            .arg(format!("-Djava.io.tmpdir={}", launch_directory.display()))
            .arg("-Douter.value=inherited")
            .arg("-jar")
            .arg(&output)
            .arg("user value")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "standalone-ok"
        );
        assert_eq!(fs::read_dir(&launch_directory).unwrap().count(), 0);
    }
    for mode in [LaunchMode::Bootstrap, LaunchMode::Direct] {
        let mut run = RunOptions::new(&output);
        run.allow_unsigned = true;
        run.launch_mode = mode;
        run.arguments = vec!["user value".into()];
        // The native path has no outer JVM from which to inherit this property.
        let mut native_options = options.clone();
        native_options.output = temp.path().join(format!("native-{mode:?}.janex"));
        native_options
            .jvm_options
            .push("-Douter.value=inherited".into());
        if mode == LaunchMode::Direct {
            native_options.arguments[0] = "ascii".into();
            native_options
                .jvm_options
                .push("-Dnative.direct=true".into());
        }
        pack(&native_options).unwrap();
        run.target = native_options.output;
        let plan = prepare(&run).unwrap();
        let result = plan
            .command()
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let mut corrupt = fs::read(&output).unwrap();
    corrupt[20] ^= 1;
    fs::write(temp.path().join("corrupt.janex"), corrupt).unwrap();
    let result = Command::new("java")
        .arg(format!("-Djava.io.tmpdir={}", launch_directory.display()))
        .arg("-jar")
        .arg(temp.path().join("corrupt.janex"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("checksum mismatch"));
    assert_eq!(fs::read_dir(&launch_directory).unwrap().count(), 0);
}

#[test]
fn jar_tail_launches_named_module_and_propagates_exit_status() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("src/app")).unwrap();
    fs::write(
        temp.path().join("src/module-info.java"),
        "module sample.app { requires java.logging; }",
    )
    .unwrap();
    fs::write(temp.path().join("src/app/Main.java"), r#"
package app;
public class Main {
    public static void main(String[] args) throws Exception {
        if (!Main.class.getModule().getName().equals("sample.app")) throw new AssertionError();
        java.util.logging.Logger.getLogger("sample");
        java.net.URL resource = Main.class.getResource("data.txt");
        if (java.nio.file.Files.readAllBytes(java.nio.file.Paths.get(resource.toURI()))[0] != 42) throw new AssertionError();
        System.exit(37);
    }
}

"#).unwrap();
    javac(
        temp.path(),
        &[
            "--release",
            "11",
            "-d",
            "classes",
            "src/module-info.java",
            "src/app/Main.java",
        ],
    );
    fs::write(temp.path().join("classes/app/data.txt"), [42]).unwrap();
    let mut options = PackOptions::new(
        temp.path().join("classes"),
        temp.path().join("module.janex"),
    );
    options.main_class = Some("app.Main".into());
    options.main_module = Some("sample.app".into());
    options.with_launcher = true;
    pack(&options).unwrap();
    let result = Command::new("java")
        .arg("-jar")
        .arg(options.output)
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(37),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn java_reader_matches_rust_for_extents_transforms_layers_and_links() {
    use janex_format::{
        application::Application,
        blob::{BlobRef, Extent, PoolBuilder},
        cbor::Value,
        checksum::{Algorithm, Checksum},
        classfile,
        condition::Condition,
        container::{APPLICATION, BLOB_POOL, Writer},
        content::{Content, Source, Transform},
        resource::{Directory, DirectoryEntry, Layer, ResourceRoot},
        strings::StringPool,
    };
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), r#"
import java.nio.file.*;
public class Main {
    public static void main(String[] args) throws Exception {
        if (!java.util.Arrays.equals(args, new String[]{"selected", "user"})) throw new AssertionError();
        if (Main.class.getResource("/obsolete") != null) throw new AssertionError();
        if (!Files.isDirectory(Paths.get(Main.class.getResource("/replaced/").toURI()))) throw new AssertionError();
        Path path = Paths.get(Main.class.getResource("/alias/value.txt").toURI());
        if (!new String(Files.readAllBytes(path), "UTF-8").equals("abcd")) throw new AssertionError();
        Object time = Files.getAttribute(path, "janex:creationTimeNanos");
        if (!time.equals(java.math.BigInteger.ONE.shiftLeft(80))) throw new AssertionError(time);
        if (!Files.getAttribute(path, "janex:permissions").equals(0)) throw new AssertionError();
        try (java.io.InputStream in = Main.class.getResourceAsStream("/Main.class")) {
            if (in.read() != 0xca || in.read() != 0xfe || in.read() != 0xba || in.read() != 0xbe) throw new AssertionError();
        }
        System.out.println("advanced-ok");
    }
}
"#).unwrap();
    javac(
        temp.path(),
        &["--release", "8", "-d", "classes", "Main.java"],
    );
    let limits = Limits::default();
    let reference = |index| BlobRef { pool: 7, index };
    let mut pool = PoolBuilder::new();
    let stored = pool.push(b"abXXcd", 3).unwrap();
    for _ in 0..260 {
        pool.push(b"unused", 3).unwrap();
    }
    let extents = pool
        .push_extents(vec![
            Extent {
                stored_blob_index: stored,
                decoded_offset: 0,
                decoded_length: 2,
            },
            Extent {
                stored_blob_index: stored,
                decoded_offset: 4,
                decoded_length: 2,
            },
        ])
        .unwrap();
    let class = fs::read(temp.path().join("classes/Main.class")).unwrap();
    let mut class_strings = StringPool::new();
    let transformed = classfile::transform(&class, &mut class_strings, limits)
        .unwrap()
        .unwrap();
    let class_source = pool.push(&transformed, 3).unwrap();
    let class_pool = pool.push(&class_strings.encode().unwrap(), 3).unwrap();
    let mut names = StringPool::new();
    names.intern("Main");
    let mut root = ResourceRoot {
        string_pool: reference(pool.len() as u64),
        strings: names,
        metadata: Value::empty_map(),
        layers: vec![
            Layer {
                condition: Condition::unconditional(),
                directories: vec![
                    Directory {
                        path: String::new(),
                        metadata: Value::empty_map(),
                        entries: vec![
                            DirectoryEntry::File {
                                name: "Main.class".into(),
                                content: Content {
                                    source: Source::Blob(reference(class_source)),
                                    transforms: vec![Transform {
                                        input_size: class.len() as u64,
                                        method: 1,
                                        properties: Value::map([(
                                            Value::uint(0),
                                            reference(class_pool).to_value(),
                                        )])
                                        .unwrap(),
                                    }],
                                },
                                metadata: Value::empty_map(),
                            },
                            DirectoryEntry::File {
                                name: "obsolete".into(),
                                content: Content::inline(b"old".to_vec()),
                                metadata: Value::empty_map(),
                            },
                        ],
                    },
                    Directory {
                        path: "data".into(),
                        metadata: Value::empty_map(),
                        entries: vec![DirectoryEntry::File {
                            name: "value.txt".into(),
                            content: Content::blob(reference(extents)),
                            metadata: Value::map([
                                (Value::uint(2), Value::integer(1i128 << 80)),
                                (Value::uint(5), Value::uint(0)),
                            ])
                            .unwrap(),
                        }],
                    },
                ],
            },
            Layer {
                condition: Condition::unconditional(),
                directories: vec![Directory {
                    path: String::new(),
                    metadata: Value::empty_map(),
                    entries: vec![
                        DirectoryEntry::SymbolicLink {
                            name: "alias".into(),
                            target: "data".into(),
                            metadata: Value::empty_map(),
                        },
                        DirectoryEntry::Tombstone {
                            name: "obsolete".into(),
                        },
                    ],
                }],
            },
        ],
    };
    root.layers[0].directories[0]
        .entries
        .push(DirectoryEntry::File {
            name: "replaced".into(),
            content: Content::inline(b"file becomes directory".to_vec()),
            metadata: Value::empty_map(),
        });
    root.layers[1].directories[0]
        .entries
        .push(DirectoryEntry::Tombstone {
            name: "replaced".into(),
        });
    root.layers[1].directories.push(Directory {
        path: "replaced".into(),
        metadata: Value::empty_map(),
        entries: vec![],
    });
    let root_bytes = root.encode(limits).unwrap();
    assert_eq!(
        pool.push(&root.strings.encode().unwrap(), 3).unwrap(),
        root.string_pool.index
    );
    let root_index = pool.push(&root_bytes, 3).unwrap();
    let pool = pool.finish(8, 3).unwrap();
    let point = Value::map([(Value::uint(0), Value::text("Main"))]).unwrap();
    let entry = Value::map([
        (Value::uint(0), Value::uint(0)),
        (Value::uint(1), reference(root_index).to_value()),
    ])
    .unwrap();
    let skipped = Value::map([
        (
            Value::uint(0),
            Value::map([(Value::uint(1), Value::text("never"))]).unwrap(),
        ),
        (Value::uint(7), Value::array([Value::text("skipped")])),
        (
            Value::uint(6),
            Value::array([Value::map([(
                Value::uint(7),
                Value::array([Value::text("also-skipped")]),
            )])
            .unwrap()]),
        ),
    ])
    .unwrap();
    let config = Value::map([
        (Value::uint(1), point),
        (Value::uint(3), Value::array([entry])),
        (
            Value::uint(6),
            Value::array([
                skipped,
                Value::map([(Value::uint(7), Value::array([Value::text("selected")]))]).unwrap(),
            ]),
        ),
    ])
    .unwrap();
    let app = Application::from_values(
        Value::map([
            (Value::uint(0), Value::text("main")),
            (Value::uint(1), Value::text("janex.java")),
        ])
        .unwrap(),
        Value::map([(
            Value::uint(0),
            Value::map([(Value::uint(0), config)]).unwrap(),
        )])
        .unwrap(),
        limits,
    )
    .unwrap();
    let tail = include_bytes!("../../janex-bootstrap/bootstrap.jar");
    let mut writer = Writer::new(Vec::new()).unwrap();
    writer
        .write_section(7, BLOB_POOL, &pool.bytes, Some(pool.type_info))
        .unwrap();
    writer
        .write_section(
            8,
            APPLICATION,
            &app.encode().unwrap(),
            Some(app.type_info().clone()),
        )
        .unwrap();
    let mut bytes = writer
        .finish(
            Value::map([
                (
                    Value::uint(1),
                    Value::map([(Value::uint(0), Value::uint(0))]).unwrap(),
                ),
                (
                    Value::uint(2),
                    Value::map([
                        (Value::uint(0), Value::uint(tail.len() as u64)),
                        (
                            Value::uint(1),
                            Value::bytes(
                                &Checksum::compute(Algorithm::Sha256, &tail[..])
                                    .unwrap()
                                    .encode(),
                            ),
                        ),
                    ])
                    .unwrap(),
                ),
            ])
            .unwrap(),
        )
        .unwrap();
    bytes.extend_from_slice(tail);
    let output = temp.path().join("advanced.janex");
    fs::write(&output, bytes).unwrap();
    let result = Command::new("java")
        .arg("-jar")
        .arg(&output)
        .arg("user")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "advanced-ok"
    );
    let mut run = RunOptions::new(&output);
    run.allow_unsigned = true;
    run.arguments = vec!["user".into()];
    let plan = prepare(&run).unwrap();
    let result = plan
        .command()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "advanced-ok"
    );
}

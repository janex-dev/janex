// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Complete packaging round trips, deterministic output, and real Java fixtures.

use janex_format::{
    application::{JavaLaunch, PathEntry, read_applications},
    binary::Limits,
    blob::BlobStore,
    condition::{Context, RuntimeContext},
    container::{Reader, Verification},
    content::Source,
    resource::{Node, ResourceRoot},
    version::JavaVersion,
};
use janex_host::pack::{PackOptions, pack};
use std::{fs, path::Path, process::Command};

/// Creates a Java 25 run context.
fn context() -> Context {
    Context {
        os: "linux".into(),
        arch: "x86-64".into(),
        invocation: Some("run".into()),
        runtime: Some(RuntimeContext {
            runtime_type: "janex.java".into(),
            java_version: Some(JavaVersion::parse("25").unwrap()),
            vendor: String::new(),
        }),
    }
}

/// Opens and verifies a generated package before evaluating its only application.
fn open(path: &Path) -> (JavaLaunch, BlobStore<fs::File>) {
    let mut reader = Reader::open_auto(fs::File::open(path).unwrap(), Limits::default()).unwrap();
    assert!(matches!(reader.verification(), Verification::Checksum(_)));
    assert!(reader.verify_checksums().unwrap().complete_secure_coverage);
    let apps = read_applications(&mut reader).unwrap();
    assert_eq!(apps.len(), 1);
    let launch = apps[0].evaluate_java(&context()).unwrap().unwrap();
    (launch, BlobStore::new(reader))
}

/// Resolves one local root from an evaluated path.
fn root(entry: &PathEntry, blobs: &mut BlobStore<fs::File>) -> ResourceRoot {
    let PathEntry::Local(reference) = entry else {
        panic!("expected local root")
    };
    let bytes = blobs.resolve(*reference).unwrap();
    ResourceRoot::decode(&bytes, blobs).unwrap()
}

/// Runs a JDK tool, retaining diagnostics when the command fails.
fn tool(directory: &Path, program: &str, arguments: &[&str]) -> std::process::Output {
    let output = Command::new(program)
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("JDK tools must be on PATH");
    assert!(
        output.status.success(),
        "{program} {arguments:?}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn packages_directory_resources_arguments_and_ordered_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["main", "cp1", "cp2", "mp"] {
        fs::create_dir(temp.path().join(name)).unwrap();
    }
    let shared = b"identical untransformed file contents";
    fs::write(temp.path().join("main/a.bin"), shared).unwrap();
    fs::write(temp.path().join("main/b.bin"), shared).unwrap();
    fs::write(temp.path().join("main/empty"), b"").unwrap();
    fs::write(
        temp.path().join("main/invalid.class"),
        b"ordinary non-class resource",
    )
    .unwrap();
    fs::write(temp.path().join("cp1/marker"), b"one").unwrap();
    fs::write(temp.path().join("cp2/marker"), b"two").unwrap();
    let output = temp.path().join("app.janex");
    let mut options = PackOptions::new(temp.path().join("main"), &output);
    options.main_class = Some("example.Main".into());
    options.application = "application".into();
    options.class_path = vec![temp.path().join("cp1"), temp.path().join("cp2")];
    options.module_path = vec![temp.path().join("mp")];
    options.jvm_options = vec!["-ea".into(), "-Dkey=one value".into()];
    options.arguments = vec![
        "".into(),
        "one value".into(),
        "--flag".into(),
        "\u{4e2d}".into(),
    ];
    options.java_version = Some("vers:jep322/>=21|<26".into());
    let report = pack(&options).unwrap();
    assert_eq!(report.resource_roots, 4);
    assert_eq!(report.file_bytes, fs::metadata(&output).unwrap().len());
    assert_eq!(report.classfiles_transformed, 0);
    let (launch, mut blobs) = open(&output);
    assert_eq!(launch.jvm_options, options.jvm_options);
    assert_eq!(launch.arguments, options.arguments);
    assert_eq!(launch.class_path.len(), 3);
    assert_eq!(launch.module_path.len(), 1);
    let resource = root(&launch.class_path[0], &mut blobs);
    let tree = resource.merge(&context(), Limits::default()).unwrap();
    assert_eq!(tree.read_file("a.bin", &mut blobs).unwrap(), shared);
    assert_eq!(
        tree.read_file("invalid.class", &mut blobs).unwrap(),
        b"ordinary non-class resource"
    );
    let Node::File { content: a, .. } = tree.get("a.bin").unwrap() else {
        panic!("expected file")
    };
    let Node::File { content: b, .. } = tree.get("b.bin").unwrap() else {
        panic!("expected file")
    };
    assert!(matches!((&a.source, &b.source), (Source::Blob(a), Source::Blob(b)) if a == b));
    assert!(
        matches!(tree.get("empty"), Some(Node::File { content, .. }) if matches!(&content.source, Source::Inline(bytes) if bytes.is_empty()))
    );
    for (index, expected) in [(1, b"one".as_slice()), (2, b"two".as_slice())] {
        assert_eq!(
            root(&launch.class_path[index], &mut blobs)
                .merge(&context(), Limits::default())
                .unwrap()
                .read_file("marker", &mut blobs)
                .unwrap(),
            expected
        );
    }
    options.output = temp.path().join("again.janex");
    pack(&options).unwrap();
    assert_eq!(fs::read(output).unwrap(), fs::read(options.output).unwrap());
}

#[test]
fn errors_and_concurrent_publication_never_replace_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let output = temp.path().join("app.janex");
    let mut options = PackOptions::new(&source, &output);
    assert!(pack(&options).is_err());
    assert!(!output.exists());
    options.main_class = Some("Main".into());
    options.java_version = Some("invalid".into());
    assert!(pack(&options).is_err());
    assert!(!output.exists());
    options.java_version = None;
    fs::write(&output, b"existing output").unwrap();
    assert!(pack(&options).is_err());
    assert_eq!(fs::read(&output).unwrap(), b"existing output");
    let race = temp.path().join("race.janex");
    options.output = race.clone();
    std::thread::scope(|scope| {
        let a = scope.spawn(|| pack(&options));
        let b = scope.spawn(|| pack(&options));
        assert_ne!(a.join().unwrap().is_ok(), b.join().unwrap().is_ok());
    });
    open(&race);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 3);
}

#[test]
fn real_classes_transform_only_when_the_complete_package_is_smaller() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("src/sample")).unwrap();
    let mut sources = String::new();
    for index in 0..32 {
        let path = format!("src/sample/C{index}.java");
        fs::write(temp.path().join(&path), format!("package sample; public class C{index} {{ public static String value() {{ return \"a long repeated resource string shared by many classes\"; }} }}")).unwrap();
        sources.push_str(&path);
        sources.push('\n');
    }
    fs::write(temp.path().join("src/sample/Main.java"), "package sample; public class Main { public static void main(String[] args) { System.out.println(C0.value()); } }").unwrap();
    sources.push_str("src/sample/Main.java\n");
    fs::write(temp.path().join("sources.txt"), sources).unwrap();
    tool(
        temp.path(),
        "javac",
        &["--release", "11", "-d", "classes", "@sources.txt"],
    );
    let mut options = PackOptions::new(
        temp.path().join("classes"),
        temp.path().join("normal.janex"),
    );
    options.main_class = Some("sample.Main".into());
    options.transform_classfiles = false;
    let ordinary = pack(&options).unwrap();
    options.output = temp.path().join("compact.janex");
    options.transform_classfiles = true;
    let compact = pack(&options).unwrap();
    assert!(compact.file_bytes < ordinary.file_bytes);
    assert!(compact.classfiles_transformed > 0);
    let (launch, mut blobs) = open(&options.output);
    let root = root(&launch.class_path[0], &mut blobs);
    let tree = root.merge(&context(), Limits::default()).unwrap();
    fs::create_dir_all(temp.path().join("restored/sample")).unwrap();
    for (path, node) in tree.entries() {
        if matches!(node, Node::File { .. }) {
            let bytes = tree.read_file(path, &mut blobs).unwrap();
            assert_eq!(
                bytes,
                fs::read(temp.path().join("classes").join(path)).unwrap()
            );
            fs::write(temp.path().join("restored").join(path), bytes).unwrap();
        }
    }
    let output = tool(temp.path(), "java", &["-cp", "restored", "sample.Main"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "a long repeated resource string shared by many classes"
    );
    // A lone short class often costs more with its external pool; complete-file comparison decides.
    fs::create_dir(temp.path().join("single")).unwrap();
    fs::copy(
        temp.path().join("classes/sample/C0.class"),
        temp.path().join("single/C0.class"),
    )
    .unwrap();
    options.source = temp.path().join("single");
    options.main_class = Some("sample.C0".into());
    options.output = temp.path().join("single-normal.janex");
    options.transform_classfiles = false;
    let ordinary = pack(&options).unwrap();
    options.output = temp.path().join("single-auto.janex");
    options.transform_classfiles = true;
    assert!(pack(&options).unwrap().file_bytes <= ordinary.file_bytes);
}

#[test]
fn jar_and_module_descriptor_entry_points_are_inferred() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("src/sample")).unwrap();
    fs::write(
        temp.path().join("src/sample/Main.java"),
        "package sample; public class Main { public static void main(String[] args) {} }",
    )
    .unwrap();
    fs::write(
        temp.path().join("src/module-info.java"),
        "module sample.app { exports sample; }",
    )
    .unwrap();
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "11",
            "-d",
            "classes",
            "src/module-info.java",
            "src/sample/Main.java",
        ],
    );
    tool(
        temp.path(),
        "jar",
        &[
            "--create",
            "--file",
            "original-name.jar",
            "--main-class",
            "sample.Main",
            "-C",
            "classes",
            ".",
        ],
    );
    let mut options = PackOptions::new(
        temp.path().join("original-name.jar"),
        temp.path().join("app.janex"),
    );
    pack(&options).unwrap();
    let (launch, mut blobs) = open(&options.output);
    assert_eq!(
        launch.entry_point.main_class.as_deref(),
        Some("sample.Main")
    );
    assert_eq!(
        root(&launch.class_path[0], &mut blobs).jar_name().unwrap(),
        "original-name.jar"
    );
    options.output = temp.path().join("module.janex");
    options.main_module = Some("sample.app".into());
    pack(&options).unwrap();
    let (launch, _) = open(&options.output);
    assert!(launch.class_path.is_empty());
    assert_eq!(launch.module_path.len(), 1);
    assert_eq!(
        launch.entry_point.main_module.as_deref(),
        Some("sample.app")
    );
    fs::create_dir(temp.path().join("descriptor")).unwrap();
    tool(
        &temp.path().join("descriptor"),
        "jar",
        &[
            "--extract",
            "--file",
            "../original-name.jar",
            "module-info.class",
        ],
    );
    options.source = temp.path().join("descriptor");
    options.output = temp.path().join("inferred.janex");
    options.main_module = None;
    pack(&options).unwrap();
    assert_eq!(
        open(&options.output).0.entry_point.main_class.as_deref(),
        Some("sample.Main")
    );
    options.main_class = Some("Explicit".into());
    options.output = temp.path().join("explicit.janex");
    pack(&options).unwrap();
    assert_eq!(
        open(&options.output).0.entry_point.main_class.as_deref(),
        Some("Explicit")
    );
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Complete local package launches against a real JDK.

use janex_core::{
    java::{JavaOptions, JavaRuntime, candidates},
    pack::{PackOptions, pack},
    run::{ExecutionPlan, LaunchMode, RunOptions, prepare},
};
use janex_format::{
    application::{Application, PathEntry},
    binary::Limits,
    cbor::Value,
    container::{APPLICATION, Reader, Writer},
};
use std::{
    fs,
    io::Cursor,
    path::Path,
    process::{Command, Output, Stdio},
};

/// Runs a JDK fixture tool with diagnostics retained on failure.
fn tool(directory: &Path, program: &str, arguments: &[&str]) -> Output {
    let output = Command::new(program)
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("JDK tools must be on PATH");
    assert!(
        output.status.success(),
        "{program} {arguments:?} in {}:\n{}\n{}",
        directory.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Selects PATH's Java explicitly and permits unsigned local fixtures.
fn options(path: &Path) -> RunOptions {
    let mut options = RunOptions::new(path);
    options.java.java = Some("java".into());
    options.allow_unsigned = true;
    options
}

/// Captures one prepared invocation while retaining its resource lifetime.
fn capture(plan: &ExecutionPlan) -> Output {
    plan.command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

/// Replaces an integer-keyed descriptor field without discarding other fields.
fn replace(value: Value, key: u64, replacement: Value) -> Value {
    let mut fields = value.as_map().unwrap();
    fields.retain(|(field, _)| field.as_u64().unwrap() != key);
    fields.push((Value::uint(key), replacement));
    Value::map(fields).unwrap()
}

/// Rewrites a fixture's launch configuration while regenerating its section and metadata checksums.
fn change_launch(path: &Path, mut change: impl FnMut(Value) -> Value) {
    let mut reader =
        Reader::open_auto(Cursor::new(fs::read(path).unwrap()), Limits::default()).unwrap();
    let sections: Vec<_> = reader.sections().cloned().collect();
    let mut writer = Writer::new(Vec::new()).unwrap();
    for section in sections {
        let type_info = section.type_info().unwrap();
        let mut bytes = reader.read_section(section.id()).unwrap();
        if section.kind() == APPLICATION {
            let app =
                Application::decode(&bytes, type_info.clone().unwrap(), Limits::default()).unwrap();
            let descriptor = app.value().required(0).unwrap();
            let config = change(descriptor.required(0).unwrap());
            let descriptor = replace(descriptor, 0, config);
            let value = replace(app.value().clone(), 0, descriptor);
            bytes = Application::from_values(app.type_info().clone(), value, Limits::default())
                .unwrap()
                .encode()
                .unwrap();
        }
        writer
            .write_section(section.id(), section.kind(), &bytes, type_info)
            .unwrap();
    }
    let mut metadata = reader.metadata().as_map().unwrap();
    metadata.retain(|(key, _)| key.as_u64().unwrap() != 0);
    fs::write(path, writer.finish(Value::map(metadata).unwrap()).unwrap()).unwrap();
}

#[test]
fn directory_launch_preserves_arguments_snapshot_and_exit_status() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), r#"
import java.nio.charset.StandardCharsets;
import java.util.Base64;
public class Main {
    public static void main(String[] args) {
        System.out.println(System.getProperty("sample"));
        for (String arg : args) {
            System.out.println("arg:" + Base64.getEncoder().encodeToString(arg.getBytes(StandardCharsets.UTF_8)));
        }
        System.exit(37);
    }
}
"#).unwrap();
    tool(
        temp.path(),
        "javac",
        &["--release", "8", "-d", "classes", "Main.java"],
    );
    let target = temp.path().join("unicode \u{4e2d}.janex");
    let mut packing = PackOptions::new(temp.path().join("classes"), &target);
    packing.main_class = Some("Main".into());
    packing.jvm_options = vec!["-Dsample=two words".into()];
    packing.arguments = vec!["preset".into(), "".into()];
    pack(&packing).unwrap();
    let mut options = options(&target);
    options.arguments = [
        "--java",
        "@missing-argfile",
        "\u{4e2d}\u{1f680}",
        "two words",
        "\"quoted\"",
        "C:\\tail\\",
        "--disable-@files",
        "@@double",
        "",
    ]
    .map(Into::into)
    .into();
    let plan = prepare(&options).unwrap();
    assert!(plan.integrity().complete_secure_coverage);
    let directory = plan.directory().to_owned();
    assert!(directory.is_dir());
    fs::write(&target, b"source changed after preparation").unwrap();
    let output = capture(&plan);
    assert_eq!(
        output.status.code(),
        Some(37),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "two words\narg:cHJlc2V0\narg:\narg:LS1qYXZh\narg:QG1pc3NpbmctYXJnZmlsZQ==\narg:5Lit8J+agA==\narg:dHdvIHdvcmRz\narg:InF1b3RlZCI=\narg:QzpcdGFpbFw=\narg:LS1kaXNhYmxlLUBmaWxlcw==\narg:QEBkb3VibGU=\narg:\n"
    );
    drop(plan);
    assert!(!directory.exists());
}

#[test]
fn java_8_launches_classpath_applications() {
    let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") else {
        eprintln!("set JANEX_TEST_JAVA8_HOME to run the Java 8 integration fixture");
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), "class Main { public static void main(String... args) { for (String arg : args) System.out.println(java.util.Base64.getEncoder().encodeToString(arg.getBytes(java.nio.charset.StandardCharsets.UTF_8))); } }").unwrap();
    tool(
        temp.path(),
        "javac",
        &["--release", "8", "-d", "classes", "Main.java"],
    );
    let mut packing =
        PackOptions::new(temp.path().join("classes"), temp.path().join("java8.janex"));
    packing.main_class = Some("Main".into());
    packing.java_version = Some("vers:jep322/>=8|<9".into());
    pack(&packing).unwrap();
    let mut options = options(&packing.output);
    options.java = JavaOptions {
        java: None,
        java_home: Some(home.into()),
    };
    options.arguments = ["", "two words", "@literal", "--flag", "\u{4e2d}\u{1f680}"]
        .map(Into::into)
        .into();
    let plan = prepare(&options).unwrap();
    assert_eq!(plan.runtime().version.feature(), 8);
    let output = capture(&plan);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "\ndHdvIHdvcmRz\nQGxpdGVyYWw=\nLS1mbGFn\n5Lit8J+agA==\n"
    );
    options.launch_mode = LaunchMode::Direct;
    options.arguments.pop();
    let direct = prepare(&options).unwrap();
    assert_eq!(direct.launch_mode(), LaunchMode::Direct);
    assert!(!direct.directory().join("bootstrap.jar").exists());
    assert_eq!(
        String::from_utf8(capture(&direct).stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "\ndHdvIHdvcmRz\nQGxpdGVyYWw=\nLS1mbGFn\n"
    );
}

#[test]
fn both_launch_modes_support_modern_main_methods_and_uncaught_exceptions() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("Instance.java"),
        "class Instance { void main(String... args) { System.out.println(args.length); } }",
    )
    .unwrap();
    fs::write(
        temp.path().join("NoArgs.java"),
        "class NoArgs { void main() { System.out.println(\"no-args\"); } }",
    )
    .unwrap();
    fs::write(temp.path().join("Failure.java"), "class Failure { public static void main(String[] args) { throw new IllegalStateException(\"application-failure\"); } }").unwrap();
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "25",
            "-d",
            "classes",
            "Instance.java",
            "NoArgs.java",
            "Failure.java",
        ],
    );
    for (main, expected) in [("Instance", "2"), ("NoArgs", "no-args"), ("Failure", "")] {
        let mut packing = PackOptions::new(
            temp.path().join("classes"),
            temp.path().join(format!("{main}.janex")),
        );
        packing.main_class = Some(main.into());
        pack(&packing).unwrap();
        for mode in [LaunchMode::Bootstrap, LaunchMode::Direct] {
            let mut options = options(&packing.output);
            options.launch_mode = mode;
            options.arguments = vec!["".into(), "two words".into()];
            let plan = prepare(&options).unwrap();
            assert_eq!(plan.launch_mode(), mode);
            let output = capture(&plan);
            let error = String::from_utf8_lossy(&output.stderr);
            if main == "Failure" {
                assert!(!output.status.success());
                assert!(
                    error.contains("java.lang.IllegalStateException: application-failure"),
                    "{error}"
                );
                assert!(!error.contains("InvocationTargetException"), "{error}");
            } else {
                assert!(output.status.success(), "{error}");
                assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
            }
        }
    }
}

#[test]
fn bootstrap_preserves_utf16_code_units_and_arguments_beyond_native_command_limits() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("Units.java"),
        r#"
class Units {
    public static void main(String[] args) {
        for (String arg : args) {
            System.out.print(arg.length());
            if (arg.length() <= 8) {
                for (char unit : arg.toCharArray()) System.out.printf(":%04x", (int) unit);
            }
            System.out.println();
        }
    }
}
"#,
    )
    .unwrap();
    tool(
        temp.path(),
        "javac",
        &["--release", "8", "-d", "classes", "Units.java"],
    );
    let mut packing =
        PackOptions::new(temp.path().join("classes"), temp.path().join("units.janex"));
    packing.main_class = Some("Units".into());
    packing.arguments = vec!["".into(), "\0".into(), "\u{1f680}".repeat(20_000)];
    pack(&packing).unwrap();
    let mut options = options(&packing.output);
    options.arguments = vec!["\u{4e2d}\u{1f680}".into()];
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        options
            .arguments
            .push(std::ffi::OsString::from_wide(&[0xd800, 0x61, 0xdc00]));
    }
    let plan = prepare(&options).unwrap();
    assert!(
        plan.arguments()
            .iter()
            .all(|argument| argument.len() < 10_000)
    );
    let output = capture(&plan);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = if cfg!(windows) {
        "0\n1:0000\n40000\n3:4e2d:d83d:de80\n3:d800:0061:dc00\n"
    } else {
        "0\n1:0000\n40000\n3:4e2d:d83d:de80\n"
    };
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        expected
    );
    options.launch_mode = LaunchMode::Direct;
    assert!(prepare(&options).unwrap_err().to_string().contains("NUL"));
}

#[test]
fn local_file_uri_and_execution_policy_fail_before_running_java() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("classes");
    fs::create_dir(&source).unwrap();
    let target = temp.path().join("space \u{4e2d}.janex");
    let mut packing = PackOptions::new(&source, &target);
    packing.main_class = Some("Main".into());
    pack(&packing).unwrap();
    let mut options = options(&target);
    options.target = url::Url::from_file_path(&target).unwrap().as_str().into();
    let plan = prepare(&options).unwrap();
    assert!(plan.runtime().executable.is_absolute());
    options.allow_unsigned = false;
    assert!(
        prepare(&options)
            .unwrap_err()
            .to_string()
            .contains("--allow-unsigned")
    );
    options.allow_unsigned = true;
    options.max_snapshot_bytes = 1;
    assert!(
        prepare(&options)
            .unwrap_err()
            .to_string()
            .contains("snapshot byte limit")
    );
    options.max_snapshot_bytes = 1024 * 1024;
    options.java.java = Some(temp.path().join("missing-java"));
    assert!(prepare(&options).is_err());
    for target in [
        "https://example.invalid/app.janex",
        "file://remote.invalid/app.janex",
        "file:///tmp/app.janex?query",
        "file:///tmp/%XX",
    ] {
        options.target = target.into();
        assert!(prepare(&options).is_err(), "{target}");
    }
    packing.output = temp.path().join("future.janex");
    packing.java_version = Some("vers:jep322/>=999".into());
    pack(&packing).unwrap();
    let mut future = RunOptions::new(&packing.output);
    future.allow_unsigned = true;
    future.java.java = Some("java".into());
    assert!(
        prepare(&future)
            .unwrap_err()
            .to_string()
            .contains("conditions do not match")
    );
}

#[test]
fn rejects_corruption_and_never_downgrades_signed_input() {
    let temp = tempfile::tempdir().unwrap();
    for kind in [2, 3] {
        let bytes = Writer::new(Vec::new())
            .unwrap()
            .finish_with(Value::empty_map(), kind, |_| Ok(vec![1]))
            .unwrap();
        let path = temp.path().join(format!("signed-{kind}.janex"));
        fs::write(&path, bytes).unwrap();
        let mut options = options(&path);
        options.java.java = Some(temp.path().join("must-not-start-java"));
        match prepare(&options).unwrap_err() {
            janex_core::Error::Format(error) if kind == 2 => {
                assert_eq!(error.kind(), janex_format::ErrorKind::Trust)
            }
            janex_core::Error::Format(error) if kind == 3 => {
                assert_eq!(error.kind(), janex_format::ErrorKind::Invalid)
            }
            error => panic!("unexpected signed-input result: {error}"),
        }
    }
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("marker"), b"payload").unwrap();
    let mut packing = PackOptions::new(&source, temp.path().join("corrupt.janex"));
    packing.main_class = Some("Main".into());
    pack(&packing).unwrap();
    let mut bytes = fs::read(&packing.output).unwrap();
    let index = bytes
        .windows(8)
        .position(|bytes| bytes == b"JANEXAPP")
        .unwrap();
    bytes[index + 8] ^= 1;
    fs::write(&packing.output, bytes).unwrap();
    let mut options = options(&packing.output);
    options.java.java = Some(temp.path().join("must-not-start-java"));
    assert!(
        prepare(&options)
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
}

#[test]
fn jar_launch_merges_multi_release_resources_and_uses_explicit_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("Dependency.java"),
        "public class Dependency { public static String value() { return \"dependency\"; } }",
    )
    .unwrap();
    fs::write(temp.path().join("Main.java"), r#"
public class Main {
    public static void main(String[] args) throws Exception {
        System.out.println(Dependency.value());
        try (java.io.InputStream in = Main.class.getResourceAsStream("/version.txt")) {
            System.out.println(new String(in.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8));
        }
    }
}
"#).unwrap();
    tool(
        temp.path(),
        "javac",
        &["--release", "11", "-d", "dependency", "Dependency.java"],
    );
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "11",
            "-cp",
            temp.path().join("dependency").to_str().unwrap(),
            "-d",
            "main",
            "Main.java",
        ],
    );
    fs::create_dir_all(temp.path().join("main/META-INF/versions/9")).unwrap();
    fs::create_dir_all(temp.path().join("main/META-INF/versions/999")).unwrap();
    fs::write(temp.path().join("main/version.txt"), b"base").unwrap();
    fs::write(
        temp.path().join("main/META-INF/versions/9/version.txt"),
        b"selected",
    )
    .unwrap();
    fs::write(
        temp.path().join("main/META-INF/versions/999/version.txt"),
        b"future",
    )
    .unwrap();
    fs::write(temp.path().join("manifest.mf"), b"Manifest-Version: 1.0\nMain-Class: Main\nMulti-Release: true\nClass-Path: forbidden.jar\n\n").unwrap();
    tool(
        temp.path(),
        "jar",
        &[
            "--create",
            "--file",
            "main.jar",
            "--manifest",
            "manifest.mf",
            "-C",
            "main",
            ".",
        ],
    );
    tool(
        temp.path(),
        "jar",
        &[
            "--create",
            "--file",
            "dependency.jar",
            "-C",
            "dependency",
            ".",
        ],
    );
    let mut packing = PackOptions::new(temp.path().join("main.jar"), temp.path().join("app.janex"));
    packing.class_path.push(temp.path().join("dependency.jar"));
    pack(&packing).unwrap();
    let plan = prepare(&options(&packing.output)).unwrap();
    let output = capture(&plan);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "dependency\nselected\n"
    );
}

#[test]
fn module_launch_uses_filename_derived_names_and_reports_missing_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("src/library")).unwrap();
    fs::create_dir_all(temp.path().join("src/app")).unwrap();
    fs::write(temp.path().join("src/library/Library.java"), "package library; public class Library { public static String value() { return \"automatic\"; } public static void main(String[] args) { System.out.println(value()); } }").unwrap();
    fs::write(
        temp.path().join("src/module-info.java"),
        "module sample.app { requires auto.library; }",
    )
    .unwrap();
    fs::write(temp.path().join("src/app/Main.java"), "package app; public class Main { public static void main(String[] args) { System.out.println(library.Library.value()); for (String arg : args) System.out.println(java.util.Base64.getEncoder().encodeToString(arg.getBytes(java.nio.charset.StandardCharsets.UTF_8))); } }").unwrap();
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "11",
            "-d",
            "library",
            "src/library/Library.java",
        ],
    );
    tool(
        temp.path(),
        "jar",
        &[
            "--create",
            "--file",
            "auto-library-1.2.jar",
            "-C",
            "library",
            ".",
        ],
    );
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "11",
            "--module-path",
            temp.path().join("auto-library-1.2.jar").to_str().unwrap(),
            "-d",
            "main",
            "src/module-info.java",
            "src/app/Main.java",
        ],
    );
    tool(
        temp.path(),
        "jar",
        &[
            "--create",
            "--file",
            "main.jar",
            "--main-class",
            "app.Main",
            "-C",
            "main",
            ".",
        ],
    );
    let mut packing = PackOptions::new(temp.path().join("main.jar"), temp.path().join("app.janex"));
    packing.main_module = Some("sample.app".into());
    packing
        .module_path
        .push(temp.path().join("auto-library-1.2.jar"));
    pack(&packing).unwrap();
    change_launch(&packing.output, |config| {
        replace(
            config,
            1,
            Value::map([(Value::uint(1), Value::text("sample.app"))]).unwrap(),
        )
    });
    let mut unicode_options = options(&packing.output);
    unicode_options.arguments = vec!["\u{4e2d}\u{1f680}".into()];
    let plan = prepare(&unicode_options).unwrap();
    let output = capture(&plan);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "automatic\n5Lit8J+agA==\n"
    );
    let mut direct_options = options(&packing.output);
    direct_options.launch_mode = LaunchMode::Direct;
    let direct = capture(&prepare(&direct_options).unwrap());
    assert!(
        direct.status.success(),
        "{}",
        String::from_utf8_lossy(&direct.stderr)
    );
    assert_eq!(
        String::from_utf8(direct.stdout).unwrap().trim(),
        "automatic"
    );
    let mut automatic = PackOptions::new(
        temp.path().join("auto-library-1.2.jar"),
        temp.path().join("automatic-main.janex"),
    );
    automatic.main_module = Some("auto.library".into());
    automatic.main_class = Some("library.Library".into());
    pack(&automatic).unwrap();
    let output = capture(&prepare(&options(&automatic.output)).unwrap());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "automatic"
    );
    let paths = vec![temp.path().join("auto-library-1.2.jar")];
    let runtime = JavaRuntime::probe(
        &candidates(&JavaOptions {
            java: Some("java".into()),
            java_home: None,
        })
        .unwrap()[0],
    )
    .unwrap();
    assert_eq!(
        runtime.validate_module_path(&paths).unwrap()["auto.library"].as_deref(),
        Some("1.2")
    );
    change_launch(&packing.output, |config| {
        let mut entries = config.required(2).unwrap().as_array().unwrap();
        for uri in [
            "pkg:janex/java-module/auto.library@1.2",
            "pkg:janex/java-module/java.base",
        ] {
            entries.push(
                PathEntry::External {
                    uri: uri.into(),
                    checksum: None,
                }
                .to_value(true)
                .unwrap(),
            );
        }
        replace(config, 2, Value::array(entries))
    });
    assert!(
        capture(&prepare(&options(&packing.output)).unwrap())
            .status
            .success()
    );
    change_launch(&packing.output, |config| {
        let mut entries = config.required(2).unwrap().as_array().unwrap();
        entries.push(
            PathEntry::External {
                uri: "pkg:janex/java-module/auto.library@2".into(),
                checksum: None,
            }
            .to_value(true)
            .unwrap(),
        );
        replace(config, 2, Value::array(entries))
    });
    assert!(
        prepare(&options(&packing.output))
            .unwrap_err()
            .to_string()
            .contains("required module version")
    );
    packing.module_path.clear();
    packing.output = temp.path().join("missing.janex");
    pack(&packing).unwrap();
    assert!(prepare(&options(&packing.output)).is_err());
}

#[test]
fn agents_run_once_after_preparation_with_unsplit_options() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Agent.java"), r#"
public class Agent {
    public static void premain(String option) throws Exception {
        java.nio.file.Files.write(java.nio.file.Paths.get(System.getProperty("agent.marker")), option.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        System.out.println("agent:" + option);
    }
}
"#).unwrap();
    fs::write(temp.path().join("Main.java"), "public class Main { public static void main(String[] args) { System.out.println(\"main\"); } }").unwrap();
    tool(
        temp.path(),
        "javac",
        &["--release", "8", "-d", "agent", "Agent.java"],
    );
    tool(
        temp.path(),
        "javac",
        &["--release", "8", "-d", "main", "Main.java"],
    );
    fs::write(
        temp.path().join("agent.mf"),
        "Manifest-Version: 1.0\nPremain-Class: Agent\n\n",
    )
    .unwrap();
    tool(
        temp.path(),
        "jar",
        &[
            "--create",
            "--file",
            "agent=tools.jar",
            "--manifest",
            "agent.mf",
            "-C",
            "agent",
            ".",
        ],
    );
    let mut packing = PackOptions::new(temp.path().join("main"), temp.path().join("agent.janex"));
    packing.main_class = Some("Main".into());
    packing.class_path.push(temp.path().join("agent=tools.jar"));
    let marker = temp.path().join("agent-marker");
    packing
        .jvm_options
        .push(format!("-Dagent.marker={}", marker.display()));
    pack(&packing).unwrap();
    change_launch(&packing.output, |config| {
        let entries = config.required(3).unwrap().as_array().unwrap();
        let agent = Value::map([
            (Value::uint(0), entries[1].clone()),
            (Value::uint(1), Value::text("one=two words")),
        ])
        .unwrap();
        replace(config, 4, Value::array([agent]))
    });
    let plan = prepare(&options(&packing.output)).unwrap();
    assert!(!marker.exists());
    let output = capture(&plan);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "agent:one=two words\nmain\n"
    );
    assert_eq!(fs::read(marker).unwrap(), b"one=two words");
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Complete local package launches against a real JDK.

#[path = "support/http.rs"]
mod http;

use janex_format::{
    application::{Application, PathEntry},
    binary::Limits,
    cbor::Value,
    container::{APPLICATION, Reader, Writer},
};
use janex_host::{
    pack::{PackOptions, pack},
    run::{ExecutionPlan, LaunchMode, RunOptions, prepare},
};
use janex_java::runtime::{JavaOptions, JavaRuntime, candidates};
use std::{
    fs,
    io::Cursor,
    path::Path,
    process::{Command, Output, Stdio},
};

#[test]
fn remote_jars_support_both_modes_modules_agents_and_offline_cache() {
    use janex_format::checksum::{Algorithm, Checksum};
    let temp = tempfile::tempdir().unwrap();
    let server = http::Server::new();
    fs::create_dir_all(temp.path().join("src/library")).unwrap();
    fs::create_dir_all(temp.path().join("src/app")).unwrap();
    fs::write(temp.path().join("src/library/Lib.java"), r#"
package library;
public class Lib {
    public static String value(boolean bootstrap) throws Exception {
        java.net.URL resource = Lib.class.getResource("value.txt");
        if (bootstrap) {
            return new String(java.nio.file.Files.readAllBytes(java.nio.file.Paths.get(resource.toURI())), "UTF-8");
        }
        try (java.io.InputStream input = resource.openStream()) {
            java.io.ByteArrayOutputStream output = new java.io.ByteArrayOutputStream();
            for (int value; (value = input.read()) != -1;) {
                output.write(value);
            }
            return new String(output.toByteArray(), "UTF-8");
        }
    }
}
"#).unwrap();
    fs::write(
        temp.path().join("src/app/Main.java"),
        r#"
package app;
public class Main {
    public static void main(String[] args) throws Exception {
        System.setOut(new java.io.PrintStream(System.out, true, "UTF-8"));
        System.out.println(library.Lib.value(Boolean.parseBoolean(args[0])));
        System.out.println(args[1]);
    }
}
"#,
    )
    .unwrap();
    fs::write(temp.path().join("Agent.java"), r#"
public class Agent {
    public static void premain(String option) throws Exception {
        java.nio.file.Files.write(java.nio.file.Paths.get(System.getProperty("agent.marker")),
            option.getBytes("UTF-8"), java.nio.file.StandardOpenOption.CREATE, java.nio.file.StandardOpenOption.APPEND);
    }
}
"#).unwrap();
    tool(
        temp.path(),
        "javac",
        &["--release", "8", "-d", "library", "src/library/Lib.java"],
    );
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "8",
            "-cp",
            "library",
            "-d",
            "app",
            "src/app/Main.java",
        ],
    );
    tool(
        temp.path(),
        "javac",
        &["--release", "8", "-d", "agent", "Agent.java"],
    );
    fs::write(temp.path().join("library/library/value.txt"), b"base").unwrap();
    fs::create_dir_all(temp.path().join("library/META-INF/versions/9/library")).unwrap();
    fs::write(
        temp.path()
            .join("library/META-INF/versions/9/library/value.txt"),
        b"current",
    )
    .unwrap();
    fs::write(
        temp.path().join("library.mf"),
        "Manifest-Version: 1.0\nMulti-Release: true\n\n",
    )
    .unwrap();
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
            "library-1.2.jar",
            "--manifest",
            "library.mf",
            "-C",
            "library",
            ".",
        ],
    );
    tool(
        temp.path(),
        "jar",
        &["--create", "--file", "app.jar", "-C", "app", "."],
    );
    tool(
        temp.path(),
        "jar",
        &[
            "--create",
            "--file",
            "agent.jar",
            "--manifest",
            "agent.mf",
            "-C",
            "agent",
            ".",
        ],
    );
    let library = fs::read(temp.path().join("library-1.2.jar")).unwrap();
    let agent = fs::read(temp.path().join("agent.jar")).unwrap();
    server.file("/maven/example/library/1.2/library-1.2.jar", &library);
    server.file("/agent.jar", &agent);
    let dependency = PathEntry::External {
        uri: "pkg:maven/example/library@1.2".into(),
        checksum: Some(Checksum::compute(Algorithm::Sha256, library.as_slice()).unwrap()),
    };
    let agent = PathEntry::External {
        uri: format!("{}/agent.jar", server.url),
        checksum: Some(Checksum::compute(Algorithm::Sha512, agent.as_slice()).unwrap()),
    };
    let marker = temp.path().join("marker");
    for modular in [false, true] {
        let mut packing = PackOptions::new(
            temp.path().join("app.jar"),
            temp.path().join(format!("app-{modular}.janex")),
        );
        packing.main_class = Some("app.Main".into());
        packing
            .jvm_options
            .push(format!("-Dagent.marker={}", marker.display()));
        if modular {
            packing.main_module = Some("app".into());
            packing.external_module_path.push(dependency.clone());
        } else {
            packing.external_class_path.push(dependency.clone());
        }
        pack(&packing).unwrap();
        change_launch(&packing.output, |config| {
            replace(
                config,
                4,
                Value::array([Value::map([
                    (Value::uint(0), agent.to_value(false).unwrap()),
                    (Value::uint(1), Value::text("once")),
                ])
                .unwrap()]),
            )
        });
        let mut launch = options(&packing.output);
        launch.dependencies.cache_directory = Some(temp.path().join("cache"));
        launch.dependencies.maven_repository = format!("{}/maven", server.url);
        if !modular {
            launch.allow_unsigned = false;
            assert!(prepare(&launch).is_err());
            assert!(server.requests.lock().unwrap().is_empty());
            launch.allow_unsigned = true;
        }
        for mode in [LaunchMode::Bootstrap, LaunchMode::Direct] {
            launch.launch_mode = mode;
            launch.dependencies.offline = !server.requests.lock().unwrap().is_empty();
            launch.arguments = vec![
                (mode == LaunchMode::Bootstrap).to_string().into(),
                "@literal".into(),
            ];
            let plan = prepare(&launch).unwrap();
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
                "current\n@literal\n"
            );
            assert_eq!(fs::read(&marker).unwrap(), b"once");
            fs::remove_file(&marker).unwrap();
        }
        if !modular && let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
            launch.java = JavaOptions {
                java: None,
                java_home: Some(home.into()),
            };
            launch.launch_mode = LaunchMode::Bootstrap;
            launch.arguments = vec!["true".into(), "unicode \u{4e2d}\u{1f680}".into()];
            let plan = prepare(&launch).unwrap();
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
                "base\nunicode \u{4e2d}\u{1f680}\n"
            );
            fs::remove_file(&marker).unwrap();
        }
    }
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    let mut launch = options(&temp.path().join("app-false.janex"));
    launch.dependencies.cache_directory = Some(temp.path().join("cache"));
    launch.dependencies.maven_repository = format!("{}/maven", server.url);
    launch.dependencies.offline = true;
    launch.arguments = vec!["true".into(), "snapshot".into()];
    let plan = prepare(&launch).unwrap();
    let digest = Checksum::compute(Algorithm::Sha256, library.as_slice()).unwrap();
    let hex: String = digest
        .digest()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let cached_library = temp
        .path()
        .join("cache/files/sha256")
        .join(&hex[..2])
        .join(&hex[2..])
        .join("library-1.2.jar");
    assert_eq!(fs::read(&cached_library).unwrap(), library);
    let mut changed = library.clone();
    changed[0] ^= 1;
    fs::write(cached_library, changed).unwrap();
    assert!(!capture(&plan).status.success());
    assert!(
        !marker.exists(),
        "changed dependencies must fail before agent premain"
    );
    assert!(prepare(&launch).is_err());
    assert!(!marker.exists());
}

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
    let output = plan
        .command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    if !output.status.success() {
        eprintln!(
            "Java exited with {}\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output
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
fn directory_launch_preserves_arguments_and_exit_status_without_copying_input() {
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
    assert!(plan.directory().is_none());
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
    let mut changed = fs::read(&target).unwrap();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    fs::write(&target, changed).unwrap();
    let changed_output = capture(&plan);
    assert!(!changed_output.status.success());
    assert!(String::from_utf8_lossy(&changed_output.stderr).contains("Launch file changed"));
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
    assert_eq!(plan.runtime().feature, 8);
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
    assert!(!direct.directory().unwrap().join("bootstrap.jar").exists());
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
    packing.jvm_options.push("-Djanex.launch=overridden".into());
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
            .finish_with::<janex_format::Error>(Value::empty_map(), kind, |_| Ok(vec![1]))
            .unwrap();
        let path = temp.path().join(format!("signed-{kind}.janex"));
        fs::write(&path, bytes).unwrap();
        let mut options = options(&path);
        options.java.java = Some(temp.path().join("must-not-start-java"));
        match prepare(&options).unwrap_err() {
            janex_host::Error::Trust(_) if kind == 2 => {}
            janex_host::Error::Signature(error) if kind == 3 => {
                assert_eq!(error.kind(), janex_signature::ErrorKind::Invalid)
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
    assert!(plan.directory().is_none());
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
    direct_options.arguments = vec![
        "@literal".into(),
        "@@double".into(),
        "--disable-@files".into(),
    ];
    let direct = capture(&prepare(&direct_options).unwrap());
    assert!(
        direct.status.success(),
        "{}",
        String::from_utf8_lossy(&direct.stderr)
    );
    assert_eq!(
        String::from_utf8(direct.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "automatic\nQGxpdGVyYWw=\nQEBkb3VibGU=\nLS1kaXNhYmxlLUBmaWxlcw==\n"
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

#[test]
fn bootstrap_loads_snapshot_resources_services_and_sealed_packages() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("Service.java"),
        "package sample; public interface Service { String value(); }",
    )
    .unwrap();
    fs::write(temp.path().join("Main.java"), r#"
package sample;
import java.io.*;
import java.net.*;
import java.util.*;
public class Main {
    static byte[] read(URL url) throws Exception {
        try (InputStream input = url.openStream(); ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] block = new byte[4096]; int count;
            while ((count = input.read(block)) != -1) output.write(block, 0, count);
            return output.toByteArray();
        }
    }
    static void check(boolean value) { if (!value) throw new AssertionError(); }
    public static void main(String[] args) throws Exception {
        ClassLoader loader = Main.class.getClassLoader();
        check(loader == ClassLoader.getSystemClassLoader());
        check(loader == Thread.currentThread().getContextClassLoader());
        check(Main.class.getPackage().isSealed());
        check("fixture".equals(Main.class.getPackage().getImplementationTitle()));
        URL first = loader.getResource("dupe.txt");
        check("one".equals(new String(read(first), "UTF-8")));
        Enumeration<URL> matches = loader.getResources("dupe.txt");
        check("one".equals(new String(read(matches.nextElement()), "UTF-8")));
        check("two".equals(new String(read(matches.nextElement()), "UTF-8")));
        check(!matches.hasMoreElements());
        check(loader.getResource("META-INF/OLD.SF") == null);
        check(loader.getResource("missing.txt") == null);
        URL escaped = loader.getResource("space # percent % \u4e2d.txt");
        check("escaped".equals(new String(read(escaped), "UTF-8")));
        check(Arrays.equals(read(escaped), read(new URL(escaped.toString()))));
        byte[] large = read(loader.getResource("large.bin"));
        check(large.length == 1024 * 1024);
        for (int i = 0; i < large.length; i++) check(large[i] == (byte) (i % 251));
        Iterator<Service> services = ServiceLoader.load(Service.class).iterator();
        check("provider".equals(services.next().value())); check(!services.hasNext());
        try { Class.forName("sample.Foreign"); throw new AssertionError("sealing ignored"); }
        catch (SecurityException expected) {}
        if (args[0].equals("bootstrap")) {
            check("janex".equals(first.getProtocol()));
            check(first.openConnection().getPermission() == null);
            check("janex".equals(Main.class.getProtectionDomain().getCodeSource().getLocation().getProtocol()));
            check(loader.getResources("").hasMoreElements());
            org.glavo.janex.bootstrap.fs.FileSystemTest.run(escaped);
        }
        System.out.println("resources-ok");
    }
}
"#).unwrap();
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "8",
            "-d",
            "main",
            "Service.java",
            "Main.java",
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join(
                    "janex-bootstrap/src/testFixtures/java/org/glavo/janex/bootstrap/fs/FileSystemTest.java",
                )
                .to_str()
                .unwrap(),
        ],
    );
    fs::write(temp.path().join("Provider.java"), "package other; public class Provider implements sample.Service { public String value() { return \"provider\"; } }").unwrap();
    fs::write(
        temp.path().join("Foreign.java"),
        "package sample; public class Foreign {}",
    )
    .unwrap();
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "8",
            "-cp",
            temp.path().join("main").to_str().unwrap(),
            "-d",
            "dep",
            "Provider.java",
            "Foreign.java",
        ],
    );
    for root in ["main", "dep"] {
        fs::create_dir_all(temp.path().join(root).join("META-INF/services")).unwrap();
    }
    fs::write(temp.path().join("main/META-INF/MANIFEST.MF"), "Manifest-Version: 1.0\nSealed: true\nImplementation-Title: fixture\nClass-Path: absent.jar\n\n").unwrap();
    fs::write(temp.path().join("main/META-INF/OLD.SF"), b"not retained").unwrap();
    fs::write(
        temp.path().join("dep/META-INF/services/sample.Service"),
        "# comment\nother.Provider\n",
    )
    .unwrap();
    fs::write(temp.path().join("main/dupe.txt"), b"one").unwrap();
    fs::write(temp.path().join("dep/dupe.txt"), b"two").unwrap();
    fs::write(
        temp.path().join("main/space # percent % \u{4e2d}.txt"),
        b"escaped",
    )
    .unwrap();
    fs::write(
        temp.path().join("main/large.bin"),
        (0..1024 * 1024)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let mut packing = PackOptions::new(
        temp.path().join("main"),
        temp.path().join("resources.janex"),
    );
    packing.main_class = Some("sample.Main".into());
    packing.class_path.push(temp.path().join("dep"));
    pack(&packing).unwrap();
    for mode in [LaunchMode::Bootstrap, LaunchMode::Direct] {
        let mut running = options(&packing.output);
        running.launch_mode = mode;
        running.arguments = vec![
            if mode == LaunchMode::Bootstrap {
                "bootstrap"
            } else {
                "direct"
            }
            .into(),
        ];
        let plan = prepare(&running).unwrap();
        if mode == LaunchMode::Bootstrap {
            assert!(plan.directory().is_none());
        }
        let output = capture(&plan);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "resources-ok"
        );
        if mode == LaunchMode::Bootstrap
            && let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME")
        {
            running.java.java = None;
            running.java.java_home = Some(home.into());
            let output = capture(&prepare(&running).unwrap());
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().trim(),
                "resources-ok"
            );
        }
    }
    let mut running = options(&packing.output);
    running.max_materialized_bytes = 100;
    let output = capture(&prepare(&running).unwrap());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("resource byte limit"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("resources-ok"));
}

#[test]
fn indexed_modules_expose_readers_filesystems_services_and_access_options() {
    let temp = tempfile::tempdir().unwrap();
    for path in [
        "src/service.api/api",
        "src/service.provider/impl",
        "src/sample.app/app",
        "extra/outside",
        "agent",
    ] {
        fs::create_dir_all(temp.path().join(path)).unwrap();
    }
    for (path, text) in [
        (
            "src/service.api/module-info.java",
            "module service.api { exports api; }",
        ),
        (
            "src/service.api/api/Greeting.java",
            "package api; public interface Greeting { String value(); }",
        ),
        (
            "src/service.provider/module-info.java",
            "module service.provider { requires service.api; provides api.Greeting with impl.GreetingImpl; }",
        ),
        (
            "src/service.provider/impl/GreetingImpl.java",
            "package impl; public class GreetingImpl implements api.Greeting { private String secret = \"private\"; public String value() { return \"service\"; } }",
        ),
        (
            "src/sample.app/module-info.java",
            "module sample.app { requires service.api; uses api.Greeting; }",
        ),
        (
            "extra/outside/Helper.java",
            "package outside; public class Helper {}",
        ),
    ] {
        fs::write(temp.path().join(path), text).unwrap();
    }
    fs::write(temp.path().join("src/sample.app/app/Main.java"), r#"
package app;
import java.io.*;
import java.lang.module.*;
import java.nio.*;
import java.nio.file.*;
import java.util.*;
public class Main {
    static void check(boolean value) { if (!value) throw new AssertionError(); }
    public static void main(String[] args) throws Exception {
        Module self = Main.class.getModule();
        ClassLoader loader = ClassLoader.getSystemClassLoader();
        check(self.getLayer() != ModuleLayer.boot());
        check(Main.class.getClassLoader() == loader && Thread.currentThread().getContextClassLoader() == loader);
        check("once".equals(System.getProperty("agent.executed")));
        api.Greeting service = ServiceLoader.load(api.Greeting.class).iterator().next();
        check("service".equals(service.value()));
        check(ServiceLoader.load(self.getLayer(), api.Greeting.class).iterator().hasNext());
        Module provider = service.getClass().getModule();
        check(self.canRead(provider) && provider.isExported("impl", self));
        java.lang.reflect.Field field = service.getClass().getDeclaredField("secret");
        field.setAccessible(true); check(field.get(service).equals("private"));
        String.class.getDeclaredField("value").setAccessible(true);
        Module foreign = new ClassLoader() {}.getUnnamedModule();
        check(self.canRead(foreign) && self.isExported("app", foreign) && self.isOpen("app", foreign));
        check((Boolean) Module.class.getMethod("isNativeAccessEnabled").invoke(self));
        check(!Class.forName("outside.Helper").getModule().isNamed());
        check(loader.getResource("impl/secret.txt") == null);
        try (InputStream input = provider.getResourceAsStream("impl/secret.txt")) { check(input.read() == 's'); }
        ModuleReference reference = self.getLayer().configuration().findModule("sample.app").get().reference();
        check(reference.location().get().getScheme().equals("janex"));
        check(Files.isDirectory(Paths.get(reference.location().get())));
        ModuleReader reader = reference.open();
        ModuleReader independent = reference.open();
        check(reader.find("missing").isEmpty());
        check(reader.find("").isEmpty());
        check(reader.find("app/../app/data.txt").isEmpty());
        check(reader.find("app").get().getPath().endsWith("/"));
        check(reader.find("app/").isPresent());
        try (java.util.stream.Stream<String> names = reader.list()) { check(names.anyMatch("app/"::equals)); }
        java.net.URI uri = reader.find("app/data.txt").get();
        check("payload".equals(new String(Files.readAllBytes(Paths.get(uri)), "UTF-8")));
        check(uri.toURL().openStream().read() == 'p');
        ByteBuffer data = reader.read("app/data.txt").get();
        check(data.isReadOnly() && data.remaining() == 7 && data.get(0) == 'p');
        reader.release(data); reader.close(); reader.close();
        check(data.get(0) == 'p');
        try { reader.find("app/data.txt"); throw new AssertionError(); } catch (IOException expected) {}
        check(independent.read("app/data.txt").get().remaining() == 7); independent.close();
        check(loader.getResource("app/data.txt") == null);
        System.out.println("modules-ok");
    }
}
"#).unwrap();
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "11",
            "--module-source-path",
            "src",
            "-d",
            "modules",
            "--module",
            "service.api,service.provider,sample.app",
        ],
    );
    tool(
        temp.path(),
        "javac",
        &["--release", "8", "-d", "extra", "extra/outside/Helper.java"],
    );
    fs::write(
        temp.path().join("modules/sample.app/app/data.txt"),
        b"payload",
    )
    .unwrap();
    fs::write(
        temp.path().join("modules/service.provider/impl/secret.txt"),
        b"secret",
    )
    .unwrap();
    fs::write(temp.path().join("agent/Agent.java"), r#"
public class Agent {
    public static void premain(String value, java.lang.instrument.Instrumentation instrumentation) throws Exception {
        if (System.getProperty("agent.executed") != null) throw new AssertionError();
        Class<?> application = Class.forName("app.Main", false, ClassLoader.getSystemClassLoader());
        if (!application.getModule().getName().equals("sample.app")) throw new AssertionError();
        System.setProperty("agent.executed", "once");
        java.nio.file.Files.write(java.nio.file.Paths.get(value), new byte[] {1});
    }
}
"#).unwrap();
    tool(
        temp.path(),
        "javac",
        &["--release", "11", "-d", "agent", "agent/Agent.java"],
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
            "agent.jar",
            "--manifest",
            "agent.mf",
            "-C",
            "agent",
            ".",
        ],
    );
    let mut packing = PackOptions::new(
        temp.path().join("modules/sample.app"),
        temp.path().join("modules.janex"),
    );
    packing.main_module = Some("sample.app".into());
    packing.main_class = Some("app.Main".into());
    packing.module_path = vec![
        temp.path().join("modules/service.api"),
        temp.path().join("modules/service.provider"),
    ];
    packing.class_path = vec![temp.path().join("extra"), temp.path().join("agent.jar")];
    packing.jvm_options = [
        "--add-reads=sample.app=service.provider,ALL-UNNAMED",
        "--add-exports=service.provider/impl=sample.app",
        "--add-opens=service.provider/impl=sample.app",
        "--add-opens=java.base/java.lang=sample.app",
        "--add-exports=sample.app/app=ALL-UNNAMED",
        "--add-opens=sample.app/app=ALL-UNNAMED",
        "--enable-native-access=sample.app",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    pack(&packing).unwrap();
    let marker = temp.path().join("agent-marker");
    change_launch(&packing.output, |config| {
        let entries = config.required(3).unwrap().as_array().unwrap();
        replace(
            config,
            4,
            Value::array([Value::map([
                (Value::uint(0), entries[1].clone()),
                (Value::uint(1), Value::text(marker.to_str().unwrap())),
            ])
            .unwrap()]),
        )
    });
    let plan = prepare(&options(&packing.output)).unwrap();
    assert!(!marker.exists(), "preparation must not execute agents");
    assert_eq!(fs::read_dir(plan.directory().unwrap()).unwrap().count(), 2);
    let output = capture(&plan);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "modules-ok"
    );
    assert_eq!(fs::read(marker).unwrap(), [1]);
}

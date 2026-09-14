// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Real native wrappers, signature policy, and cross-architecture Java execution.
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use janex_format::{
    application::Application,
    binary::Limits,
    cbor::Value,
    container::{APPLICATION, Reader, Writer},
};
use janex_host::{
    authentication::{self, MATERIAL_LIMITS},
    native_launcher::{self, LaunchOverrides},
    pack::{PackOptions, PackSigner, pack},
    run::LaunchMode,
};
use janex_java::runtime::{JavaOptions, JavaRuntime, candidates};
use std::{
    fs::{self, File},
    io::Cursor,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

/// Compiles a Java 8-compatible fixture reporting architecture, arguments, and packaged resources.
fn fixture(directory: &Path) -> PathBuf {
    let source = directory.join("classes");
    fs::create_dir(&source).unwrap();
    fs::write(directory.join("Main.java"), r#"
import java.io.InputStream;

/// Reports launch properties using ASCII output.
public class Main {
    /// Exercises arguments, packaged resources, and process termination.
    public static void main(String[] args) throws Exception {
        System.out.println("arch=" + System.getProperty("os.arch"));
        System.out.println("bits=" + System.getProperty("sun.arch.data.model"));
        System.out.println("cwd=" + System.getProperty("user.dir"));
        for (String arg : args) {
            StringBuilder value = new StringBuilder();
            for (int i = 0; i < arg.length(); i++) value.append(String.format("%04x", (int) arg.charAt(i)));
            System.out.println("arg=" + value);
        }
        try (InputStream input = Main.class.getResourceAsStream("/marker.txt")) {
            if (input == null || input.read() != 74) throw new AssertionError("Missing packaged resource");
        }
        if (System.getenv("JANEX_TEST_WAIT") != null) {
            System.out.println("ready");
            System.out.flush();
            Thread.sleep(60000);
        }
        System.exit(23);
    }
}
"#).unwrap();
    let output = Command::new("javac")
        .current_dir(directory)
        .args(["--release", "8", "-d", "classes", "Main.java"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(source.join("marker.txt"), b"Janex").unwrap();
    source
}

/// Configures the actual launcher built by Cargo for this test's target architecture.
fn options(source: &Path, output: &Path) -> PackOptions {
    let mut options = PackOptions::new(source, output);
    options.main_class = Some("Main".into());
    options.arguments = vec!["preset".into()];
    options.native_launcher = Some(env!("CARGO_BIN_EXE_janex-launcher").into());
    options
}

/// Starts the packed binary with controlled overrides while preserving normal Java discovery.
fn command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command
        .env_remove("JANEX_JAVA")
        .env_remove("JANEX_LAUNCH_MODE")
        .env_remove("JANEX_TEST_WAIT");
    command
}

/// Verifies the fixture's application exit status rather than requiring success.
fn report(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(23),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// Adds an architecture and invocation condition while retaining the original executable prefix.
fn require_runtime_architecture(path: &Path, arch: &str) {
    let bytes = fs::read(path).unwrap();
    let mut reader = Reader::open_auto(Cursor::new(&bytes), Limits::default()).unwrap();
    let mut writer = Writer::new(bytes[..reader.range().start as usize].to_vec()).unwrap();
    for section in reader.sections().cloned().collect::<Vec<_>>() {
        let mut body = reader.read_section(section.id()).unwrap();
        if section.kind() == APPLICATION {
            let app = Application::decode(
                &body,
                section.type_info().unwrap().unwrap(),
                Limits::default(),
            )
            .unwrap();
            let mut launch = app
                .value()
                .required(0)
                .unwrap()
                .required(0)
                .unwrap()
                .as_map()
                .unwrap();
            launch.push((
                Value::uint(0),
                Value::map([
                    (Value::uint(2), Value::text(arch)),
                    (Value::uint(4), Value::text("open")),
                ])
                .unwrap(),
            ));
            let value = Value::map([(
                Value::uint(0),
                Value::map([(Value::uint(0), Value::map(launch).unwrap())]).unwrap(),
            )])
            .unwrap();
            body = Application::from_values(app.type_info().clone(), value, Limits::default())
                .unwrap()
                .encode()
                .unwrap();
        }
        writer
            .write_section(
                section.id(),
                section.kind(),
                &body,
                section.type_info().unwrap(),
            )
            .unwrap();
    }
    let metadata = Value::map(
        reader
            .metadata()
            .as_map()
            .unwrap()
            .into_iter()
            .filter(|(key, _)| key.as_u64().ok() != Some(0)),
    )
    .unwrap();
    fs::write(path, writer.finish(metadata).unwrap()).unwrap();
}

#[test]
fn executable_preserves_arguments_resources_exit_status_and_native_java() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture(temp.path());
    let path = temp.path().join("application with spaces.exe");
    let options = options(&source, &path);
    pack(&options).unwrap();
    let output = command(&path)
        .current_dir(temp.path())
        .args(["", "--help", "a b", "\u{1f680}"])
        .output()
        .unwrap();
    let text = report(&output);
    assert!(text.contains("arg=\n") || text.contains("arg=\r\n"));
    assert!(text.contains("arg=002d002d00680065006c0070"));
    assert!(text.contains("arg=006100200062"));
    assert!(text.contains("arg=d83dde80"));
    assert!(text.contains("arg=007000720065007300650074"));
    let runtime = JavaRuntime::probe(
        &candidates(&JavaOptions {
            java: Some("java".into()),
            java_home: None,
        })
        .unwrap()[0],
    )
    .unwrap();
    let constrained = temp.path().join("architecture-constrained.exe");
    fs::copy(&path, &constrained).unwrap();
    let path = constrained;
    require_runtime_architecture(&path, &runtime.architecture);
    report(
        &command(&path)
            .env("JANEX_JAVA", &runtime.executable)
            .output()
            .unwrap(),
    );
    if let Ok(expected) = std::env::var("JANEX_TEST_NATIVE_ARCH") {
        assert_eq!(janex_platform::native_architecture().unwrap(), expected);
        assert_eq!(runtime.architecture, expected);
        assert_eq!(
            janex_platform::normalize_architecture(
                text.lines()
                    .find_map(|line| line.strip_prefix("arch="))
                    .unwrap()
            ),
            expected
        );
    }
    #[cfg(all(windows, target_arch = "x86"))]
    if janex_platform::native_architecture().unwrap() != "x86" {
        assert!(
            text.contains("bits=64"),
            "32-bit launcher did not launch a 64-bit JVM: {text}"
        );
    }
    let bytes = fs::read(&path).unwrap();
    let mut reader = Reader::open_auto(Cursor::new(&bytes), Limits::default()).unwrap();
    assert!(reader.range().start > 0);
    assert!(reader.verify_checksums().unwrap().complete_secure_coverage);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(fs::metadata(&path).unwrap().permissions().mode() & 0o111, 0);
    }
    let plan = native_launcher::prepare(
        File::open(&path).unwrap(),
        &path,
        Vec::new(),
        LaunchOverrides::default(),
    )
    .unwrap();
    let launch_directory = plan.directory().to_owned();
    let output = plan
        .command()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    report(&output);
    drop(plan);
    assert!(!launch_directory.exists());
}

#[test]
fn direct_mode_java8_and_optional_jar_tail_remain_usable() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture(temp.path());
    for (index, mode) in [LaunchMode::Bootstrap, LaunchMode::Direct]
        .into_iter()
        .enumerate()
    {
        let path = temp.path().join(format!("mode-{index}.exe"));
        let mut options = options(&source, &path);
        options.native_launch_mode = mode;
        options.with_launcher = true;
        pack(&options).unwrap();
        report(&command(&path).args(["--version", ""]).output().unwrap());
        report(
            &command(&path)
                .env("JANEX_LAUNCH_MODE", "direct")
                .output()
                .unwrap(),
        );
        report(
            &Command::new("java")
                .arg("-jar")
                .arg(&path)
                .output()
                .unwrap(),
        );
        if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
            let java = PathBuf::from(home).join("bin").join(if cfg!(windows) {
                "java.exe"
            } else {
                "java"
            });
            report(&command(&path).env("JANEX_JAVA", java).output().unwrap());
        }
    }
}

#[test]
fn embedded_public_pins_authenticate_signed_wrappers_and_tampering_fails() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture(temp.path());
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../janex-signature/tests/fixtures");
    let cms = authentication::load_cms_signer(
        &fixtures.join("cms/rsa256.cert.pem"),
        &fixtures.join("cms/rsa256.key.pem"),
        None,
        MATERIAL_LIMITS,
        || panic!("unencrypted fixture"),
    )
    .unwrap();
    let pgp = authentication::load_openpgp_signer(
        &fixtures.join("openpgp/ed25519.secret.pgp"),
        None,
        None,
        MATERIAL_LIMITS,
        || panic!("unencrypted fixture"),
    )
    .unwrap();
    for (index, signer) in [
        PackSigner::Cms(Arc::new(cms)),
        PackSigner::OpenPgp(Arc::new(pgp)),
    ]
    .into_iter()
    .enumerate()
    {
        let path = temp.path().join(format!("signed-{index}.exe"));
        let mut options = options(&source, &path);
        options.signer = Some(signer);
        pack(&options).unwrap();
        report(&command(&path).output().unwrap());
        let mut bytes = fs::read(&path).unwrap();
        let start = Reader::open_auto(Cursor::new(&bytes), Limits::default())
            .unwrap()
            .range()
            .start as usize;
        bytes[start + 16] ^= 1;
        let tampered = temp.path().join(format!("tampered-{index}.exe"));
        fs::write(&tampered, &bytes).unwrap();
        fs::set_permissions(&tampered, fs::metadata(&path).unwrap().permissions()).unwrap();
        let output = command(&tampered).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn malformed_headers_and_configuration_fail_before_application_execution() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture(temp.path());
    let path = temp.path().join("bad.exe");
    let mut options = options(&source, &path);
    let invalid = temp.path().join("invalid");
    fs::write(&invalid, b"not an executable").unwrap();
    options.native_launcher = Some(invalid);
    assert!(pack(&options).is_err());
    assert!(!path.exists());
    options.native_launcher = Some(env!("CARGO_BIN_EXE_janex-launcher").into());
    pack(&options).unwrap();
    let original = fs::read(&path).unwrap();
    let start = Reader::open_auto(Cursor::new(&original), Limits::default())
        .unwrap()
        .range()
        .start as usize;
    for offset in [start - 1, start - 12, start - 13] {
        let mut bytes = original.clone();
        bytes[offset] ^= 0xff;
        let malformed = temp.path().join(format!("malformed-{offset}.exe"));
        fs::write(&malformed, bytes).unwrap();
        fs::set_permissions(&malformed, fs::metadata(&path).unwrap().permissions()).unwrap();
        let output = command(&malformed).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
    }
    let output = command(Path::new(env!("CARGO_BIN_EXE_janex-launcher")))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

#[cfg(unix)]
#[test]
fn termination_of_launcher_reaches_java_and_releases_the_child() {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let temp = tempfile::tempdir().unwrap();
    let source = fixture(temp.path());
    let path = temp.path().join("waiting");
    pack(&options(&source, &path)).unwrap();
    let mut child = command(&path)
        .env("JANEX_TEST_WAIT", "1")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    assert!(lines.any(|line| line.unwrap() == "ready"));
    kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(143));
}

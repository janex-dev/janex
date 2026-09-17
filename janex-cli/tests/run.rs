// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! End-to-end command-line execution and native child status propagation.

use std::{fs, process::Command};

/// Adds an open-only preset argument to a packed fixture while regenerating its checksums.
fn add_open_argument(path: &std::path::Path) {
    use janex_format::{
        application::Application,
        binary::Limits,
        cbor::Value,
        container::{APPLICATION, Reader, Writer},
    };

    /// Replaces a fixture metadata field without dropping other fields.
    fn set(value: &Value, key: u64, replacement: Value) -> Value {
        let mut fields = value.as_map().unwrap();
        fields.retain(|(field, _)| field.as_u64().unwrap() != key);
        fields.push((Value::uint(key), replacement));
        Value::map(fields).unwrap()
    }

    let mut reader = Reader::open_auto(
        std::io::Cursor::new(fs::read(path).unwrap()),
        Limits::default(),
    )
    .unwrap();
    let sections: Vec<_> = reader.sections().cloned().collect();
    let mut writer = Writer::new(Vec::new()).unwrap();
    for section in sections {
        let type_info = section.type_info().unwrap();
        let mut bytes = reader.read_section(section.id()).unwrap();
        if section.kind() == APPLICATION {
            let app =
                Application::decode(&bytes, type_info.clone().unwrap(), Limits::default()).unwrap();
            let descriptor = app.value().required(0).unwrap();
            let overlay = Value::map([
                (
                    Value::uint(0),
                    Value::map([(Value::uint(4), Value::text("open"))]).unwrap(),
                ),
                (Value::uint(7), Value::array([Value::text("desktop")])),
            ])
            .unwrap();
            let launch = set(&descriptor.required(0).unwrap(), 6, Value::array([overlay]));
            bytes = Application::from_values(
                app.type_info().clone(),
                set(app.value(), 0, set(&descriptor, 0, launch)),
                Limits::default(),
            )
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
fn cli_runs_a_packed_application_and_propagates_its_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("Main.java"),
        r#"
public class Main {
    public static void main(String[] args) throws Exception {
        if (!java.nio.file.Files.exists(java.nio.file.Paths.get("working-directory-marker"))) {
            throw new AssertionError("working directory was not inherited");
        }
        for (String arg : args) System.out.println("[" + arg + "]");
        System.exit(42);
    }
}
"#,
    )
    .unwrap();
    let compile = Command::new("javac")
        .current_dir(temp.path())
        .args(["--release", "8", "-d", "classes", "Main.java"])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let target = temp.path().join("app.janex");
    let packed = Command::new(env!("CARGO_BIN_EXE_janex"))
        .arg("pack")
        .arg(temp.path().join("classes"))
        .arg("--output")
        .arg(&target)
        .args(["--main-class", "Main", "--argument", "preset"])
        .output()
        .unwrap();
    assert!(
        packed.status.success(),
        "{}",
        String::from_utf8_lossy(&packed.stderr)
    );
    fs::write(temp.path().join("working-directory-marker"), b"").unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_janex"))
        .arg("run")
        .arg(&target)
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("--allow-unsigned"));
    let launched = Command::new(env!("CARGO_BIN_EXE_janex"))
        .current_dir(temp.path())
        .args(["run", "--allow-unsigned", "--java", "java"])
        .arg(&target)
        .args([
            "",
            "two words",
            "--help",
            "--java",
            "fake",
            "@missing",
            "--",
        ])
        .output()
        .unwrap();
    assert_eq!(
        launched.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&launched.stderr)
    );
    assert_eq!(
        String::from_utf8(launched.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "[preset]\n[]\n[two words]\n[--help]\n[--java]\n[fake]\n[@missing]\n[--]\n"
    );
    let direct = Command::new(env!("CARGO_BIN_EXE_janex"))
        .current_dir(temp.path())
        .args(["run", "--allow-unsigned", "--launch-mode", "direct"])
        .arg(&target)
        .args(["", "two words"])
        .output()
        .unwrap();
    assert_eq!(
        direct.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&direct.stderr)
    );
    assert_eq!(
        String::from_utf8(direct.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "[preset]\n[]\n[two words]\n"
    );
    let help = Command::new(env!("CARGO_BIN_EXE_janex"))
        .args(["run", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8_lossy(&help.stdout).contains("janex run [OPTIONS] <TARGET> [ARGS...]")
    );
    let fallback = Command::new(env!("CARGO_BIN_EXE_janex"))
        .current_dir(temp.path())
        .env("JAVA_HOME", temp.path().join("missing-java-home"))
        .args(["run", "--allow-unsigned"])
        .arg(&target)
        .output()
        .unwrap();
    assert_eq!(
        fallback.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&fallback.stderr)
    );
    let explicit = Command::new(env!("CARGO_BIN_EXE_janex"))
        .args(["run", "--allow-unsigned", "--java-home"])
        .arg(temp.path().join("missing-java-home"))
        .arg(&target)
        .output()
        .unwrap();
    assert_eq!(explicit.status.code(), Some(1));

    // Desktop opening uses the same argument, authentication, and child-status machinery.
    add_open_argument(&target);
    let opened = Command::new(env!("CARGO_BIN_EXE_janex"))
        .current_dir(temp.path())
        .args(["open", "--allow-unsigned", "--java", "java", "--"])
        .arg(&target)
        .args(["", "two words", "--java"])
        .output()
        .unwrap();
    assert_eq!(
        opened.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    assert_eq!(
        String::from_utf8(opened.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "[preset]\n[desktop]\n[]\n[two words]\n[--java]\n"
    );
}

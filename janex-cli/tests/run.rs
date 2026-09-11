// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! End-to-end command-line execution and native child status propagation.

use std::{fs, process::Command};

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
}

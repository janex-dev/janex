// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Command-line external declarations, acquisition, offline reuse, and argument boundaries.

#[path = "../../janex-host/tests/support/http.rs"]
mod http;

use janex_format::{
    application::read_applications,
    binary::Limits,
    checksum::{Algorithm, Checksum},
    container::Reader,
};
use std::{fs, io::Cursor, process::Command};

#[test]
fn cli_records_remote_dependencies_and_reuses_them_offline() {
    let temp = tempfile::tempdir().unwrap();
    let server = http::Server::new();
    fs::write(
        temp.path().join("Main.java"),
        r#"
public class Main {
    public static void main(String[] args) throws Exception {
        try (java.io.InputStream input = Main.class.getResourceAsStream("/remote.txt")) {
            if (input == null || input.read() != 'r') {
                throw new AssertionError("Missing remote resource");
            }
        }
        System.out.println(args[0]);
    }
}
"#,
    )
    .unwrap();
    fs::create_dir(temp.path().join("dependency")).unwrap();
    fs::write(temp.path().join("dependency/remote.txt"), b"remote").unwrap();
    for (program, args) in [
        (
            "javac",
            vec!["--release", "8", "-d", "classes", "Main.java"],
        ),
        (
            "jar",
            vec!["--create", "--file", "remote.jar", "-C", "dependency", "."],
        ),
    ] {
        let result = Command::new(program)
            .current_dir(temp.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let bytes = fs::read(temp.path().join("remote.jar")).unwrap();
    let checksum = Checksum::compute(Algorithm::Sha256, bytes.as_slice()).unwrap();
    let hex: String = checksum
        .digest()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    server.file("/remote.jar", &bytes);
    let target = temp.path().join("app.janex");
    let packed = Command::new(env!("CARGO_BIN_EXE_janex"))
        .arg("pack")
        .arg(temp.path().join("classes"))
        .arg("--output")
        .arg(&target)
        .args([
            "--main-class",
            "Main",
            "--external-class-path",
            &format!("{}/remote.jar", server.url),
            &format!("sha256:{hex}"),
        ])
        .output()
        .unwrap();
    assert!(
        packed.status.success(),
        "{}",
        String::from_utf8_lossy(&packed.stderr)
    );
    assert!(server.requests.lock().unwrap().is_empty());
    let mut reader =
        Reader::open_auto(Cursor::new(fs::read(&target).unwrap()), Limits::default()).unwrap();
    assert_eq!(read_applications(&mut reader).unwrap().len(), 1);
    let invoke = |offline: bool, mode: &str| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_janex"));
        command
            .args([
                "run",
                "--allow-unsigned",
                "--java",
                "java",
                "--launch-mode",
                mode,
                "--dependency-cache",
            ])
            .arg(temp.path().join("cache"));
        if offline {
            command.arg("--offline");
        }
        command.arg(&target).arg("--offline").output().unwrap()
    };
    assert!(!invoke(true, "bootstrap").status.success());
    assert!(server.requests.lock().unwrap().is_empty());
    let online = invoke(false, "bootstrap");
    assert!(
        online.status.success(),
        "{}",
        String::from_utf8_lossy(&online.stderr)
    );
    assert_eq!(
        String::from_utf8(online.stdout).unwrap().trim(),
        "--offline"
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    drop(server);
    for mode in ["bootstrap", "direct"] {
        let cached = invoke(true, mode);
        assert!(
            cached.status.success(),
            "{}",
            String::from_utf8_lossy(&cached.stderr)
        );
    }
    let invalid = Command::new(env!("CARGO_BIN_EXE_janex"))
        .args(["run", "--offline", "--refresh-dependencies"])
        .arg(&target)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Process-level CLI packaging and argument-boundary checks.

use janex_format::{
    application::read_applications, binary::Limits, condition::Context, container::Reader,
};
use std::{fs, process::Command};

#[test]
fn cli_packs_local_input_and_preserves_complete_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source with spaces");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("resource.txt"), b"resource").unwrap();
    let output = temp.path().join("output with spaces.janex");
    let result = Command::new(env!("CARGO_BIN_EXE_janex"))
        .arg("pack")
        .arg(&source)
        .arg("--output")
        .arg(&output)
        .args([
            "--with-launcher",
            "--main-class",
            "example.Main",
            "--application",
            "chosen",
            "--jvm-option=-ea",
            "--jvm-option",
            "-Dkey=one value",
            "--argument",
            "",
            "--argument=--flag",
            "--argument",
            "one value",
            "--argument",
            "\u{4e2d}",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .starts_with("Packed ")
    );
    let mut reader =
        Reader::open_auto(fs::File::open(&output).unwrap(), Limits::default()).unwrap();
    assert!(reader.verify_checksums().unwrap().complete_secure_coverage);
    assert!(reader.range().end < fs::metadata(&output).unwrap().len());
    let apps = read_applications(&mut reader).unwrap();
    assert_eq!(apps[0].id(), "chosen");
    let launch = apps[0]
        .evaluate_java(&Context {
            os: "windows".into(),
            arch: "x86-64".into(),
            invocation: Some("run".into()),
            runtime: None,
        })
        .unwrap()
        .unwrap();
    assert_eq!(launch.jvm_options, ["-ea", "-Dkey=one value"]);
    assert_eq!(launch.arguments, ["", "--flag", "one value", "\u{4e2d}"]);
    let original = fs::read(&output).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_janex"))
        .arg("pack")
        .arg(&source)
        .arg("--output")
        .arg(&output)
        .args(["--main-class", "Main"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("already exists"));
    assert_eq!(fs::read(output).unwrap(), original);
}

#[test]
fn cli_documents_implemented_options_and_rejects_incomplete_commands() {
    let help = Command::new(env!("CARGO_BIN_EXE_janex"))
        .args(["pack", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    for flag in [
        "--output",
        "--class-path",
        "--module-path",
        "--main-class",
        "--main-module",
        "--application",
        "--jvm-option",
        "--argument",
        "--java-version",
        "--with-launcher",
    ] {
        assert!(text.contains(flag), "{flag}");
    }
    assert_eq!(
        Command::new(env!("CARGO_BIN_EXE_janex"))
            .args(["pack", "source"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! SDK registration, selection, environment, and process integration without network access.

use janex_java::runtime::{JavaOptions, runtimes};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

/// Executes Janex with an isolated home and no inherited Java-home selection.
fn invoke(home: &Path, project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_janex"))
        .env("JANEX_HOME", home)
        .env_remove("JAVA_HOME")
        .current_dir(project)
        .args(args)
        .output()
        .unwrap()
}

/// Checks process success while retaining both output streams in failure diagnostics.
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn registered_sdk_supports_selection_exec_and_safe_unregister() {
    let runtime = runtimes(&JavaOptions::default()).unwrap().remove(0);
    let mut java_home = runtime.home;
    if !java_home.join("release").is_file()
        && let Some(parent) = java_home.parent()
    {
        java_home = parent.to_owned();
    }
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project's workspace");
    fs::create_dir(&project).unwrap();
    let target = format!("java:bellsoft@{}", runtime.feature);
    let output = success(invoke(
        &home,
        &project,
        &[
            "install",
            &target,
            "--path",
            java_home.to_str().unwrap(),
            "--json",
        ],
    ));
    let installations: serde_json::Value = serde_json::from_str(&output).unwrap();
    let id = installations[0]["id"].as_str().unwrap();
    success(invoke(&home, &project, &["unpin", &target]));
    success(invoke(&home, &project, &["pin", &target]));
    success(invoke(&home, &project, &["update", &target, "--json"]));
    success(invoke(&home, &project, &["default", id]));
    fs::write(project.join("Main.java"), "public class Main { public static void main(String[] args) { System.out.println(\"managed-sdk\"); } }").unwrap();
    let compiler = java_home
        .join("bin")
        .join(if cfg!(windows) { "javac.exe" } else { "javac" });
    success(
        Command::new(compiler)
            .current_dir(&project)
            .args(["--release", "8", "-d", "classes", "Main.java"])
            .output()
            .unwrap(),
    );
    success(invoke(
        &home,
        &project,
        &[
            "pack",
            "classes",
            "--output",
            "app.janex",
            "--main-class",
            "Main",
        ],
    ));
    for mode in ["direct", "bootstrap"] {
        let output = Command::new(env!("CARGO_BIN_EXE_janex"))
            .env("JANEX_HOME", &home)
            .env_remove("JAVA_HOME")
            .env("PATH", "")
            .current_dir(&project)
            .args([
                "run",
                "--allow-unsigned",
                "--launch-mode",
                mode,
                "app.janex",
            ])
            .output()
            .unwrap();
        assert_eq!(success(output).trim(), "managed-sdk");
    }
    let current = success(invoke(&home, &project, &["current", "--json"]));
    let current: serde_json::Value = serde_json::from_str(&current).unwrap();
    assert_eq!(
        Path::new(current["java_home"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        java_home.canonicalize().unwrap()
    );
    success(invoke(
        &home,
        &project,
        &["exec", "--java", id, "--", "java", "-version"],
    ));
    let env = success(invoke(
        &home,
        &project,
        &["env", "--java", id, "--shell", "powershell"],
    ));
    assert!(env.starts_with("$env:JAVA_HOME = '"));
    success(invoke(&home, &project, &["use", id, "--pin"]));
    assert!(
        fs::read_to_string(project.join(".janex-toolchains.toml"))
            .unwrap()
            .contains(id)
    );
    assert!(!invoke(&home, &project, &["uninstall", id]).status.success());
    success(invoke(&home, &project, &["default", "--clear"]));
    success(invoke(&home, &project, &["uninstall", id]));
    assert!(java_home.join("release").exists());
    let list: serde_json::Value =
        serde_json::from_str(&success(invoke(&home, &project, &["list", "--json"]))).unwrap();
    assert!(list["installations"].as_array().unwrap().is_empty());
}

#[test]
fn empty_list_is_read_only_and_malformed_requests_do_not_create_installations() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    success(invoke(&home, temp.path(), &["list", "--json"]));
    assert!(!home.exists());
    for target in ["bellsoft@../21", "java:../vendor@21", "org.example:app:1.0"] {
        assert!(
            !invoke(&home, temp.path(), &["install", target, "--offline"])
                .status
                .success()
        );
    }
    assert!(!home.exists());
}

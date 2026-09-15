// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Shell integration exercised through real interpreters with isolated SDK registries.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

/// Checks success while retaining both output streams for diagnostics.
fn success(output: Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Registers a portable fixture without executing it or requiring a JVM.
fn register(root: &Path, home: &Path, version: &str) {
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    let launcher = root.join("bin").join(if cfg!(windows) {
        "gradle.bat"
    } else {
        "gradle"
    });
    fs::write(
        &launcher,
        if cfg!(windows) {
            "@echo off\r\necho fixture\r\n"
        } else {
            "#!/bin/sh\nprintf 'fixture\\n'\n"
        },
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        root.join("lib").join(format!("gradle-core-{version}.jar")),
        b"fixture",
    )
    .unwrap();
    success(
        Command::new(env!("CARGO_BIN_EXE_janex"))
            .env("JANEX_HOME", home)
            .env_remove("JANEX_SHELL_STATE")
            .args(["install", &format!("gradle@{version}"), "--path"])
            .arg(root)
            .output()
            .unwrap(),
    );
}

/// Creates isolated projects and passes fixture paths as data, never interpolating shell code.
fn exercise(shell: &str, script: &str, extension: &str) {
    eprintln!("Testing shell: {shell}");
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("Janex home's $value");
    let first = temp.path().join("SDK's first $value");
    let second = temp.path().join("SDK's second $value");
    register(&first, &home, "8.14.2");
    register(&second, &home, "8.14.3");
    fs::create_dir_all(home.join("bin")).unwrap();
    fs::create_dir_all(home.join("shell")).unwrap();
    let executable = home
        .join("bin")
        .join(if cfg!(windows) { "janex.exe" } else { "janex" });
    fs::copy(env!("CARGO_BIN_EXE_janex"), &executable).unwrap();
    for (name, content) in [
        ("init.sh", include_str!("../../shell/init.sh")),
        ("init.ps1", include_str!("../../shell/init.ps1")),
        ("init.fish", include_str!("../../shell/init.fish")),
    ] {
        fs::write(home.join("shell").join(name), content).unwrap();
    }
    let project = temp.path().join("project");
    let other = temp.path().join("other");
    fs::create_dir(&project).unwrap();
    fs::create_dir(&other).unwrap();
    fs::write(
        project.join(".janex-toolchains.toml"),
        "gradle = 'gradle@8.14.2'\n",
    )
    .unwrap();
    fs::write(
        other.join(".janex-toolchains.toml"),
        "gradle = 'gradle@8.14.3'\n",
    )
    .unwrap();
    let script_path = temp.path().join(format!("test.{extension}"));
    fs::write(&script_path, script).unwrap();
    let mut command = Command::new(shell);
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    command.env(
        "PATH",
        std::env::join_paths(
            std::iter::once(first.join("bin")).chain(std::env::split_paths(&original_path)),
        )
        .unwrap(),
    );
    if extension == "ps1" {
        command.args(["-NoProfile", "-NonInteractive", "-File"]);
    } else if shell == "zsh" {
        command.arg("-f");
    } else if shell == "fish" {
        command.arg("--no-config");
    } else if shell == "bash" {
        command.args(["--noprofile", "--norc"]);
    }
    command
        .arg(&script_path)
        .current_dir(&project)
        .env("JANEX_TEST_EXE", &executable)
        .env("JANEX_TEST_FIRST", &first)
        .env("JANEX_TEST_SECOND", &second)
        .env("JANEX_TEST_OTHER", &other)
        .env("JANEX_HOME", &home)
        .env_remove("JANEX_SHELL_STATE")
        .env("JAVA_HOME", "original '$value' ; literal")
        .env_remove("GRADLE_HOME")
        .env_remove("MAVEN_HOME");
    success(command.output().unwrap());
}

/// Keeps initialization available even when project configuration or inherited SDK state is invalid.
#[test]
fn initialization_does_not_resolve_sdks_or_create_a_home() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::write(temp.path().join(".janex-toolchains.toml"), "invalid [").unwrap();
    for shell in ["sh", "fish", "powershell"] {
        let output = Command::new(env!("CARGO_BIN_EXE_janex"))
            .env("JANEX_HOME", &home)
            .env("JANEX_SHELL_STATE", "invalid state")
            .current_dir(temp.path())
            .args(["init", shell])
            .output()
            .unwrap();
        assert!(!output.stdout.is_empty());
        success(output);
        assert!(!home.exists());
    }
}

#[test]
fn native_use_requires_integration_and_does_not_write_project_files() {
    let temp = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_janex"))
        .env("JANEX_HOME", temp.path().join("home"))
        .env_remove("JANEX_SHELL_STATE")
        .current_dir(temp.path())
        .args(["use", "gradle@8"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!temp.path().join(".janex-toolchains.toml").exists());
}

#[cfg(windows)]
#[test]
fn powershell_switches_and_restores_transactionally() {
    exercise("pwsh.exe", include_str!("shell/session.ps1"), "ps1");
}

#[cfg(unix)]
#[test]
fn unix_shells_switch_and_restore_transactionally() {
    for shell in ["sh", "bash", "zsh", "fish"] {
        if Command::new(shell)
            .arg("-c")
            .arg("exit 0")
            .status()
            .is_err()
        {
            eprintln!("Skipping unavailable shell: {shell}");
            continue;
        }
        exercise(
            shell,
            if shell == "fish" {
                include_str!("shell/session.fish")
            } else {
                include_str!("shell/session.sh")
            },
            if shell == "fish" { "fish" } else { "sh" },
        );
    }
}

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
        .env_remove("JANEX_SHELL_STATE")
        .env_remove("GRADLE_HOME")
        .env_remove("MAVEN_HOME")
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
    let target = format!("bellsoft/liberica-jdk@{}", runtime.feature);
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
            .env_remove("JANEX_SHELL_STATE")
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
    success(invoke(&home, &project, &["use", id, "--project", "--pin"]));
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
    for target in [
        "bellsoft/liberica-jdk@../21",
        "java:../vendor@21",
        "org.example:app:1.0",
    ] {
        assert!(
            !invoke(&home, temp.path(), &["install", target, "--offline"])
                .status
                .success()
        );
    }
    assert!(!home.exists());
}

/// Creates an external tool with a versioned core library and a platform launcher.
fn tool_fixture(root: &Path, family: &str, version: &str) {
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(
        root.join("lib")
            .join(format!("{family}-core-{version}.jar")),
        b"fixture",
    )
    .unwrap();
    let name = match (family, cfg!(windows)) {
        ("gradle", true) => "gradle.bat",
        ("gradle", false) => "gradle",
        (_, true) => "mvn.cmd",
        (_, false) => "mvn",
    };
    let script = if cfg!(windows) {
        "@echo off\r\necho %GRADLE_HOME%\r\necho %MAVEN_HOME%\r\necho %JAVA_HOME%\r\necho %~1\r\n"
    } else {
        "#!/bin/sh\nprintf '%s\\n' \"$GRADLE_HOME\" \"$MAVEN_HOME\" \"$JAVA_HOME\" \"$@\"\n"
    };
    let path = root.join("bin").join(name);
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn portable_tools_share_commands_and_preserve_independent_selections() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let gradle = temp.path().join("Gradle's home");
    let maven = temp.path().join("Maven home");
    tool_fixture(&gradle, "gradle", "8.14.3");
    tool_fixture(&maven, "maven", "3.9.9");
    for (target, path) in [("gradle@8", &gradle), ("maven@3.9", &maven)] {
        success(invoke(
            &home,
            &project,
            &["install", target, "--path", path.to_str().unwrap()],
        ));
        success(invoke(&home, &project, &["default", target]));
        success(invoke(&home, &project, &["use", target, "--project"]));
        success(invoke(&home, &project, &["update", target]));
    }
    let text = fs::read_to_string(project.join(".janex-toolchains.toml")).unwrap();
    assert!(text.contains("gradle@8") && text.contains("maven@3.9"));
    for command in ["gradle", "mvn"] {
        let output = success(invoke(
            &home,
            &project,
            &["exec", "--", command, "two words"],
        ));
        assert!(
            output.contains("Gradle's home")
                && output.contains("Maven home")
                && output.contains("two words")
        );
        assert!(
            !output.contains(r"\\?\"),
            "script environment must use ordinary Windows paths"
        );
    }
    let env = success(invoke(&home, &project, &["env", "--shell", "powershell"]));
    assert!(
        env.contains("$env:GRADLE_HOME")
            && env.contains("$env:MAVEN_HOME")
            && env.contains("$env:JAVA_HOME")
    );
    success(invoke(
        &home,
        &project,
        &["default", "--clear", "--family", "gradle"],
    ));
    let status: serde_json::Value =
        serde_json::from_str(&success(invoke(&home, &project, &["list", "--json"]))).unwrap();
    assert!(status["defaults"].get("gradle").is_none());
    assert!(status["defaults"].get("maven").is_some());
    assert!(
        !invoke(
            &home,
            &project,
            &["install", "gradle@8", "--arch", "x86", "--offline"]
        )
        .status
        .success()
    );
    assert!(
        !invoke(
            &home,
            &project,
            &["exec", "--java", "gradle@8", "--", "java", "-version"]
        )
        .status
        .success()
    );
    success(invoke(&home, &project, &["uninstall", "gradle@8.14.3"]));
    assert!(gradle.is_dir());
    assert!(
        !invoke(&home, &project, &["uninstall", "maven@3.9.9"])
            .status
            .success()
    );
    success(invoke(
        &home,
        &project,
        &["default", "--clear", "--family", "maven"],
    ));
    success(invoke(&home, &project, &["uninstall", "maven@3.9.9"]));
    assert!(maven.is_dir());
}

/// Creates a foreign-architecture Java home whose executables must never be run during registration.
fn java_fixture(root: &Path, arch: &str) {
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(
        root.join("release"),
        format!(
            "JAVA_VERSION=\"21.0.8\"\nJAVA_RUNTIME_VERSION=\"21.0.8+12\"\nOS_ARCH=\"{arch}\"\n"
        ),
    )
    .unwrap();
    for name in if cfg!(windows) {
        ["java.exe", "javac.exe"]
    } else {
        ["java", "javac"]
    } {
        fs::write(root.join("bin").join(name), b"not executable").unwrap();
    }
}

#[test]
fn cli_selects_platform_defaults_variants_and_portable_project_requests() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let product = "bellsoft/liberica-jdk@21";
    for arch in ["x86-64", "aarch64"] {
        let sdk = temp.path().join(arch);
        java_fixture(&sdk, arch);
        success(invoke(
            &home,
            &project,
            &[
                "install",
                product,
                "--arch",
                arch,
                "--variant",
                "full",
                "--path",
                sdk.to_str().unwrap(),
            ],
        ));
        success(invoke(
            &home,
            &project,
            &["default", product, "--arch", arch, "--variant", "full"],
        ));
    }
    let status: serde_json::Value =
        serde_json::from_str(&success(invoke(&home, &project, &["list", "--json"]))).unwrap();
    assert_eq!(status["installations"].as_array().unwrap().len(), 2);
    assert_eq!(status["defaults"].as_object().unwrap().len(), 2);
    for arch in ["x86-64", "aarch64"] {
        let environment = success(invoke(
            &home,
            &project,
            &["env", "--arch", arch, "--shell", "powershell"],
        ));
        assert!(environment.contains(arch));
        let path = success(invoke(
            &home,
            &project,
            &["home", product, "--arch", arch, "--variant", "full"],
        ));
        assert!(path.trim().ends_with(arch));
    }
    assert!(
        !invoke(
            &home,
            &project,
            &["home", product, "--arch", "x86-64", "--variant", "standard"]
        )
        .status
        .success()
    );
    assert!(
        !invoke(
            &home,
            &project,
            &["env", "--arch", "x86", "--shell", "powershell"]
        )
        .status
        .success()
    );
    success(invoke(
        &home,
        &project,
        &[
            "use",
            product,
            "--project",
            "--arch",
            "aarch64",
            "--variant",
            "full",
        ],
    ));
    let text = fs::read_to_string(project.join(".janex-toolchains.toml")).unwrap();
    assert!(text.contains("arch=aarch64") && text.contains("variant=full"));
    assert!(!text.contains("os="));
    let output = success(invoke(
        &home,
        &project,
        &[
            "use",
            product,
            "--arch",
            "x86-64",
            "--variant",
            "full",
            "--shell",
            "powershell",
        ],
    ));
    assert!(output.contains("x86-64"));
    assert!(
        !invoke(
            &home,
            &project,
            &[
                "uninstall",
                "bellsoft/liberica-jdk@21.0.8+12",
                "--arch",
                "aarch64",
                "--variant",
                "full"
            ]
        )
        .status
        .success()
    );
    success(invoke(
        &home,
        &project,
        &["default", "--clear", "--arch", "aarch64"],
    ));
    success(invoke(
        &home,
        &project,
        &[
            "uninstall",
            "bellsoft/liberica-jdk@21.0.8+12",
            "--arch",
            "aarch64",
            "--variant",
            "full",
        ],
    ));
    assert!(temp.path().join("aarch64/release").exists());
    let products: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        &project,
        &["available", "java", "--json"],
    )))
    .unwrap();
    assert!(
        products
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "bellsoft/liberica-nik")
    );
}

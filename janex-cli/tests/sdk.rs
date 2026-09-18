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
    let target = format!("sdk:bellsoft/liberica-jdk@{}", runtime.feature);
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
        Path::new(current["sdks"]["java"]["home"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        java_home.canonicalize().unwrap()
    );
    success(invoke(
        &home,
        &project,
        &["exec", "--with", id, "--", "java", "-version"],
    ));
    let env = success(invoke(
        &home,
        &project,
        &["shell", "env", "--with", id, "--shell", "powershell"],
    ));
    assert!(env.starts_with("$env:JAVA_HOME = '"));
    success(invoke(&home, &project, &["use", id, "--project", "--pin"]));
    assert!(
        fs::read_to_string(project.join(".janex-toolchains.toml"))
            .unwrap()
            .contains(id)
    );
    assert!(!invoke(&home, &project, &["uninstall", id]).status.success());
    success(invoke(
        &home,
        &project,
        &["default", "--clear", "--family", "java"],
    ));
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
        "sdk:bellsoft/liberica-jdk@../21",
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

#[test]
fn sdk_prefix_is_required_across_commands_and_project_files() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let output = invoke(
        &home,
        temp.path(),
        &["install", "sdk:gradle@9", "gradle@9", "--offline"],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("sdk: prefix")
    );
    assert!(!home.exists());

    for args in [
        vec!["available", "gradle@9", "--offline"],
        vec!["home", "gradle@9"],
        vec!["default", "gradle@9"],
        vec!["update", "gradle@9"],
        vec!["pin", "gradle@9"],
        vec!["unpin", "gradle@9"],
        vec!["uninstall", "gradle@9.1.0"],
        vec!["use", "gradle@9", "--project"],
        vec!["use", "gradle@9", "--shell", "powershell"],
        vec![
            "shell",
            "env",
            "--with",
            "gradle@9",
            "--shell",
            "powershell",
        ],
        vec!["exec", "--with", "gradle@9", "--", "unused-command"],
    ] {
        let output = invoke(&home, temp.path(), &args);
        assert!(!output.status.success(), "{args:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("sdk: prefix"), "{args:?}: {error}");
    }

    fs::write(
        temp.path().join(".janex-toolchains.toml"),
        "gradle = 'gradle@9'\n",
    )
    .unwrap();
    let output = invoke(&home, temp.path(), &["current"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("sdk: prefix")
    );
    let products = success(invoke(&home, temp.path(), &["available", "gradle"]));
    assert!(products.starts_with("sdk:gradle/gradle  "));
}

#[test]
fn install_rejects_shared_qualifiers_and_validates_all_targets_before_starting() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    for (option, value) in [
        ("--variant", "all"),
        ("--arch", "aarch64"),
        ("--os", "linux"),
        ("--libc", "musl"),
    ] {
        for targets in [vec!["sdk:gradle@9"], vec!["sdk:gradle@9", "sdk:maven@3.9"]] {
            let mut args = vec!["install"];
            args.extend(targets);
            args.extend([option, value, "--offline"]);
            let output = invoke(&home, temp.path(), &args);
            assert_eq!(output.status.code(), Some(2));
            assert!(String::from_utf8(output.stderr).unwrap().contains(option));
        }
    }
    let output = invoke(
        &home,
        temp.path(),
        &[
            "install",
            "sdk:gradle@9",
            "sdk:gradle@9[variant=invalid]",
            "--offline",
        ],
    );
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("unsupported variant"), "{error}");
    assert!(!error.contains("Installing"), "{error}");
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
        "@echo off\r\necho(%GRADLE_HOME%\r\necho(%MAVEN_HOME%\r\necho(%JAVA_HOME%\r\necho(%~1\r\n"
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

/// Exercises SDK selection and reporting without any Java executable on PATH.
#[test]
fn generic_selections_report_sources_and_project_updates_are_atomic() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let first = temp.path().join("gradle8");
    let second = temp.path().join("gradle9");
    let maven = temp.path().join("maven");
    for (target, path, family, version) in [
        ("sdk:gradle@8", &first, "gradle", "8.14.3"),
        ("sdk:gradle@9", &second, "gradle", "9.1.0"),
        ("sdk:maven@3.9", &maven, "maven", "3.9.9"),
    ] {
        tool_fixture(path, family, version);
        success(invoke(
            &home,
            &project,
            &["install", target, "--path", path.to_str().unwrap()],
        ));
    }
    let run = |args: &[&str], environment: Option<&Path>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_janex"));
        command
            .env("JANEX_HOME", &home)
            .env("PATH", "")
            .env_remove("JAVA_HOME")
            .env_remove("GRADLE_HOME")
            .env_remove("MAVEN_HOME")
            .env_remove("JANEX_SHELL_STATE")
            .current_dir(&project)
            .args(args);
        if let Some(path) = environment {
            command.env("GRADLE_HOME", path);
        }
        command.output().unwrap()
    };
    success(run(&["default", "sdk:gradle@8"], None));
    let current = |environment| -> serde_json::Value {
        serde_json::from_str(&success(run(
            &["current", "--kind", "sdk", "--json"],
            environment,
        )))
        .unwrap()
    };
    let state = current(None);
    assert_eq!(state["sdks"]["gradle"]["source"]["kind"], "default");
    assert!(state["sdks"].get("java").is_none());
    assert_eq!(run(&["default", "--clear"], None).status.code(), Some(2));
    assert_eq!(current(None)["sdks"]["gradle"]["source"]["kind"], "default");

    success(run(
        &["use", "--project", "sdk:gradle@9", "sdk:maven@3.9"],
        None,
    ));
    let path = project.join(".janex-toolchains.toml");
    let content = fs::read(&path).unwrap();
    for targets in [
        ["sdk:gradle@8", "sdk:maven@99"],
        ["sdk:gradle@8", "sdk:gradle@9"],
    ] {
        assert!(
            !run(&["use", "--project", targets[0], targets[1]], None)
                .status
                .success()
        );
        assert_eq!(fs::read(&path).unwrap(), content);
    }
    let state = current(None);
    for family in ["gradle", "maven"] {
        assert_eq!(state["sdks"][family]["source"]["kind"], "project");
        assert!(
            state["sdks"][family]["source"]["value"]
                .as_str()
                .unwrap()
                .ends_with(".janex-toolchains.toml")
        );
    }
    let state = current(Some(&first));
    assert_eq!(state["sdks"]["gradle"]["source"]["kind"], "environment");
    assert_eq!(state["sdks"]["gradle"]["source"]["value"], "GRADLE_HOME");
    let output = success(run(
        &[
            "exec",
            "--with",
            "sdk:gradle@9",
            "--with",
            "sdk:maven@3.9",
            "--",
            "gradle",
            "two words",
        ],
        Some(&first),
    ));
    assert!(output.contains("gradle9") && output.contains("maven") && output.contains("two words"));
    assert!(
        !run(
            &[
                "exec",
                "--with",
                "sdk:gradle@8",
                "--with",
                "sdk:gradle@9",
                "--",
                "gradle"
            ],
            None
        )
        .status
        .success()
    );
    let environment = success(run(
        &[
            "shell",
            "env",
            "--with",
            "sdk:gradle@8",
            "--with",
            "sdk:maven@3.9",
            "--shell",
            "powershell",
        ],
        None,
    ));
    assert!(environment.contains("gradle8") && environment.contains("maven"));

    let summary = success(run(&["list", "--kind", "sdk"], None));
    let status: serde_json::Value =
        serde_json::from_str(&success(run(&["list", "--json"], None))).unwrap();
    let id = status["installations"][0]["id"].as_str().unwrap();
    assert!(summary.contains("default") && summary.contains("external"));
    assert!(!summary.contains(id) && !summary.contains(temp.path().to_str().unwrap()));
    assert!(success(run(&["list", "--verbose"], None)).contains(id));
    let apps: serde_json::Value =
        serde_json::from_str(&success(run(&["list", "--kind", "app", "--json"], None))).unwrap();
    assert!(apps["installations"].as_array().unwrap().is_empty());
    success(run(&["default", "--clear", "--family", "gradle"], None));
    let all = success(run(&["available"], None));
    assert!(all.contains("sdk:bellsoft/liberica-jdk") && all.contains("sdk:gradle/gradle"));
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
    for (target, path) in [("sdk:gradle@8", &gradle), ("sdk:maven@3.9", &maven)] {
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
    assert!(text.contains("sdk:gradle@8") && text.contains("sdk:maven@3.9"));
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
    let env = success(invoke(
        &home,
        &project,
        &["shell", "env", "--shell", "powershell"],
    ));
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
            &["install", "sdk:gradle@8[arch=x86]", "--offline"]
        )
        .status
        .success()
    );
    assert!(
        !invoke(
            &home,
            &project,
            &[
                "exec",
                "--with",
                "sdk:gradle@8",
                "--with",
                "sdk:gradle@8",
                "--",
                "java",
                "-version"
            ]
        )
        .status
        .success()
    );
    success(invoke(&home, &project, &["uninstall", "sdk:gradle@8.14.3"]));
    assert!(gradle.is_dir());
    assert!(
        !invoke(&home, &project, &["uninstall", "sdk:maven@3.9.9"])
            .status
            .success()
    );
    success(invoke(
        &home,
        &project,
        &["default", "--clear", "--family", "maven"],
    ));
    success(invoke(&home, &project, &["uninstall", "sdk:maven@3.9.9"]));
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
    let product = "sdk:bellsoft/liberica-jdk@21";
    for arch in ["x86-64", "aarch64"] {
        let sdk = temp.path().join(arch);
        java_fixture(&sdk, arch);
        success(invoke(
            &home,
            &project,
            &[
                "install",
                &format!("{product}[arch={arch},variant=full]"),
                "--path",
                sdk.to_str().unwrap(),
            ],
        ));
        success(invoke(
            &home,
            &project,
            &["default", &format!("{product}[arch={arch},variant=full]")],
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
            &[
                "shell",
                "env",
                "--with",
                &format!("{product}[arch={arch},variant=full]"),
                "--shell",
                "powershell",
            ],
        ));
        assert!(environment.contains(arch));
        let path = success(invoke(
            &home,
            &project,
            &["home", &format!("{product}[arch={arch},variant=full]")],
        ));
        assert!(path.trim().ends_with(arch));
    }
    assert!(
        !invoke(
            &home,
            &project,
            &["home", &format!("{product}[arch=x86-64,variant=standard]")]
        )
        .status
        .success()
    );
    assert!(
        !invoke(
            &home,
            &project,
            &[
                "shell",
                "env",
                "--with",
                &format!("{product}[arch=x86,variant=full]"),
                "--shell",
                "powershell"
            ]
        )
        .status
        .success()
    );
    success(invoke(
        &home,
        &project,
        &[
            "use",
            &format!("{product}[arch=aarch64,variant=full]"),
            "--project",
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
            &format!("{product}[arch=x86-64,variant=full]"),
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
                "sdk:bellsoft/liberica-jdk@21.0.8+12[arch=aarch64,variant=full]"
            ]
        )
        .status
        .success()
    );
    success(invoke(
        &home,
        &project,
        &[
            "default",
            "--clear",
            &format!("{product}[arch=aarch64,variant=full]"),
        ],
    ));
    success(invoke(
        &home,
        &project,
        &[
            "uninstall",
            "sdk:bellsoft/liberica-jdk@21.0.8+12[arch=aarch64,variant=full]",
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

#[test]
fn gradle_variants_flow_through_cli_projects_and_shell_selection() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    for variant in ["bin", "all"] {
        let tool = temp.path().join(format!("gradle-{variant}"));
        tool_fixture(&tool, "gradle", "9.1.0");
        if variant == "all" {
            fs::create_dir(tool.join("docs")).unwrap();
            fs::create_dir(tool.join("src")).unwrap();
        }
        success(invoke(
            &home,
            &project,
            &[
                "install",
                &format!("sdk:gradle@9[variant={variant}]"),
                "--path",
                tool.to_str().unwrap(),
            ],
        ));
    }
    assert!(
        !invoke(
            &home,
            &project,
            &[
                "install",
                "sdk:gradle@9[variant=all]",
                "--path",
                temp.path().join("gradle-bin").to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    let list: serde_json::Value =
        serde_json::from_str(&success(invoke(&home, &project, &["list", "--json"]))).unwrap();
    let entries = list["installations"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0]["id"], entries[1]["id"]);
    let installed: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        &project,
        &[
            "install",
            "sdk:gradle@9[variant=all]",
            "sdk:gradle@9",
            "--offline",
            "--json",
        ],
    )))
    .unwrap();
    assert_eq!(installed[0]["sdk"]["variant"], "all");
    assert_eq!(installed[1]["sdk"]["variant"], "bin");
    success(invoke(
        &home,
        &project,
        &[
            "update",
            "sdk:gradle@9[variant=all]",
            "sdk:gradle@9[variant=bin]",
        ],
    ));
    for variant in ["bin", "all"] {
        assert!(
            entries
                .iter()
                .any(|entry| entry["sdk"]["variant"] == variant
                    && entry["sdk"]["platform"].is_null())
        );
        let path = success(invoke(
            &home,
            &project,
            &["home", &format!("sdk:gradle@9[variant={variant}]")],
        ));
        assert!(path.trim().ends_with(&format!("gradle-{variant}")));
    }
    success(invoke(
        &home,
        &project,
        &["default", "sdk:gradle@9[variant=all]"],
    ));
    success(invoke(
        &home,
        &project,
        &["use", "sdk:gradle@9[variant=all]", "--project"],
    ));
    let text = fs::read_to_string(project.join(".janex-toolchains.toml")).unwrap();
    assert!(text.contains("sdk:gradle@9[variant=all]"));
    let environment = success(invoke(
        &home,
        &project,
        &["use", "sdk:gradle@9[variant=bin]", "--shell", "powershell"],
    ));
    assert!(environment.contains("gradle-bin"));
    let output = success(invoke(
        &home,
        &project,
        &[
            "exec",
            "--with",
            "sdk:gradle@9[variant=all]",
            "--",
            "gradle",
            "test argument",
        ],
    ));
    assert!(output.contains("gradle-all") && output.contains("test argument"));
    success(invoke(
        &home,
        &project,
        &["unpin", "sdk:gradle@9[variant=all]"],
    ));
    success(invoke(
        &home,
        &project,
        &["pin", "sdk:gradle@9[variant=all]"],
    ));
    success(invoke(&home, &project, &["update", "--all"]));
    success(invoke(
        &home,
        &project,
        &["uninstall", "sdk:gradle@9.1.0[variant=bin]"],
    ));
    assert!(
        !invoke(&home, &project, &["home", "sdk:gradle@9"])
            .status
            .success()
    );
    assert!(temp.path().join("gradle-bin").is_dir());
    assert!(
        !invoke(
            &home,
            &project,
            &["uninstall", "sdk:gradle@9.1.0[variant=all]"]
        )
        .status
        .success()
    );
    let products: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        &project,
        &["available", "gradle", "--json"],
    )))
    .unwrap();
    assert_eq!(products[0]["variants"], serde_json::json!(["bin", "all"]));
    assert_eq!(products[0]["platform_specific"], false);
}

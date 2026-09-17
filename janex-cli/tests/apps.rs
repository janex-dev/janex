// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Maven application installation and native command execution in isolated homes.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};

/// Runs Janex with isolated persistent state.
fn invoke(home: &Path, directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_janex"))
        .env("JANEX_HOME", home)
        .env_remove("JANEX_SHELL_STATE")
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap()
}

/// Checks success and retains both streams for failure diagnostics.
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n")
}

/// Local Maven fixture repository with a real Java application.
struct Repository {
    /// Repository directory.
    root: PathBuf,
    /// Compiled application directory.
    classes: PathBuf,
    /// JDK archive tool.
    jar_tool: PathBuf,
    /// File repository URL.
    url: String,
}

impl Repository {
    /// Compiles Java 8 bytecode that reports resources, arguments, and a selected exit status.
    fn new(directory: &Path) -> Self {
        let root = directory.join("repository");
        let classes = directory.join("classes");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&classes).unwrap();
        let source = directory.join("Main.java");
        fs::write(&source, r#"import java.nio.file.*;
import java.util.*;
public class Main {
    public static void main(String[] args) throws Exception {
        if (args.length > 1 && args[0].equals("wait")) {
            Files.write(Paths.get(args[1]), new byte[] {1});
            Thread.sleep(2500);
        }
        System.out.println(new Scanner(Main.class.getResourceAsStream("/version.txt"), "UTF-8").nextLine());
        for (String arg : args) System.out.println(Base64.getEncoder().encodeToString(arg.getBytes("UTF-8")));
        if (args.length > 0 && args[0].equals("exit")) System.exit(23);
    }
}"#).unwrap();
        let runtime = janex_java::runtime::runtimes(&Default::default())
            .unwrap()
            .remove(0);
        let compiler =
            runtime
                .home
                .join("bin")
                .join(if cfg!(windows) { "javac.exe" } else { "javac" });
        success(
            Command::new(compiler)
                .args(["--release", "8", "-d"])
                .arg(&classes)
                .arg(&source)
                .output()
                .unwrap(),
        );
        let jar_tool = runtime
            .home
            .join("bin")
            .join(if cfg!(windows) { "jar.exe" } else { "jar" });
        let url = format!(
            "file://{}{}",
            if cfg!(windows) { "/" } else { "" },
            root.display()
        )
        .replace('\\', "/");
        Self {
            root,
            classes,
            jar_tool,
            url,
        }
    }

    /// Writes a standalone archive in the standard Maven layout.
    fn jar(&self, artifact: &str, version: &str, classifier: Option<&str>) -> PathBuf {
        let directory = self.root.join("org/example").join(artifact).join(version);
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!(
            "{artifact}-{version}{}.jar",
            classifier.map_or(String::new(), |v| format!("-{v}"))
        ));
        fs::write(self.classes.join("version.txt"), version).unwrap();
        success(
            Command::new(&self.jar_tool)
                .args(["--create", "--file"])
                .arg(&path)
                .args(["--main-class", "Main", "-C"])
                .arg(&self.classes)
                .arg(".")
                .output()
                .unwrap(),
        );
        path
    }

    /// Publishes an explicit release pointer, independently of version sorting.
    fn release(&self, artifact: &str, version: &str) {
        let directory = self.root.join("org/example").join(artifact);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("maven-metadata.xml"), format!("<metadata><groupId>org.example</groupId><artifactId>{artifact}</artifactId><versioning><release>{version}</release></versioning></metadata>")).unwrap();
    }

    /// Builds a request using this local repository.
    fn target(&self, artifact: &str, version: &str) -> String {
        format!(
            "maven:org.example:{artifact}@{version}[repository={}]",
            self.url
        )
    }
}

/// Returns the native command path.
fn entry(home: &Path, name: &str) -> PathBuf {
    home.join("bin").join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    })
}

/// Executes a generated command with uninterpreted native arguments.
fn native(home: &Path, directory: &Path, name: &str, args: &[&str]) -> Output {
    Command::new(entry(home, name))
        .env("JANEX_HOME", home)
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn maven_apps_keep_versions_pins_command_defaults_and_original_jars() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("Janex's home");
    let repository = Repository::new(temp.path());
    let first = repository.jar("demo", "1.0", None);
    repository.jar("demo", "2.0", None);
    repository.jar("other", "1.0", None);
    repository.release("demo", "1.0");
    let latest = repository.target("demo", "latest");
    let second = repository.target("demo", "2.0");
    let other = repository.target("other", "1.0");
    let installed: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &["install", &latest, &other, "--json"],
    )))
    .unwrap();
    assert_eq!(installed.as_array().unwrap().len(), 2);
    let first_id = installed[0]["id"].as_str().unwrap();
    let directory = success(invoke(&home, temp.path(), &["home", first_id]));
    assert_eq!(
        fs::read(Path::new(directory.trim()).join("demo-1.0.jar")).unwrap(),
        fs::read(first).unwrap()
    );
    assert_eq!(
        success(native(
            &home,
            temp.path(),
            "demo",
            &["", "\u{4f60}\u{597d}\u{1f600}", "--help", "a b", "\""]
        )),
        "1.0\n\n5L2g5aW98J+YgA==\nLS1oZWxw\nYSBi\nIg==\n"
    );
    assert_eq!(
        native(&home, temp.path(), "demo", &["exit"]).status.code(),
        Some(23)
    );
    for mode in ["bootstrap", "direct"] {
        let output = success(invoke(
            &home,
            temp.path(),
            &["run", "--launch-mode", mode, first_id, "plain"],
        ));
        assert!(output.starts_with("1.0") && output.contains("cGxhaW4="));
    }
    success(invoke(&home, temp.path(), &["install", &second]));
    assert!(success(native(&home, temp.path(), "demo", &[])).starts_with("1.0"));
    success(invoke(&home, temp.path(), &["pin", &latest]));
    repository.release("demo", "2.0");
    success(invoke(&home, temp.path(), &["update", "--all"]));
    assert!(success(native(&home, temp.path(), "demo", &[])).starts_with("1.0"));
    success(invoke(&home, temp.path(), &["unpin", &latest]));
    success(invoke(&home, temp.path(), &["update", &latest]));
    assert!(success(native(&home, temp.path(), "demo", &[])).starts_with("2.0"));
    success(invoke(&home, temp.path(), &["default", first_id]));
    success(invoke(&home, temp.path(), &["update", &latest]));
    assert!(success(native(&home, temp.path(), "demo", &[])).starts_with("1.0"));
    let state: serde_json::Value =
        serde_json::from_str(&success(invoke(&home, temp.path(), &["list", "--json"]))).unwrap();
    assert_eq!(
        state["applications"]["installations"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(state["installations"].as_array().unwrap().is_empty());
    success(invoke(
        &home,
        temp.path(),
        &["default", "--clear", first_id],
    ));
    assert!(!entry(&home, "demo").exists());
    success(invoke(&home, temp.path(), &["default", &latest]));
    success(invoke(&home, temp.path(), &["uninstall", first_id]));
    assert!(entry(&home, "demo").is_file());
    fs::rename(&repository.root, temp.path().join("offline-repository")).unwrap();
    if home.join("cache").exists() {
        fs::remove_dir_all(home.join("cache")).unwrap();
    }
    success(invoke(
        &home,
        temp.path(),
        &["install", &latest, "--offline"],
    ));
    assert!(success(native(&home, temp.path(), "demo", &[])).starts_with("2.0"));
    success(invoke(&home, temp.path(), &["uninstall", &second]));
    assert!(!entry(&home, "demo").exists());
    assert!(entry(&home, "other").is_file());
}

#[test]
fn collisions_preserve_user_files_and_running_applications_hold_leases() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repository = Repository::new(temp.path());
    repository.jar("demo", "1.0", None);
    repository.jar("other", "1.0", None);
    let target = repository.target("demo", "1.0");
    fs::create_dir_all(home.join("bin")).unwrap();
    fs::write(entry(&home, "demo"), b"user file").unwrap();
    assert!(
        !invoke(&home, temp.path(), &["install", &target])
            .status
            .success()
    );
    assert_eq!(fs::read(entry(&home, "demo")).unwrap(), b"user file");
    fs::remove_file(entry(&home, "demo")).unwrap();
    success(invoke(&home, temp.path(), &["install", &target]));
    let conflict = format!(
        "maven:org.example:other@1.0[command=demo,repository={}]",
        repository.url
    );
    assert!(
        !invoke(&home, temp.path(), &["install", &conflict])
            .status
            .success()
    );
    let ready = temp.path().join("ready");
    let mut child = Command::new(entry(&home, "demo"))
        .env("JANEX_HOME", &home)
        .arg("wait")
        .arg(&ready)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "application did not become ready"
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "application exited before becoming ready"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = invoke(&home, temp.path(), &["uninstall", &target]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("in use"));
    #[cfg(windows)]
    {
        assert!(
            !invoke(&home, temp.path(), &["default", "--clear", &target])
                .status
                .success()
        );
        let state: serde_json::Value =
            serde_json::from_str(&success(invoke(&home, temp.path(), &["list", "--json"])))
                .unwrap();
        assert!(state["applications"]["commands"]["demo"].is_object());
    }
    assert!(child.wait().unwrap().success());
    success(invoke(&home, temp.path(), &["uninstall", &target]));
}

#[test]
fn janex_artifacts_and_invalid_batches_are_handled_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repository = Repository::new(temp.path());
    let jar = repository.jar("container", "1.0", Some("all"));
    let package = jar.with_extension("janex");
    success(invoke(
        &home,
        temp.path(),
        &[
            "pack",
            jar.to_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
    ));
    let target = format!(
        "maven:org.example:container@1.0[type=janex,classifier=all,command=tool,repository={}]",
        repository.url
    );
    success(invoke(&home, temp.path(), &["install", &target]));
    assert!(success(native(&home, temp.path(), "tool", &["--help"])).contains("LS1oZWxw"));
    fs::copy(&package, temp.path().join("app-demo.janex")).unwrap();
    assert!(
        success(invoke(
            &home,
            temp.path(),
            &["run", "--allow-unsigned", "app-demo.janex", "--help"],
        ))
        .contains("LS1oZWxw")
    );
    let sdk = temp.path().join("gradle");
    fs::create_dir_all(sdk.join("bin")).unwrap();
    fs::create_dir_all(sdk.join("lib")).unwrap();
    fs::write(
        sdk.join("bin").join(if cfg!(windows) {
            "gradle.bat"
        } else {
            "gradle"
        }),
        b"fixture",
    )
    .unwrap();
    fs::write(sdk.join("lib/gradle-core-9.1.0.jar"), b"fixture").unwrap();
    success(invoke(
        &home,
        temp.path(),
        &["install", "gradle@9", "--path", sdk.to_str().unwrap()],
    ));
    let mixed: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &["install", "gradle@9", &target, "--offline", "--json"],
    )))
    .unwrap();
    assert_eq!(mixed[0]["sdk"]["product"], "gradle/gradle");
    assert_eq!(mixed[1]["application"]["command"], "tool");
    let empty_home = temp.path().join("unused");
    let invalid = invoke(
        &empty_home,
        temp.path(),
        &[
            "install",
            "gradle@9",
            "maven:org.example:bad@1[arch=aarch64]",
            "--offline",
        ],
    );
    assert!(!invalid.status.success());
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("Installing"));
    assert!(!empty_home.exists());
    fs::write(
        repository
            .root
            .join("org/example/container/1.0/container-1.0.jar"),
        b"not a jar",
    )
    .unwrap();
    assert!(
        !invoke(
            &home,
            temp.path(),
            &["install", &repository.target("container", "1.0")]
        )
        .status
        .success()
    );
    assert!(!entry(&home, "container").exists());
    let state: serde_json::Value =
        serde_json::from_str(&success(invoke(&home, temp.path(), &["list", "--json"]))).unwrap();
    assert_eq!(
        state["applications"]["installations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

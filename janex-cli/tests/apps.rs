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
        self.pom(artifact, version, "");
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

    /// Publishes the unclassified POM shared by an artifact's classifiers.
    fn pom(&self, artifact: &str, version: &str, body: &str) {
        let directory = self.root.join("org/example").join(artifact).join(version);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(format!("{artifact}-{version}.pom")), format!("<project><modelVersion>4.0.0</modelVersion><groupId>org.example</groupId><artifactId>{artifact}</artifactId><version>{version}</version>{body}</project>")).unwrap();
    }

    /// Publishes an explicit release pointer, independently of version sorting.
    fn release(&self, artifact: &str, version: &str) {
        let directory = self.root.join("org/example").join(artifact);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("maven-metadata.xml"), format!("<metadata><groupId>org.example</groupId><artifactId>{artifact}</artifactId><versioning><release>{version}</release></versioning></metadata>")).unwrap();
    }

    /// Builds a request using this local repository.
    fn target(&self, artifact: &str, version: &str) -> String {
        let version = if version == "latest" {
            String::new()
        } else {
            format!("@{version}")
        };
        format!(
            "maven:org.example:{artifact}{version}[repository={}]",
            self.url
        )
    }

    /// Builds a full PURL with an independently encoded local repository qualifier.
    fn purl(&self, artifact: &str, version: Option<&str>) -> String {
        let version = version.map_or(String::new(), |version| format!("@{version}"));
        format!(
            "pkg:maven/org.example/{artifact}{version}?repository_url={}",
            self.url.replace('%', "%25").replace('/', "%2F")
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
fn thin_jars_lock_runtime_dependencies_and_launch_without_the_repository() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repository = Repository::new(temp.path());
    let source = temp.path().join("Thin.java");
    let library = temp.path().join("Library.java");
    fs::write(&source, "public class Thin { public static void main(String[] args) throws Exception { System.out.println(Library.value()); System.out.println(new java.util.Scanner(Thin.class.getResourceAsStream(\"/dependency.txt\"), \"UTF-8\").nextLine()); for (String arg : args) System.out.println(java.util.Base64.getEncoder().encodeToString(arg.getBytes(\"UTF-8\"))); } }").unwrap();
    fs::write(
        &library,
        "public class Library { public static String value() { return \"dependency-class\"; } }",
    )
    .unwrap();
    let compiler =
        repository
            .jar_tool
            .with_file_name(if cfg!(windows) { "javac.exe" } else { "javac" });
    success(
        Command::new(compiler)
            .args(["--release", "8", "-d"])
            .arg(&repository.classes)
            .arg(&source)
            .arg(&library)
            .output()
            .unwrap(),
    );
    let app = repository.root.join("org/example/thin/1/thin-1.jar");
    repository.pom("thin", "1", "<dependencies><dependency><groupId>org.example</groupId><artifactId>library</artifactId><version>1</version></dependency><dependency><groupId>org.example</groupId><artifactId>test-only</artifactId><version>1</version><scope>test</scope></dependency></dependencies>");
    repository.pom("test-only", "1", "");
    success(
        Command::new(&repository.jar_tool)
            .args(["--create", "--file"])
            .arg(&app)
            .args(["--main-class", "Thin", "-C"])
            .arg(&repository.classes)
            .arg("Thin.class")
            .output()
            .unwrap(),
    );
    let lib = repository.root.join("org/example/library/1/library-1.jar");
    repository.pom("library", "1", "");
    fs::write(
        repository.classes.join("dependency.txt"),
        "dependency-resource\n",
    )
    .unwrap();
    success(
        Command::new(&repository.jar_tool)
            .args(["--create", "--file"])
            .arg(&lib)
            .arg("-C")
            .arg(&repository.classes)
            .arg("Library.class")
            .arg("-C")
            .arg(&repository.classes)
            .arg("dependency.txt")
            .output()
            .unwrap(),
    );
    let target = repository.purl("thin", Some("1"));
    let installed: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &["install", &target, "--json"],
    )))
    .unwrap();
    let id = installed[0]["id"].as_str().unwrap();
    let dependencies = installed[0]["jar"]["dependencies"].as_array().unwrap();
    assert_eq!(dependencies.len(), 1);
    assert_eq!(installed[0]["jar"]["main_class"], "Thin");
    let dependency = &dependencies[0];
    assert!(
        dependency["purl"]
            .as_str()
            .unwrap()
            .starts_with("pkg:maven/org.example/library@1?")
    );
    assert_eq!(dependency["sha256"].as_str().unwrap().len(), 64);
    let installed_library = home
        .join("apps")
        .join(id)
        .join(dependency["file"].as_str().unwrap());
    assert_eq!(
        fs::read(&installed_library).unwrap(),
        fs::read(&lib).unwrap()
    );

    // A dependency-only update gets a new installation identity and retains the old closure.
    repository.pom("library", "2", "");
    fs::copy(
        &lib,
        repository.root.join("org/example/library/2/library-2.jar"),
    )
    .unwrap();
    repository.pom("thin", "1", "<dependencies><dependency><groupId>org.example</groupId><artifactId>library</artifactId><version>2</version></dependency></dependencies>");
    let updated: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &["update", &target, "--json"],
    )))
    .unwrap();
    let updated_id = updated[0]["id"].as_str().unwrap();
    assert_ne!(id, updated_id);
    assert_eq!(installed[0]["sha256"], updated[0]["sha256"]);
    assert!(installed_library.is_file());

    // A missing dependency must leave the previous install and command usable.
    repository.pom("thin", "1", "<dependencies><dependency><groupId>org.example</groupId><artifactId>missing</artifactId><version>1</version></dependency></dependencies>");
    assert!(
        !invoke(&home, temp.path(), &["update", &target])
            .status
            .success()
    );
    let current: serde_json::Value =
        serde_json::from_str(&success(invoke(&home, temp.path(), &["list", "--json"]))).unwrap();
    assert_eq!(
        current["applications"]["installations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fs::remove_dir_all(&repository.root).unwrap();
    let cache = home.join("cache/dependencies");
    if cache.exists() {
        fs::remove_dir_all(cache).unwrap();
    }
    for mode in ["direct", "bootstrap"] {
        let output = success(invoke(
            &home,
            temp.path(),
            &[
                "run",
                "--offline",
                "--launch-mode",
                mode,
                &target,
                "argument with spaces",
            ],
        ));
        assert_eq!(
            output,
            "dependency-class\ndependency-resource\nYXJndW1lbnQgd2l0aCBzcGFjZXM=\n"
        );
    }
    assert_eq!(
        success(native(&home, temp.path(), "thin", &["😀"])),
        "dependency-class\ndependency-resource\n8J+YgA==\n"
    );
    success(invoke(
        &home,
        temp.path(),
        &["install", &target, "--offline"],
    ));
    success(invoke(&home, temp.path(), &["uninstall", id]));
    assert!(!installed_library.exists());
    success(invoke(&home, temp.path(), &["uninstall", updated_id]));
}

#[test]
fn explicit_main_class_and_self_contained_policy_are_local_installation_options() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repository = Repository::new(temp.path());
    let artifact = repository.jar("standalone", "1", None);
    fs::remove_file(artifact.with_extension("pom")).unwrap();
    // Omit the manifest entirely, requiring the per-target entry point.
    success(
        Command::new(&repository.jar_tool)
            .args(["--create", "--no-manifest", "--file"])
            .arg(&artifact)
            .arg("-C")
            .arg(&repository.classes)
            .arg(".")
            .output()
            .unwrap(),
    );
    let purl = repository.purl("standalone", Some("1"));
    let target = format!("{purl}[main-class=Main,dependencies=none]");
    let installed: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &["install", &target, "--json"],
    )))
    .unwrap();
    assert_eq!(
        installed[0]["application"]["purl"]
            .as_str()
            .unwrap()
            .split('[')
            .count(),
        1
    );
    assert_eq!(installed[0]["application"]["jar"]["dependencies"], "none");
    for mode in ["direct", "bootstrap"] {
        assert_eq!(
            success(invoke(
                &home,
                temp.path(),
                &["run", "--launch-mode", mode, &target]
            )),
            "1\n"
        );
    }
    let missing = format!("{purl}[main-class=Main]");
    assert!(
        !invoke(&home, temp.path(), &["install", &missing])
            .status
            .success()
    );
    assert_eq!(
        success(native(&home, temp.path(), "standalone", &[])),
        "1\n"
    );
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
    let latest = repository.purl("demo", None);
    let second = repository.purl("demo", Some("2.0"));
    let other = repository.target("other", "1.0");
    let installed: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &["install", &latest, &other, "--json"],
    )))
    .unwrap();
    assert_eq!(installed.as_array().unwrap().len(), 2);
    let first_id = installed[0]["id"].as_str().unwrap();
    let canonical = installed[0]["application"]["purl"].as_str().unwrap();
    assert!(canonical.starts_with("pkg:maven/org.example/demo@1.0?repository_url="));
    janex_format::purl::parse(canonical).unwrap();
    let duplicate: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &[
            "install",
            &repository.target("demo", "latest"),
            "--offline",
            "--json",
        ],
    )))
    .unwrap();
    assert_eq!(duplicate[0]["id"], first_id);
    let recorded: serde_json::Value =
        serde_json::from_str(&success(invoke(&home, temp.path(), &["list", "--json"]))).unwrap();
    let selections = recorded["applications"]["selections"].as_array().unwrap();
    assert_eq!(selections.len(), 2);
    assert!(
        !selections[0]["request"]["purl"]
            .as_str()
            .unwrap()
            .contains('@')
    );
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
            &["run", "--launch-mode", mode, canonical, "plain"],
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
    let jar_target = format!(
        "{}&classifier=all",
        repository.purl("container", Some("1.0"))
    );
    let jar_install: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &["install", &jar_target, "--json"],
    )))
    .unwrap();
    assert!(
        jar_install[0]["source"]
            .as_str()
            .unwrap()
            .ends_with("container-1.0-all.jar")
    );
    let jar_home = success(invoke(&home, temp.path(), &["home", &jar_target]));
    assert_eq!(
        fs::read(Path::new(jar_home.trim()).join("container-1.0-all.jar")).unwrap(),
        fs::read(&jar).unwrap()
    );
    success(invoke(&home, temp.path(), &["uninstall", &jar_target]));
    fs::remove_file(&jar).unwrap();
    assert!(package.is_file());
    assert!(
        !invoke(&home, temp.path(), &["install", &jar_target])
            .status
            .success()
    );
    assert!(!entry(&home, "container").exists());
    let janex_target = format!("{jar_target}&type=janex");
    let janex_install: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &["install", &janex_target, "--json"],
    )))
    .unwrap();
    assert!(
        janex_install[0]["source"]
            .as_str()
            .unwrap()
            .ends_with("container-1.0-all.janex")
    );
    assert!(
        janex_install[0]["application"]["purl"]
            .as_str()
            .unwrap()
            .ends_with("&type=janex")
    );
    assert!(success(native(&home, temp.path(), "container", &["--help"])).contains("LS1oZWxw"));
    success(invoke(&home, temp.path(), &["uninstall", &janex_target]));
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
        &["install", "sdk:gradle@9", "--path", sdk.to_str().unwrap()],
    ));
    let mixed: serde_json::Value = serde_json::from_str(&success(invoke(
        &home,
        temp.path(),
        &[
            "install",
            "sdk:gradle@9",
            &target,
            &janex_target,
            "--offline",
            "--json",
        ],
    )))
    .unwrap();
    assert_eq!(mixed[0]["sdk"]["product"], "gradle/gradle");
    assert_eq!(mixed[1]["application"]["command"], "tool");
    assert_eq!(mixed[2]["application"]["command"], "container");
    success(invoke(&home, temp.path(), &["uninstall", &janex_target]));
    let empty_home = temp.path().join("unused");
    let invalid = invoke(
        &empty_home,
        temp.path(),
        &[
            "install",
            "sdk:gradle@9",
            "maven:org.example:bad@1[arch=aarch64]",
            "--offline",
        ],
    );
    assert!(!invalid.status.success());
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("Installing"));
    assert!(!empty_home.exists());
    let invalid_purl = invoke(
        &empty_home,
        temp.path(),
        &[
            "install",
            "sdk:gradle@9",
            "pkg:npm/typescript@5.9.2",
            "--offline",
        ],
    );
    assert!(!invalid_purl.status.success());
    assert!(String::from_utf8_lossy(&invalid_purl.stderr).contains("PURL type npm"));
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

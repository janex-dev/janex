// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Native and independent Java resolution of selected virtual module requirements.

use janex_format::{
    application::{Application, PathEntry},
    binary::Limits,
    cbor::{self, Value},
    container::{APPLICATION, Reader, Writer},
};
use janex_host::{
    pack::{PackOptions, pack},
    run::{RunOptions, prepare},
};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use zip::{ZipWriter, write::SimpleFileOptions};

/// Replaces one integer-keyed field while retaining the other encoded values.
fn replace(value: Value, key: u64, replacement: Value) -> Value {
    let mut entries = value.as_map().unwrap();
    entries.retain(|(candidate, _)| candidate.as_u64().unwrap() != key);
    entries.push((Value::uint(key), replacement));
    Value::map(entries).unwrap()
}

/// Rewrites a descriptor and container checksums while preserving its executable tail.
fn configure(source: &Path, target: &Path, change: impl FnOnce(Value) -> Value) {
    let original = fs::read(source).unwrap();
    let mut reader = Reader::open_auto(Cursor::new(&original), Limits::default()).unwrap();
    let tail = &original[reader.range().end as usize..];
    let mut writer = Writer::new(Vec::new()).unwrap();
    let mut change = Some(change);
    for section in reader.sections().cloned().collect::<Vec<_>>() {
        let info = section.type_info().unwrap();
        let mut bytes = reader.read_section(section.id()).unwrap();
        if section.kind() == APPLICATION {
            let application =
                Application::decode(&bytes, info.clone().unwrap(), Limits::default()).unwrap();
            let descriptor = application.value().required(0).unwrap();
            let config = change.take().unwrap()(descriptor.required(0).unwrap());
            let value = replace(
                application.value().clone(),
                0,
                replace(descriptor, 0, config),
            );
            bytes = b"JANEXAPP".to_vec();
            cbor::write_sized(&mut bytes, &value).unwrap();
        }
        writer
            .write_section(section.id(), section.kind(), &bytes, info)
            .unwrap();
    }
    let mut metadata = reader.metadata().as_map().unwrap();
    metadata.retain(|(key, _)| key.as_u64().unwrap() != 0);
    let mut bytes = writer.finish(Value::map(metadata).unwrap()).unwrap();
    bytes.extend(tail);
    fs::write(target, bytes).unwrap();
}

/// Encodes an external URI without prevalidating deliberately malformed fixtures.
fn requirement(uri: &str) -> Value {
    Value::map([
        (Value::uint(0), Value::uint(1)),
        (Value::uint(1), Value::text(uri)),
    ])
    .unwrap()
}

/// Runs a JDK build tool and reports captured diagnostics on failure.
fn tool(directory: &Path, name: &str, arguments: &[&str]) {
    let output = Command::new(name)
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Writes an automatic module or agent from one compiled Java class.
fn jar(target: &Path, name: &str, class: &Path, manifest: &str) {
    let mut zip = ZipWriter::new(fs::File::create(target).unwrap());
    zip.start_file("META-INF/MANIFEST.MF", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(manifest.as_bytes()).unwrap();
    zip.start_file(name, SimpleFileOptions::default()).unwrap();
    zip.write_all(&fs::read(class).unwrap()).unwrap();
    zip.finish().unwrap();
}

/// Launches both implementations and verifies invalid modules fail before agent execution.
fn check(target: &Path, marker: &Path, directory: &Path, success: bool, java: Option<&Path>) {
    check_launch(target, marker, directory, success, success, java);
}

/// Distinguishes candidate-selection failures from module-layer initialization failures.
fn check_launch(
    target: &Path,
    marker: &Path,
    directory: &Path,
    preparation_success: bool,
    success: bool,
    java: Option<&Path>,
) {
    let mut options = RunOptions::new(target);
    options.allow_unsigned = true;
    options.java.java = Some(java.unwrap_or_else(|| Path::new("java")).into());
    let prepared = prepare(&options);
    assert_eq!(
        prepared.is_ok(),
        preparation_success,
        "{target:?}: {prepared:?}"
    );
    assert!(!marker.exists(), "agent ran during native preparation");
    if let Ok(plan) = prepared {
        let output = plan
            .command()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(
            output.status.success() == success,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if success {
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "modules-ok");
            assert_eq!(fs::read(marker).unwrap(), b"agent");
            fs::remove_file(marker).unwrap();
        } else {
            assert!(!marker.exists(), "agent ran before module failure");
            assert!(!String::from_utf8_lossy(&output.stdout).contains("modules-ok"));
        }
    }
    let output = Command::new(java.unwrap_or_else(|| Path::new("java")))
        .arg(format!("-Djava.io.tmpdir={}", directory.display()))
        .arg("-jar")
        .arg(target)
        .output()
        .unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{target:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if success {
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "modules-ok");
        assert_eq!(fs::read(marker).unwrap(), b"agent");
        fs::remove_file(marker).unwrap();
    } else {
        assert!(!marker.exists(), "agent ran before module failure");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("modules-ok"));
    }
    assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
}

#[test]
fn virtual_modules_resolve_before_agents_and_respect_overlays_and_exact_versions() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), r#"
/// Checks that a module preparation process never invokes an agent or application entry point.
public class Main {
    /// Requires exactly one agent invocation before reporting successful application execution.
    public static void main(String[] args) {
        if (!"once".equals(System.getProperty("agent.ran"))) throw new AssertionError("agent missing");
        System.out.println("modules-ok");
    }
}

"#).unwrap();
    fs::write(temp.path().join("Agent.java"), r#"
/// Records any execution occurring before module validation completes.
public class Agent {
    /// Fails on repeated execution and writes an externally observable marker.
    public static void premain(String option) throws Exception {
        if (System.setProperty("agent.ran", "once") != null) throw new AssertionError("agent repeated");
        java.nio.file.Files.write(java.nio.file.Paths.get(option), "agent".getBytes("UTF-8"));
    }
}
"#).unwrap();
    fs::create_dir_all(temp.path().join("library/auto")).unwrap();
    fs::write(
        temp.path().join("library/auto/Library.java"),
        "package auto; /// Supplies a package for automatic-module discovery.\npublic class Library {}",
    )
    .unwrap();
    for (source, destination) in [
        ("Main.java", "main"),
        ("Agent.java", "agent"),
        ("library/auto/Library.java", "automatic"),
    ] {
        tool(
            temp.path(),
            "javac",
            &["--release", "8", "-d", destination, source],
        );
    }
    jar(
        &temp.path().join("agent.jar"),
        "Agent.class",
        &temp.path().join("agent/Agent.class"),
        "Manifest-Version: 1.0\r\nPremain-Class: Agent\r\nAutomatic-Module-Name: agent.library\r\n\r\n",
    );
    jar(
        &temp.path().join("auto-library-1.2.jar"),
        "auto/Library.class",
        &temp.path().join("automatic/auto/Library.class"),
        "Manifest-Version: 1.0\r\n\r\n",
    );
    fs::create_dir_all(temp.path().join("explicit")).unwrap();
    fs::write(
        temp.path().join("explicit/module-info.java"),
        "/// Provides an explicit descriptor with an exact version.\nmodule sample.library {}",
    )
    .unwrap();
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "9",
            "-d",
            "explicit-classes",
            "explicit/module-info.java",
        ],
    );
    tool(
        temp.path(),
        "jar",
        &[
            "--create",
            "--file",
            "explicit.jar",
            "--module-version",
            "2+build.7",
            "-C",
            "explicit-classes",
            ".",
        ],
    );
    let source = temp.path().join("source.janex");
    let marker = temp.path().join("agent-marker");
    let launch_directory = temp.path().join("launch");
    fs::create_dir(&launch_directory).unwrap();
    let mut options = PackOptions::new(temp.path().join("main"), &source);
    options.main_class = Some("Main".into());
    options.class_path.push(temp.path().join("agent.jar"));
    options.module_path = vec![
        temp.path().join("auto-library-1.2.jar"),
        temp.path().join("explicit.jar"),
    ];
    options.with_launcher = true;
    pack(&options).unwrap();
    let base = temp.path().join("base.janex");
    configure(&source, &base, |config| {
        let class_path = config.required(3).unwrap().as_array().unwrap();
        let agent = Value::map([
            (Value::uint(0), class_path[1].clone()),
            (Value::uint(1), Value::text(marker.to_str().unwrap())),
        ])
        .unwrap();
        replace(
            replace(config, 3, Value::array([class_path[0].clone()])),
            4,
            Value::array([agent]),
        )
    });
    let system = Command::new("java")
        .args(["--describe-module", "java.base"])
        .output()
        .unwrap();
    assert!(system.status.success());
    let system = String::from_utf8(system.stdout).unwrap();
    let system = system.lines().next().unwrap();
    let selected = [
        format!("pkg:janex/java-module/{system}"),
        "pkg:janex/java-module/auto.library".into(),
        "pkg:janex/java-module/auto.library@1.2".into(),
        "pkg:janex/java-module/auto.library".into(),
        "pkg:janex/java-module/sample.library@2%2Bbuild.7".into(),
    ];
    let valid = temp.path().join("valid.janex");
    configure(&base, &valid, |config| {
        let mut paths = config.required(2).unwrap().as_array().unwrap();
        // Virtual entries may occur before, between, or after their physical providers.
        paths.insert(0, requirement(&selected[0]));
        paths.insert(2, requirement(&selected[1]));
        paths.extend(selected[2..].iter().map(|uri| requirement(uri)));
        replace(config, 2, Value::array(paths))
    });
    check(&valid, &marker, &launch_directory, true, None);
    // A classpath application can still require observable system modules and exact versions.
    for (index, (uri, success)) in [
        (selected[0].as_str(), true),
        ("pkg:janex/java-module/java.base@0", false),
        ("pkg:janex/java-module/absent.module", false),
    ]
    .into_iter()
    .enumerate()
    {
        let target = temp
            .path()
            .join(format!("classpath-requirement-{index}.janex"));
        configure(&base, &target, |config| {
            replace(config, 2, Value::array([requirement(uri)]))
        });
        check(&target, &marker, &launch_directory, success, None);
    }
    // Access validation belongs to the actual JVM, but must still precede agent premain.
    let bad_access = temp.path().join("bad-access.janex");
    configure(&valid, &bad_access, |config| {
        replace(
            config,
            5,
            Value::array([Value::text(
                "--add-opens=java.base/missing.package=ALL-UNNAMED",
            )]),
        )
    });
    check_launch(&bad_access, &marker, &launch_directory, true, false, None);

    for (index, uri) in [
        "pkg:janex/java-module/absent.module",
        "pkg:janex/java-module/auto.library@2",
        "pkg:janex/java-module/sample.library@2%2Bbuild.8",
        "pkg:janex/java-module/java.base@0",
    ]
    .iter()
    .enumerate()
    {
        let target = temp.path().join(format!("missing-{index}.janex"));
        configure(&valid, &target, |config| {
            let mut paths = config.required(2).unwrap().as_array().unwrap();
            paths.push(requirement(uri));
            replace(config, 2, Value::array(paths))
        });
        check(&target, &marker, &launch_directory, false, None);
    }
    let absent = requirement("pkg:janex/java-module/absent.module");
    let inactive = Value::map([
        (
            Value::uint(0),
            Value::map([(Value::uint(1), Value::text("unknown-os"))]).unwrap(),
        ),
        (Value::uint(2), Value::array([absent.clone()])),
    ])
    .unwrap();
    let cleared = temp.path().join("cleared.janex");
    configure(&base, &cleared, |config| {
        let clear = Value::map([
            (Value::uint(2), Value::null()),
            (Value::uint(6), Value::array([inactive.clone()])),
        ])
        .unwrap();
        replace(
            replace(config, 2, Value::array([absent.clone()])),
            6,
            Value::array([clear]),
        )
    });
    check(&cleared, &marker, &launch_directory, true, None);
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        let java =
            PathBuf::from(home)
                .join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" });
        check(&valid, &marker, &launch_directory, false, Some(&java));
        check(&cleared, &marker, &launch_directory, true, Some(&java));
        let virtual_only = temp.path().join("virtual-only.janex");
        configure(&base, &virtual_only, |config| {
            replace(
                config,
                2,
                Value::array([requirement("pkg:janex/java-module/java.base")]),
            )
        });
        check(
            &virtual_only,
            &marker,
            &launch_directory,
            false,
            Some(&java),
        );
    }
    // Inactive declarations still require valid syntax and correct path placement.
    for (index, uri) in [
        "pkg:janex/java-module/%6Aava.base",
        "pkg:JANEX/java-module/java.base",
        "pkg:janex/Java-module/java.base",
        "pkg:janex/java-module/java.base?x=y",
        "pkg:janex/java-module/java.base#subpath",
        "pkg:janex/java-module/nested/java.base",
        "pkg:janex/java-module/java.base@",
        "pkg:janex/java-module/sample.library@2%2bbuild.7",
        "pkg:janex/java-module/",
        "pkg:janex/java-module/%FF",
    ]
    .iter()
    .enumerate()
    {
        let target = temp.path().join(format!("invalid-{index}.janex"));
        configure(&base, &target, |config| {
            let invalid = replace(inactive.clone(), 2, Value::array([requirement(uri)]));
            replace(config, 6, Value::array([invalid]))
        });
        check(&target, &marker, &launch_directory, false, None);
    }
    for key in [3, 4] {
        let target = temp.path().join(format!("placement-{key}.janex"));
        configure(&base, &target, |config| {
            let entry = if key == 3 {
                absent.clone()
            } else {
                Value::map([
                    (Value::uint(0), absent.clone()),
                    (Value::uint(1), Value::text("")),
                ])
                .unwrap()
            };
            replace(
                config,
                6,
                Value::array([replace(inactive.clone(), key, Value::array([entry]))]),
            )
        });
        check(&target, &marker, &launch_directory, false, None);
    }
}

#[test]
fn java_module_purls_match_native_decoding_and_placement() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/build/libs/janex-bootstrap.jar");
    let fixture_class = project.join(
        "janex-reader/src/testFixtures/java/org/glavo/janex/reader/ModuleRequirementTest.java",
    );
    tool(
        temp.path(),
        "javac",
        &[
            "--release",
            "8",
            "-cp",
            bootstrap.to_str().unwrap(),
            "-d",
            ".",
            fixture_class.to_str().unwrap(),
        ],
    );
    let mut uris = vec![
        "pkg:janex/java-module/java.base".to_owned(),
        "pkg:janex/java-module/java.base@".to_owned(),
        "pkg:janex/java-module/java.base?x=y".to_owned(),
        "pkg:janex/java-module/java.base#path".to_owned(),
    ];
    for character in (0..=255).filter_map(char::from_u32).chain(['中', '🚀']) {
        for (name, version) in [
            (format!("a{character}b"), "1".to_owned()),
            ("sample.library".to_owned(), format!("1{character}2")),
        ] {
            let mut purl = packageurl::PackageUrl::new("janex", name).unwrap();
            purl.with_namespace("java-module").unwrap();
            purl.with_version(version).unwrap();
            let uri = purl.to_string();
            uris.extend([
                uri.replace("pkg:", "PKG:"),
                uri.replace("pkg:", "pkg://"),
                uri.replace("/java-module/", "/java-module//"),
                uri.replace("/java-module/", "/java%2Dmodule/"),
                uri.clone(),
            ]);
            for i in 0..uri.len() {
                let mut changed = uri.as_bytes().to_vec();
                changed[i] ^= 1;
                if let Ok(changed) = String::from_utf8(changed) {
                    // Limit mutations to the requirement payload so successful values stay Janex PURLs.
                    if i >= "pkg:janex/java-module/".len() {
                        uris.push(changed);
                    }
                }
            }
        }
    }
    let mut vectors = Vec::new();
    vectors.extend(((uris.len() * 2) as u32).to_be_bytes());
    for uri in &uris {
        for module in [true, false] {
            vectors.extend((uri.len() as u32).to_be_bytes());
            vectors.extend(uri.as_bytes());
            vectors.push(u8::from(module));
            let decoded = PathEntry::from_value(&requirement(uri), module)
                .ok()
                .and_then(|entry| entry.module_requirement());
            vectors.push(u8::from(decoded.is_some()));
            if let Some((name, version)) = decoded {
                for text in [name, version.unwrap_or_default()] {
                    vectors.extend((text.len() as u32).to_be_bytes());
                    vectors.extend(text.as_bytes());
                }
            }
        }
    }
    let fixture = temp.path().join("modules.bin");
    fs::write(&fixture, vectors).unwrap();
    let classpath = std::env::join_paths([temp.path(), bootstrap.as_path()]).unwrap();
    let mut runtimes = vec![PathBuf::from("java")];
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        runtimes.push(PathBuf::from(home).join("bin").join(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
    }
    for java in runtimes {
        let output = Command::new(java)
            .arg("-cp")
            .arg(&classpath)
            .arg("org.glavo.janex.reader.ModuleRequirementTest")
            .arg(&fixture)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

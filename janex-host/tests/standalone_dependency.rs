// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Standalone acquisition, native cache interoperability, and imported Java resources.

#[path = "support/http.rs"]
mod http;

use janex_format::{
    application::PathEntry,
    checksum::{Algorithm, Checksum},
};
use janex_host::{
    dependency::{DependencyOptions, resolve},
    pack::{PackOptions, pack},
    run::{LaunchMode, RunOptions, prepare},
};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
use zip::{ZipWriter, write::SimpleFileOptions};

/// Runs a fixture compiler, reporting its diagnostics on failure.
fn javac(directory: &Path, args: &[&str]) {
    let output = Command::new("javac")
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Requires the selected application to complete with its expected output.
fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "remote-ok");
}

/// Builds a standalone command using an explicit cache and loopback Maven repository.
fn standalone(java: &Path, package: &Path, options: &DependencyOptions) -> Command {
    let mut command = Command::new(java);
    command
        .arg(format!(
            "-Djanex.dependencyCache={}",
            options.cache_directory.as_ref().unwrap().display()
        ))
        .arg(format!(
            "-Djanex.mavenRepository={}",
            options.maven_repository
        ))
        .arg(format!("-Djanex.offline={}", options.offline))
        .arg(format!("-Djanex.refreshDependencies={}", options.refresh))
        .arg("-jar")
        .arg(package);
    command
}

/// Creates a deflated multi-release JAR with a filename-derived automatic module and a link.
fn library(directory: &Path, zip64: bool) -> Vec<u8> {
    fs::write(
        directory.join("Library.java"),
        r#"
package library;
public class Library {
    public static java.net.URL resource(String name) { return Library.class.getResource(name); }
}
"#,
    )
    .unwrap();
    javac(
        directory,
        &["--release", "8", "-d", "library", "Library.java"],
    );
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .unix_permissions(0o640)
        .large_file(zip64);
    for (name, bytes) in [
        (
            "META-INF/MANIFEST.MF",
            b"Manifest-Version: 1.0\r\nMulti-Release: true\r\nClass-Path: forbidden.jar\r\n\r\n"
                .to_vec(),
        ),
        (
            "library/Library.class",
            fs::read(directory.join("library/library/Library.class")).unwrap(),
        ),
        ("library/value.txt", b"base".to_vec()),
        (
            "META-INF/versions/9/library/value.txt",
            b"selected".to_vec(),
        ),
        (
            "META-INF/versions/999/library/value.txt",
            b"future".to_vec(),
        ),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.add_symlink("library/alias.txt", "value.txt", options)
        .unwrap();
    zip.finish().unwrap().into_inner()
}

#[test]
fn standalone_and_native_share_verified_dependencies_in_both_directions() {
    let temp = tempfile::tempdir().unwrap();
    let server = http::Server::new();
    let library = library(temp.path(), true);
    fs::write(temp.path().join("remote-library-1.2.jar"), &library).unwrap();
    fs::write(temp.path().join("Main.java"), r#"
package app;
import java.nio.file.*;
public class Main {
    public static void main(String[] args) throws Exception {
        java.net.URL url = library.Library.resource("alias.txt");
        String value;
        try (java.io.InputStream input = url.openStream()) {
            java.io.ByteArrayOutputStream output = new java.io.ByteArrayOutputStream();
            for (int ch; (ch = input.read()) >= 0;) output.write(ch);
            value = output.toString("UTF-8");
        }
        if (!value.equals(System.getProperty("java.version").startsWith("1.8.") ? "base" : "selected")) throw new AssertionError(value);
        if (url.getProtocol().equals("janex")) {
            Path path = Paths.get(url.toURI());
            if (!Files.isDirectory(path.getParent())) throw new AssertionError();
            if (!new String(Files.readAllBytes(path), "UTF-8").equals(value)) throw new AssertionError();
            Object mode = Files.getAttribute(path, "janex:permissions");
            if (!Integer.valueOf(0640).equals(mode)) throw new AssertionError(mode);
        }
        try (java.io.InputStream input = library.Library.resource("/META-INF/MANIFEST.MF").openStream()) {
            if (new java.util.jar.Manifest(input).getMainAttributes().getValue("Class-Path") != null) throw new AssertionError();
        }
        System.out.println("remote-ok");
    }
}
"#).unwrap();
    javac(
        temp.path(),
        &[
            "--release",
            "8",
            "-cp",
            "library",
            "-d",
            "classes",
            "Main.java",
        ],
    );
    server.file("/remote-library-1.2.jar", &library);
    server.file(
        "/maven&mirror/example/remote-library/1.2/remote-library-1.2.jar",
        &library,
    );
    for module in [false, true] {
        let (source, uri, algorithm) = if module {
            fs::write(
                temp.path().join("module-info.java"),
                "module sample.app { requires remote.library; }",
            )
            .unwrap();
            javac(
                temp.path(),
                &[
                    "--release",
                    "9",
                    "--module-path",
                    "remote-library-1.2.jar",
                    "-d",
                    "modules",
                    "module-info.java",
                    "Main.java",
                ],
            );
            (
                "modules",
                format!(
                    "pkg:maven/example/remote-library@1.2?repository_url={}%2Fmaven%26mirror",
                    server.url.replace('/', "%2F")
                ),
                Algorithm::Sha512,
            )
        } else {
            (
                "classes",
                format!("{}/remote-library-1.2.jar", server.url),
                Algorithm::Sha256,
            )
        };
        let checksum = Checksum::compute(algorithm, library.as_slice()).unwrap();
        let mut packing = PackOptions::new(
            temp.path().join(source),
            temp.path().join(format!("app-{module}.janex")),
        );
        packing.main_class = Some("app.Main".into());
        packing.java_version = Some(format!("vers:jep322/>={}", if module { 9 } else { 8 }));
        packing.with_launcher = true;
        let entries = if module {
            packing.main_module = Some("sample.app".into());
            &mut packing.external_module_path
        } else {
            &mut packing.external_class_path
        };
        entries.push(PathEntry::External {
            uri: uri.clone(),
            checksum: Some(checksum.clone()),
        });
        pack(&packing).unwrap();
        let mut dependencies = DependencyOptions {
            cache_directory: Some(temp.path().join(format!("cache-{module}"))),
            maven_repository: format!("{}/maven", server.url),
            ..Default::default()
        };
        // Native acquisition produces the record consumed by the Java offline launcher.
        assert_eq!(
            resolve(&uri, Some(&checksum), &dependencies, false)
                .unwrap()
                .bytes,
            library
        );
        dependencies.offline = true;
        let mut runtimes = vec![PathBuf::from("java")];
        if !module && let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
            runtimes.push(Path::new(&home).join("bin").join(if cfg!(windows) {
                "java.exe"
            } else {
                "java"
            }));
        }
        for java in &runtimes {
            success(
                standalone(java, &packing.output, &dependencies)
                    .output()
                    .unwrap(),
            );
        }
        // Java acquisition produces an independent cache consumed by both native launch modes.
        dependencies.cache_directory = Some(temp.path().join(format!("java-cache-{module}")));
        dependencies.offline = false;
        success(
            standalone(runtimes.last().unwrap(), &packing.output, &dependencies)
                .output()
                .unwrap(),
        );
        dependencies.offline = true;
        for mode in [LaunchMode::Bootstrap, LaunchMode::Direct] {
            let mut run = RunOptions::new(&packing.output);
            run.allow_unsigned = true;
            run.launch_mode = mode;
            run.dependencies = dependencies.clone();
            success(
                prepare(&run)
                    .unwrap()
                    .command()
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .unwrap(),
            );
        }
        assert_eq!(
            server.requests.lock().unwrap().len(),
            if module { 4 } else { 2 }
        );
        if module {
            let home = temp.path().join("maven-home");
            let candidate =
                home.join(".m2/repository/example/remote-library/1.2/remote-library-1.2.jar");
            fs::create_dir_all(candidate.parent().unwrap()).unwrap();
            for (index, valid) in [true, false].iter().enumerate() {
                let original = if *valid { library.as_slice() } else { b"wrong" };
                fs::write(&candidate, original).unwrap();
                dependencies.cache_directory =
                    Some(temp.path().join(format!("maven-cache-{index}")));
                dependencies.offline = false;
                let before = server.requests.lock().unwrap().len();
                success(
                    standalone(Path::new("java"), &packing.output, &dependencies)
                        .env("HOME", &home)
                        .env("USERPROFILE", &home)
                        .output()
                        .unwrap(),
                );
                assert_eq!(fs::read(&candidate).unwrap(), original);
                assert_eq!(
                    server.requests.lock().unwrap().len(),
                    before + usize::from(!valid)
                );
                dependencies.offline = true;
                assert_eq!(
                    resolve(&uri, Some(&checksum), &dependencies, false)
                        .unwrap()
                        .bytes,
                    library
                );
            }
        }
    }
}

#[test]
fn standalone_cache_rejects_bad_responses_and_repairs_corruption() {
    let temp = tempfile::tempdir().unwrap();
    let server = http::Server::new();
    let library = library(temp.path(), false);
    fs::write(temp.path().join("Main.java"), "public class Main { public static void main(String[] args) { System.out.println(\"remote-ok\"); } }").unwrap();
    javac(
        temp.path(),
        &["--release", "8", "-d", "classes", "Main.java"],
    );
    let checksum = Checksum::compute(Algorithm::Sha256, library.as_slice()).unwrap();
    let mut packing = PackOptions::new(temp.path().join("classes"), temp.path().join("app.janex"));
    packing.main_class = Some("Main".into());
    packing.with_launcher = true;
    packing.external_class_path.push(PathEntry::External {
        uri: format!("{}/library.jar", server.url),
        checksum: Some(checksum),
    });
    pack(&packing).unwrap();
    let mut options = DependencyOptions {
        cache_directory: Some(temp.path().join("cache")),
        offline: true,
        ..Default::default()
    };
    let java = Path::new("java");
    for (name, checksum) in [
        ("unpinned", None),
        (
            "insecure-http-hash",
            Some(Checksum::compute(Algorithm::Xxh3_64, library.as_slice()).unwrap()),
        ),
    ] {
        let mut rejected = packing.clone();
        rejected.output = temp.path().join(format!("{name}.janex"));
        rejected.external_class_path = vec![PathEntry::External {
            uri: format!("{}/library.jar", server.url),
            checksum,
        }];
        pack(&rejected).unwrap();
        let mut online = options.clone();
        online.offline = false;
        assert!(
            !standalone(java, &rejected.output, &online)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    assert!(
        !standalone(java, &packing.output, &options)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!options.cache_directory.as_ref().unwrap().exists());
    assert!(server.requests.lock().unwrap().is_empty());
    options.offline = false;
    for response in [
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 536870913\r\nConnection: close\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad".to_vec(),
        b"HTTP/1.1 302 Found\r\nLocation: /library.jar\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        b"HTTP/1.1 302 Found\r\nLocation: file:///library.jar\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    ] {
        server.raw("/library.jar", response);
        assert!(!standalone(java, &packing.output, &options).output().unwrap().status.success());
        assert!(!options.cache_directory.as_ref().unwrap().join("files").exists());
        assert!(!options.cache_directory.as_ref().unwrap().join("metadata").exists());
    }
    server.raw("/library.jar", b"HTTP/1.1 302 Found\r\nLocation: /download\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
    server.file("/download", &library);
    success(
        standalone(java, &packing.output, &options)
            .output()
            .unwrap(),
    );
    let sha = Checksum::compute(Algorithm::Sha256, library.as_slice()).unwrap();
    let hash: String = sha
        .digest()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let cache = options
        .cache_directory
        .as_ref()
        .unwrap()
        .join("files/sha256")
        .join(&hash[..2])
        .join(&hash[2..])
        .join("library.jar");
    let original = fs::read(&cache).unwrap();
    let requests = server.requests.lock().unwrap().len();
    options.offline = true;
    fs::write(&cache, b"corrupt").unwrap();
    assert!(
        !standalone(java, &packing.output, &options)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(server.requests.lock().unwrap().len(), requests);
    options.offline = false;
    success(
        standalone(java, &packing.output, &options)
            .output()
            .unwrap(),
    );
    assert_eq!(fs::read(&cache).unwrap(), original);
    let requests = server.requests.lock().unwrap().len();
    // A second origin can bind the same pinned content without another download.
    let mut mirror = packing.clone();
    mirror.output = temp.path().join("mirror.janex");
    let mirror_uri = format!("{}/mirror/library.jar", server.url);
    mirror.external_class_path = vec![PathEntry::External {
        uri: mirror_uri.clone(),
        checksum: Some(sha.clone()),
    }];
    pack(&mirror).unwrap();
    success(standalone(java, &mirror.output, &options).output().unwrap());
    assert_eq!(server.requests.lock().unwrap().len(), requests);
    options.offline = true;
    assert_eq!(
        resolve(&mirror_uri, Some(&sha), &options, false)
            .unwrap()
            .bytes,
        library
    );
    // Invalid request metadata is rejected offline and repaired from verified content online.
    for entry in fs::read_dir(
        options
            .cache_directory
            .as_ref()
            .unwrap()
            .join("metadata/urls"),
    )
    .unwrap()
    {
        fs::write(entry.unwrap().path(), b"invalid").unwrap();
    }
    assert!(
        !standalone(java, &packing.output, &options)
            .output()
            .unwrap()
            .status
            .success()
    );
    options.offline = false;
    success(
        standalone(java, &packing.output, &options)
            .output()
            .unwrap(),
    );
    assert_eq!(server.requests.lock().unwrap().len(), requests);
    options.refresh = true;
    server.file("/download", b"wrong checksum");
    assert!(
        !standalone(java, &packing.output, &options)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&cache).unwrap(), original);
    options.offline = true;
    assert!(
        !standalone(java, &packing.output, &options)
            .output()
            .unwrap()
            .status
            .success()
    );
    options.refresh = false;
    drop(server);
    success(
        standalone(java, &packing.output, &options)
            .output()
            .unwrap(),
    );
}

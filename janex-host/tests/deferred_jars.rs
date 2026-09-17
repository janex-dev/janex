// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! External resource payloads remain deferred across the reader/bootstrap boundary.

use janex_format::{cbor::Value, container::Writer};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn external_jars_exceeding_the_java_heap_remain_lazy() {
    let temp = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bootstrap = project.join("janex-bootstrap/build/libs/janex-bootstrap.jar");
    let compiled = Command::new("javac").args(["--release", "8", "-cp"]).arg(&bootstrap)
        .arg("-d").arg(temp.path()).arg(project.join(
            "janex-bootstrap/src/testFixtures/java/org/glavo/janex/bootstrap/loader/JarResourcesTest.java",
        )).output().unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let snapshot = temp.path().join("empty.janex");
    fs::write(
        &snapshot,
        Writer::new(Vec::new())
            .unwrap()
            .finish(Value::empty_map())
            .unwrap(),
    )
    .unwrap();
    let classpath = std::env::join_paths([temp.path(), bootstrap.as_path()]).unwrap();
    let mut runtimes = vec![PathBuf::from("java")];
    if let Some(home) = std::env::var_os("JANEX_TEST_JAVA8_HOME") {
        runtimes.push(Path::new(&home).join("bin").join(if cfg!(windows) {
            "java.exe"
        } else {
            "java"
        }));
    }
    for java in runtimes {
        let output = Command::new(java)
            .args(["-Xmx32m", "-cp"])
            .arg(&classpath)
            .arg("org.glavo.janex.bootstrap.loader.JarResourcesTest")
            .arg(&snapshot)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

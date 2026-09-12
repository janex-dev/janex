// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! CMS signing and authenticated Java execution through the public command-line interface.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Resolves public cryptographic fixtures shared with the format tests.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../janex-signature/tests/fixtures/cms")
        .join(name)
}

/// Reports useful process diagnostics without dumping key material.
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_signs_with_an_encrypted_key_and_requires_every_requested_signer() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), "public class Main { public static void main(String[] args) { System.out.println(\"authenticated\"); } }").unwrap();
    success(
        &Command::new("javac")
            .current_dir(temp.path())
            .args(["--release", "8", "-d", "classes", "Main.java"])
            .output()
            .unwrap(),
    );
    let target = temp.path().join("signed.janex");
    let packed = Command::new(env!("CARGO_BIN_EXE_janex"))
        .arg("pack")
        .arg(temp.path().join("classes"))
        .arg("--output")
        .arg(&target)
        .args(["--main-class", "Main", "--cms-certificate"])
        .arg(fixture("rsa256.cert.pem"))
        .arg("--cms-key")
        .arg(fixture("rsa256.encrypted.pem"))
        .arg("--key-password-file")
        .arg(fixture("password.txt"))
        .output()
        .unwrap();
    success(&packed);
    let run = |certificates: &[&str], allow_unsigned: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_janex"));
        command.args(["run", "--java", "java"]);
        for certificate in certificates {
            command
                .arg("--trust-cms-certificate")
                .arg(fixture(certificate));
        }
        if allow_unsigned {
            command.arg("--allow-unsigned");
        }
        command.arg(&target).output().unwrap()
    };
    let launched = run(&["rsa256.cert.pem"], false);
    success(&launched);
    assert_eq!(
        String::from_utf8(launched.stdout).unwrap().trim(),
        "authenticated"
    );
    for certificates in [
        &[][..],
        &["p256.cert.pem"][..],
        &["rsa256.cert.pem", "p256.cert.pem"][..],
    ] {
        assert_eq!(run(certificates, true).status.code(), Some(1));
    }
    fs::write(temp.path().join("wrong-password"), b"wrong\n").unwrap();
    let failed = Command::new(env!("CARGO_BIN_EXE_janex"))
        .arg("pack")
        .arg(temp.path().join("classes"))
        .arg("--output")
        .arg(temp.path().join("failed.janex"))
        .args(["--main-class", "Main", "--cms-certificate"])
        .arg(fixture("rsa256.cert.pem"))
        .arg("--cms-key")
        .arg(fixture("rsa256.encrypted.pem"))
        .arg("--key-password-file")
        .arg(temp.path().join("wrong-password"))
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert!(!temp.path().join("failed.janex").exists());
}

#[test]
fn cli_applies_supplied_revocation_lists_before_java_discovery() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("source")).unwrap();
    let target = temp.path().join("issued.janex");
    success(
        &Command::new(env!("CARGO_BIN_EXE_janex"))
            .arg("pack")
            .arg(temp.path().join("source"))
            .arg("--output")
            .arg(&target)
            .args(["--main-class", "Main", "--cms-certificate"])
            .arg(fixture("issued.cert.pem"))
            .arg("--cms-key")
            .arg(fixture("rsa256.key.pem"))
            .output()
            .unwrap(),
    );
    let revoked = Command::new(env!("CARGO_BIN_EXE_janex"))
        .args(["run", "--trust-cms-certificate"])
        .arg(fixture("issued.cert.pem"))
        .arg("--cms-issuer")
        .arg(fixture("ca.cert.pem"))
        .arg("--cms-crl")
        .arg(fixture("revoked.crl.pem"))
        .arg("--java")
        .arg(temp.path().join("must-not-start-java"))
        .arg(&target)
        .output()
        .unwrap();
    assert_eq!(revoked.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&revoked.stderr).contains("revoked"));
}

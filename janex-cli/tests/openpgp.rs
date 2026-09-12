// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! OpenPGP authentication and Java execution through the command-line trust interface.

use janex_format::{
    binary::Limits,
    cbor::Value,
    container::{Reader, Writer},
};
use janex_host::pack::{PackOptions, pack};
use janex_signature::openpgp::{self, Algorithm};
use pgp::{
    packet::{PacketHeader, SecretKey},
    types::{PacketLength, Password},
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::SystemTime,
};

/// Resolves public test key material without consulting a user keyring.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../janex-signature/tests/fixtures/openpgp")
        .join(name)
}

/// Reports subprocess errors without displaying key material.
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Signs a packaged file through the format API, preserving its section references and coverage.
fn signed(original: &[u8], complete: bool, kind: u8) -> Vec<u8> {
    let key_bytes = fs::read(fixture("ed25519.secret.pgp")).unwrap();
    let mut key_body = key_bytes.as_slice();
    let header = PacketHeader::try_from_reader(&mut key_body).unwrap();
    let PacketLength::Fixed(length) = header.packet_length() else {
        panic!("fixture length")
    };
    let key = SecretKey::try_from_reader(header, &key_body[..length as usize]).unwrap();
    let mut reader = Reader::open_auto(Cursor::new(original), Limits::default()).unwrap();
    let metadata = if complete {
        Value::map(
            reader
                .metadata()
                .as_map()
                .unwrap()
                .into_iter()
                .filter(|(key, _)| key.as_u64().ok() != Some(0)),
        )
        .unwrap()
    } else {
        Value::empty_map()
    };
    let mut writer = Writer::new(Vec::new()).unwrap();
    for section in reader.sections().cloned().collect::<Vec<_>>() {
        writer
            .write_section(
                section.id(),
                section.kind(),
                &reader.read_section(section.id()).unwrap(),
                section.type_info().unwrap(),
            )
            .unwrap();
    }
    writer
        .finish_with::<janex_host::Error>(metadata, kind, |input| {
            Ok(if kind == 2 {
                openpgp::sign(
                    input,
                    &key,
                    &Password::empty(),
                    Algorithm::Ed25519Sha256,
                    SystemTime::now(),
                )
            } else {
                Ok(vec![1])
            }?)
        })
        .unwrap()
}

#[test]
fn cli_authenticates_openpgp_before_launching_and_never_downgrades_a_pin() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), "public class Main { public static void main(String[] args) { System.out.println(\"openpgp authenticated\"); } }").unwrap();
    success(
        &Command::new("javac")
            .current_dir(temp.path())
            .args(["--release", "8", "-d", "classes", "Main.java"])
            .output()
            .unwrap(),
    );
    let target = temp.path().join("app.janex");
    let mut options = PackOptions::new(temp.path().join("classes"), &target);
    options.main_class = Some("Main".into());
    pack(&options).unwrap();
    let unsigned = fs::read(&target).unwrap();
    fs::remove_file(&target).unwrap();
    success(
        &Command::new(env!("CARGO_BIN_EXE_janex"))
            .arg("pack")
            .arg(temp.path().join("classes"))
            .arg("--output")
            .arg(&target)
            .args(["--main-class", "Main", "--openpgp-key"])
            .arg(fixture("ed25519.secret.pgp"))
            .output()
            .unwrap(),
    );
    let authenticated = fs::read(&target).unwrap();
    let run = |trust: Option<&str>, java: &str| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_janex"));
        command.args(["run", "--allow-unsigned", "--java", java]);
        if let Some(trust) = trust {
            command.arg("--trust-openpgp-key").arg(fixture(trust));
        }
        command.arg(&target).output().unwrap()
    };
    for trust in ["ed25519.public.pgp", "ed25519.public.asc"] {
        let output = run(Some(trust), "java");
        success(&output);
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "openpgp authenticated"
        );
    }
    for (trust, message) in [
        (None, "pinned public key"),
        (Some("p256.public.pgp"), "pinned certificate"),
        (Some("ed25519.revoked.public.pgp"), "revoked"),
    ] {
        let rejected = run(trust, "must-not-start-java");
        assert_eq!(rejected.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&rejected.stderr)
        );
    }
    for (bytes, message) in [
        (unsigned.clone(), "signer pins require"),
        (signed(&unsigned, false, 2), "complete container"),
        (signed(&unsigned, true, 3), "OpenPGP-authenticated"),
    ] {
        fs::write(&target, bytes).unwrap();
        let rejected = run(Some("ed25519.public.pgp"), "must-not-start-java");
        assert_eq!(rejected.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&rejected.stderr)
        );
    }
    let mut damaged = authenticated;
    // Alter section content after the container and section magic, leaving signed metadata intact.
    damaged[16] ^= 1;
    fs::write(&target, damaged).unwrap();
    let rejected = run(Some("ed25519.public.pgp"), "must-not-start-java");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("checksum"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    let incompatible = Command::new(env!("CARGO_BIN_EXE_janex"))
        .args([
            "run",
            "--trust-openpgp-key",
            "public.asc",
            "--trust-cms-certificate",
            "signer.pem",
            "app.janex",
        ])
        .output()
        .unwrap();
    assert_eq!(incompatible.status.code(), Some(2));
}

#[test]
fn cli_packs_with_encrypted_subkeys_and_rejects_conflicting_or_invalid_signing_options() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Main.java"), "public class Main { public static void main(String[] args) { System.out.println(args[0]); } }").unwrap();
    success(
        &Command::new("javac")
            .current_dir(temp.path())
            .args(["--release", "8", "-d", "classes", "Main.java"])
            .output()
            .unwrap(),
    );
    let password = temp.path().join("password.txt");
    fs::write(&password, b"public-fixture-password\r\n").unwrap();
    let target = temp.path().join("encrypted.janex");
    let packing = |key: &str, extra: &[&str], with_password: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_janex"));
        command
            .arg("pack")
            .arg(temp.path().join("classes"))
            .arg("--output")
            .arg(&target)
            .args([
                "--main-class",
                "Main",
                "--argument",
                "encrypted subkey",
                "--openpgp-key",
            ])
            .arg(fixture(key))
            .args(extra);
        if with_password {
            command.arg("--key-password-file").arg(&password);
        }
        command.stdin(std::process::Stdio::null()).output().unwrap()
    };
    let expected = openpgp::OpenPgpSignature::decode(
        &fs::read(fixture("encrypted.signature.pgp")).unwrap(),
        janex_signature::Limits::default(),
    )
    .unwrap()
    .issuer()
    .to_string();
    for key in ["encrypted.secret.pgp", "encrypted.secret.asc"] {
        success(&packing(
            key,
            &[
                "--openpgp-signing-key",
                &expected,
                "--openpgp-algorithm",
                "ed25519-sha512",
            ],
            true,
        ));
        let output = Command::new(env!("CARGO_BIN_EXE_janex"))
            .args(["run", "--trust-openpgp-key"])
            .arg(fixture("encrypted.public.pgp"))
            .arg(&target)
            .output()
            .unwrap();
        success(&output);
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "encrypted subkey"
        );
        let original = fs::read(&target).unwrap();
        assert!(!packing(key, &[], true).status.success());
        assert_eq!(original, fs::read(&target).unwrap());
        fs::remove_file(&target).unwrap();
    }
    for (extra, with_password, status, message) in [
        (vec![], false, 1, "--key-password-file"),
        (
            vec!["--openpgp-signing-key", "01234567"],
            true,
            1,
            "complete hexadecimal fingerprint",
        ),
        (
            vec!["--openpgp-algorithm", "rsa-sha256"],
            true,
            1,
            "algorithm does not match",
        ),
        (
            vec!["--cms-certificate", "unused.pem", "--cms-key", "unused.key"],
            true,
            2,
            "cannot be used with",
        ),
    ] {
        let output = packing("encrypted.secret.pgp", &extra, with_password);
        assert_eq!(output.status.code(), Some(status));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!target.exists());
    }
    fs::write(&password, b"wrong password\n").unwrap();
    let output = packing("encrypted.secret.pgp", &[], true);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot unlock"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("wrong password"));
    assert!(!target.exists());
}

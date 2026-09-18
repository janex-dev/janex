// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Self-installation and replacement of real running executables in isolated homes.

use janex_format::checksum::{Algorithm, Checksum};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output},
};

/// Executable basename in native distribution archives.
const EXE: &str = if cfg!(windows) { "janex.exe" } else { "janex" };

/// Executes a fixture binary without using the user's Janex state.
fn invoke(executable: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(executable)
        .env("JANEX_HOME", home)
        .args(args)
        .output()
        .unwrap()
}

/// Returns successful stdout with diagnostics on failure.
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Creates a distribution with a root executable and an unselected traversal entry.
fn zip(path: &Path, executable: &[u8]) {
    let mut archive = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("../outside", options).unwrap();
    archive.write_all(b"must not be extracted").unwrap();
    archive
        .start_file(EXE, options.unix_permissions(0o755))
        .unwrap();
    archive.write_all(executable).unwrap();
    archive.finish().unwrap();
}

#[test]
fn self_install_and_update_replace_running_binaries_without_touching_apps_or_sdks() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("Janex's home");
    let source = Path::new(env!("CARGO_BIN_EXE_janex"));
    let bytes = fs::read(source).unwrap();
    let archive = temp.path().join("janex.zip");
    zip(&archive, &bytes);
    let installed = home.join("bin").join(EXE);
    let unmanaged = invoke(
        source,
        &home,
        &["self", "update", "--from", archive.to_str().unwrap()],
    );
    assert!(!unmanaged.status.success());
    assert!(!home.exists());
    success(invoke(source, &home, &["self", "install"]));
    assert_eq!(fs::read(&installed).unwrap(), bytes);
    for loader in ["init.sh", "init.fish", "init.ps1"] {
        let code = fs::read_to_string(home.join("shell").join(loader)).unwrap();
        assert!(code.contains("shell init"));
        assert!(!code.contains("target\\debug") && !code.contains("target/debug"));
    }
    fs::write(home.join("state/apps.cbor"), b"untouched app state").unwrap();
    fs::write(home.join("state/sdk.cbor"), b"untouched SDK state").unwrap();
    success(invoke(&installed, &home, &["self", "install"]));
    let digest =
        Checksum::compute(Algorithm::Sha256, fs::read(&archive).unwrap().as_slice()).unwrap();
    let hex: String = digest.digest().iter().map(|b| format!("{b:02x}")).collect();
    for _ in 0..2 {
        let output = success(invoke(
            &installed,
            &home,
            &[
                "self",
                "update",
                "--from",
                archive.to_str().unwrap(),
                "--sha256",
                &hex,
            ],
        ));
        assert!(output.contains("Updated Janex"));
        assert!(success(invoke(&installed, &home, &["--version"])).starts_with("janex "));
        assert_eq!(fs::read(&installed).unwrap(), bytes);
    }
    assert!(!home.join("bin/outside").exists());
    assert_eq!(
        fs::read(home.join("state/apps.cbor")).unwrap(),
        b"untouched app state"
    );
    assert_eq!(
        fs::read(home.join("state/sdk.cbor")).unwrap(),
        b"untouched SDK state"
    );
    let invalid = invoke(
        &installed,
        &home,
        &[
            "self",
            "update",
            "--from",
            archive.to_str().unwrap(),
            "--sha256",
            &"00".repeat(32),
        ],
    );
    assert!(!invalid.status.success());
    zip(&archive, b"not an executable");
    assert!(
        !invoke(
            &installed,
            &home,
            &["self", "update", "--from", archive.to_str().unwrap()]
        )
        .status
        .success()
    );
    fs::write(&archive, b"invalid zip").unwrap();
    assert!(
        !invoke(
            &installed,
            &home,
            &["self", "update", "--from", archive.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(&installed).unwrap(), bytes);
    assert!(success(invoke(&installed, &home, &["--version"])).starts_with("janex "));
}

#[test]
fn tar_xz_distributions_are_decoded_without_external_tools() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let source = Path::new(env!("CARGO_BIN_EXE_janex"));
    success(invoke(source, &home, &["self", "install"]));
    let bytes = fs::read(source).unwrap();
    let archive = temp.path().join("janex.tar.xz");
    let writer = lzma_rust2::XzWriter::new(
        fs::File::create(&archive).unwrap(),
        lzma_rust2::XzOptions::with_preset(0),
    )
    .unwrap();
    let mut tar = tar::Builder::new(writer);
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, EXE, bytes.as_slice()).unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    let installed = home.join("bin").join(EXE);
    success(invoke(
        &installed,
        &home,
        &["self", "update", "--from", archive.to_str().unwrap()],
    ));
    assert_eq!(fs::read(&installed).unwrap(), bytes);
    let mut corrupt = fs::read(&archive).unwrap();
    corrupt.truncate(corrupt.len() - 8);
    fs::write(&archive, corrupt).unwrap();
    assert!(
        !invoke(
            &installed,
            &home,
            &["self", "update", "--from", archive.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(installed).unwrap(), bytes);
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Integration commands never need to change real desktop associations in these tests.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

/// Invokes Janex with isolated home and desktop-data directories.
fn invoke(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_janex"))
        .env("JANEX_HOME", root.join("janex"))
        .env("HOME", root.join("home"))
        .env("XDG_DATA_HOME", root.join("data"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

/// Checks command success with diagnostics.
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn export_preserves_live_state_and_records_an_explicit_policy() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    success(&invoke(
        root,
        &[
            "integration",
            "export",
            "--output",
            "assets",
            "--allow-unsigned",
            "--trust-cms-certificate",
            "signer.pem",
        ],
    ));
    assert!(!root.join("janex").exists());
    assert!(!root.join("data").exists());
    assert!(!root.join("home/Applications").exists());
    assert!(
        fs::read_to_string(root.join("assets/README.txt"))
            .unwrap()
            .contains("Export does not create a Janex registration record")
    );
    #[cfg(windows)]
    {
        let bytes = fs::read(root.join("assets/janex.reg")).unwrap();
        assert_eq!(&bytes[..2], &[0xff, 0xfe]);
        let text = String::from_utf16(
            &bytes[2..]
                .chunks_exact(2)
                .map(|s| u16::from_le_bytes([s[0], s[1]]))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(text.contains("HKEY_CURRENT_USER\\Software\\Classes\\.janex\\OpenWithProgids"));
        assert!(!text.contains("UserChoice"));
        assert!(text.contains("--allow-unsigned"));
        assert!(text.contains("signer.pem"));
        assert!(text.contains("\\\"--\\\" \\\"%1\\\""));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let text =
            fs::read_to_string(root.join("assets/share/applications/org.glavo.janex.desktop"))
                .unwrap();
        assert!(text.contains("\"open\" \"--allow-unsigned\""));
        assert!(text.contains("\"--\" %f\n"));
        assert!(text.contains(root.to_str().unwrap()));
    }
    #[cfg(target_os = "macos")]
    {
        assert!(
            root.join("assets/Janex.app/Contents/MacOS/applet")
                .is_file()
        );
        let checked = Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(root.join("assets/Janex.app"))
            .output()
            .unwrap();
        success(&checked);
    }
    let status = invoke(root, &["integration", "status", "--json"]);
    success(&status);
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["registered"], false);
    success(&invoke(root, &["integration", "unregister"]));
    assert!(!root.join("janex").exists());
    assert!(
        !invoke(root, &["integration", "export", "--output", "assets"])
            .status
            .success()
    );
}

#[test]
fn invalid_scope_or_policy_does_not_create_assets() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec!["integration", "export", "--output", "bad", "--binfmt"],
        vec![
            "integration",
            "export",
            "--output",
            "bad",
            "--executable",
            "relative",
        ],
        vec![
            "integration",
            "export",
            "--output",
            "bad",
            "--cms-crl",
            "crl.pem",
        ],
        vec![
            "integration",
            "export",
            "--output",
            "bad",
            "--trust-cms-certificate",
            "a",
            "--trust-openpgp-key",
            "b",
        ],
    ] {
        assert!(!invoke(temp.path(), &args).status.success(), "{args:?}");
        assert!(!temp.path().join("bad").exists());
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn desktop_registration_updates_and_preserves_external_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    success(&invoke(root, &["integration", "register"]));
    success(&invoke(
        root,
        &["integration", "register", "--allow-unsigned"],
    ));
    let status = invoke(root, &["integration", "status", "--json"]);
    success(&status);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&status.stdout).unwrap()["complete"],
        true
    );
    let desktop = root.join("data/applications/org.glavo.janex.desktop");
    let original = fs::read(&desktop).unwrap();
    fs::write(&desktop, b"externally modified").unwrap();
    assert!(
        !invoke(root, &["integration", "unregister"])
            .status
            .success()
    );
    assert_eq!(fs::read(&desktop).unwrap(), b"externally modified");
    fs::write(&desktop, original).unwrap();
    success(&invoke(root, &["integration", "unregister"]));
    success(&invoke(root, &["integration", "unregister"]));
    assert!(!desktop.exists());
    assert!(!root.join("data/mime/packages/org.glavo.janex.xml").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn binfmt_export_is_offline_and_uses_magic_matching() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    success(&invoke(
        temp.path(),
        &[
            "integration",
            "export",
            "--system",
            "--binfmt",
            "--output",
            "assets",
            "--executable",
            "/opt/janex/bin/janex",
        ],
    ));
    let rule = fs::read_to_string(temp.path().join("assets/etc/binfmt.d/janex.conf")).unwrap();
    assert_eq!(
        rule,
        ":janex:M::JANEX\\x00\\x00\\x00::/usr/libexec/janex-binfmt:\n"
    );
    let helper = temp.path().join("assets/libexec/janex-binfmt");
    assert_eq!(
        fs::metadata(&helper).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(
        fs::read_to_string(helper)
            .unwrap()
            .contains("'/opt/janex/bin/janex' 'run' '--' \"$@\"")
    );
}

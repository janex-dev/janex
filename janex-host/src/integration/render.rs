// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Native registration assets and command-line escaping.

use super::*;

/// Linux rule with no credential, descriptor-passing, or argv-preservation flags.
pub(super) const BINFMT_RULE: &str = ":janex:M::JANEX\\x00\\x00\\x00::/usr/libexec/janex-binfmt:\n";

/// Generates native assets without changing installed handlers.
pub(super) fn plan(options: &Options) -> Result<Plan> {
    let mut plan = Plan {
        assets: Vec::new(),
        registry: Vec::new(),
        mime: None,
        desktop: None,
        bundle: None,
    };
    #[cfg(windows)]
    {
        let command = arguments(options, "open")
            .iter()
            .map(|s| windows_quote(s))
            .collect::<Vec<_>>()
            .join(" ")
            + " \"%1\"";
        for (key, name, value) in [
            ("org.glavo.janex.File", "", "Janex Application".to_owned()),
            ("org.glavo.janex.File\\shell\\open\\command", "", command),
            (
                ".janex\\OpenWithProgids",
                "org.glavo.janex.File",
                String::new(),
            ),
        ] {
            plan.registry.push(RegistryValue {
                key: format!("Software\\Classes\\{key}"),
                name: name.into(),
                value,
            });
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let data = if options.system {
            PathBuf::from("/usr/local/share")
        } else {
            match std::env::var_os("XDG_DATA_HOME").filter(|s| !s.is_empty()) {
                Some(value) => {
                    let path = PathBuf::from(value);
                    if !path.is_absolute() {
                        return Err(invalid("XDG_DATA_HOME must be absolute"));
                    }
                    path
                }
                None => env_path("HOME")?.join(".local/share"),
            }
        };
        for (relative, contents) in [
            ("mime/packages/org.glavo.janex.xml", mime_xml().to_owned()),
            ("applications/org.glavo.janex.desktop", desktop(options)),
        ] {
            plan.assets.push(Asset {
                path: data.join(relative),
                export: Path::new("share").join(relative),
                bytes: contents.into_bytes(),
                executable: false,
            });
        }
        plan.mime = Some(data.join("mime"));
        plan.desktop = Some(data.join("applications"));
        if options.binfmt {
            plan.assets.push(Asset {
                path: "/usr/libexec/janex-binfmt".into(),
                export: "libexec/janex-binfmt".into(),
                bytes: shell_handler(options, "run").into_bytes(),
                executable: true,
            });
            plan.assets.push(Asset {
                path: "/etc/binfmt.d/janex.conf".into(),
                export: "etc/binfmt.d/janex.conf".into(),
                bytes: BINFMT_RULE.as_bytes().to_vec(),
                executable: false,
            });
        }
    }
    #[cfg(target_os = "macos")]
    mac_bundle(options, &mut plan)?;
    Ok(plan)
}

/// Builds fixed arguments before the filename supplied by the operating system.
fn arguments<'a>(options: &'a Options, invocation: &'a str) -> Vec<&'a str> {
    let mut result = vec![
        options.executable.to_str().expect("validated path"),
        invocation,
    ];
    result.extend(options.trust_arguments.iter().map(String::as_str));
    result.push("--");
    result
}

/// Quotes one Windows argument, including trailing backslashes.
#[cfg(any(windows, test))]
fn windows_quote(value: &str) -> String {
    let mut output = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        output.extend(std::iter::repeat_n(
            '\\',
            if c == '"' { slashes * 2 + 1 } else { slashes },
        ));
        slashes = 0;
        output.push(c);
    }
    output.extend(std::iter::repeat_n('\\', slashes * 2));
    output.push('"');
    output
}

/// Encodes a Windows Registry Editor file, including its UTF-16LE byte-order marker.
pub(super) fn reg_file(system: bool, entries: &[RegistryValue]) -> Vec<u8> {
    let hive = if system {
        "HKEY_LOCAL_MACHINE"
    } else {
        "HKEY_CURRENT_USER"
    };
    let mut text = String::from("Windows Registry Editor Version 5.00\r\n");
    for entry in entries {
        let name = if entry.name.is_empty() {
            "@".into()
        } else {
            reg_quote(&entry.name)
        };
        text.push_str(&format!(
            "\r\n[{hive}\\{}]\r\n{name}={}\r\n",
            entry.key,
            reg_quote(&entry.value)
        ));
    }
    std::iter::once(0xfeff)
        .chain(text.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Escapes a quoted Registry Editor string.
fn reg_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Quotes one argument for a POSIX shell.
#[cfg(any(unix, test))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Generates a direct-execution adapter without reparsing filenames as shell code.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn shell_handler(options: &Options, invocation: &str) -> String {
    format!(
        "#!/bin/sh\nexec {} \"$@\"\n",
        arguments(options, invocation)
            .iter()
            .map(|s| shell_quote(s))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// Generates desktop MIME metadata for plain and prefixed Janex files.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn mime_xml() -> &'static str {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n  <mime-type type=\"application/x-janex\">\n    <comment>Janex Application</comment>\n    <glob pattern=\"*.janex\"/>\n    <magic priority=\"50\"><match type=\"string\" offset=\"0\" value=\"JANEX\\000\\000\\000\"/></magic>\n  </mime-type>\n</mime-info>\n"
}

/// Encodes both Desktop Entry value escaping and Exec argument escaping.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn desktop_quote(value: &str) -> String {
    let mut output = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => output.push_str("\\\\\\\\"),
            '"' | '`' | '$' => {
                output.push_str("\\\\");
                output.push(c);
            }
            '%' => output.push_str("%%"),
            _ => output.push(c),
        }
    }
    output.push('"');
    output
}

/// Creates a hidden application entry available to Open With menus.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn desktop(options: &Options) -> String {
    let command = arguments(options, "open")
        .iter()
        .map(|s| desktop_quote(s))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "[Desktop Entry]\nType=Application\nName=Janex\nNoDisplay=true\nExec={command} %f\nIcon=application-x-executable\nTerminal=false\nStartupNotify=false\nMimeType=application/x-janex;\n"
    )
}

/// Compiles a native AppleScript droplet that receives Finder document-open events.
#[cfg(target_os = "macos")]
fn mac_bundle(options: &Options, plan: &mut Plan) -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let source = temporary.path().join("handler.applescript");
    let bundle = temporary.path().join("Janex.app");
    let command = arguments(options, "open")
        .iter()
        .map(|s| shell_quote(s))
        .collect::<Vec<_>>()
        .join(" ");
    let literal = command.replace('\\', "\\\\").replace('"', "\\\"");
    fs::write(
        &source,
        format!(
            "on open droppedFiles\n  repeat with droppedFile in droppedFiles\n    do shell script \"{literal} \" & quoted form of (POSIX path of droppedFile) & \" </dev/null >/dev/null 2>&1 &\"\n  end repeat\nend open\non run\n  display dialog \"Open a .janex file with Janex.\" buttons {{\"OK\"}} default button 1\nend run\n"
        ),
    )?;
    let output = Command::new("/usr/bin/osacompile")
        .arg("-o")
        .arg(&bundle)
        .arg(source)
        .output()?;
    if !output.status.success() {
        return Err(invalid(format!(
            "osacompile: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    // Preserve compiler-generated AppleScript runtime metadata and append document declarations.
    let plist = bundle.join("Contents/Info.plist");
    for (key, value) in [
        ("CFBundleIdentifier", "\"org.glavo.janex\""),
        (
            "CFBundleDocumentTypes",
            r#"[{"CFBundleTypeName":"Janex Application","CFBundleTypeRole":"Shell","LSHandlerRank":"Alternate","LSItemContentTypes":["org.glavo.janex.container"]}]"#,
        ),
        (
            "UTExportedTypeDeclarations",
            r#"[{"UTTypeIdentifier":"org.glavo.janex.container","UTTypeDescription":"Janex Application","UTTypeConformsTo":["public.data"],"UTTypeTagSpecification":{"public.filename-extension":["janex"],"public.mime-type":"application/x-janex"}}]"#,
        ),
    ] {
        let output = Command::new("/usr/bin/plutil")
            .args(["-replace", key, "-json", value])
            .arg(&plist)
            .output()?;
        if !output.status.success() {
            return Err(invalid(format!(
                "plutil: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
    }
    let signed = Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(&bundle)
        .output()?;
    if !signed.status.success() {
        return Err(invalid(format!(
            "codesign: {}",
            String::from_utf8_lossy(&signed.stderr)
        )));
    }
    let destination = if options.system {
        PathBuf::from("/Applications/Janex.app")
    } else {
        env_path("HOME")?.join("Applications/Janex.app")
    };
    collect_bundle(&bundle, &bundle, &destination, &mut plan.assets)?;
    plan.bundle = Some(destination);
    Ok(())
}

/// Collects compiler-generated bundle files while preserving executable permissions.
#[cfg(target_os = "macos")]
fn collect_bundle(
    root: &Path,
    directory: &Path,
    destination: &Path,
    assets: &mut Vec<Asset>,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            collect_bundle(root, &path, destination, assets)?;
        } else if metadata.is_file() {
            let relative = path.strip_prefix(root).expect("bundle member");
            assets.push(Asset {
                path: destination.join(relative),
                export: Path::new("Janex.app").join(relative),
                bytes: fs::read(&path)?,
                executable: metadata.permissions().mode() & 0o111 != 0,
            });
        } else {
            return Err(invalid(
                "unexpected non-file in generated application bundle",
            ));
        }
    }
    Ok(())
}

/// Provides installation guidance for exported assets without changing defaults.
pub(super) fn instructions(options: &Options) -> String {
    let scope = if options.system { "system" } else { "user" };
    let mut text = format!(
        "Janex {scope} integration assets\nExecutable: {}\n\n",
        options.executable.display()
    );
    if cfg!(windows) {
        text.push_str("Import janex.reg with reg.exe import. System registration requires elevation.\nThis adds an Open With handler and does not change the user's default application.\n");
    } else if cfg!(target_os = "macos") {
        text.push_str(if options.system {
            "Install Janex.app in /Applications.\n"
        } else {
            "Install Janex.app in ~/Applications.\n"
        });
        text.push_str("Register the installed bundle using /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f <bundle>.\n");
    } else {
        text.push_str(if options.system {
            "Install share/ under /usr/local/share (or your distribution's shared data prefix).\n"
        } else {
            "Install share/ under $XDG_DATA_HOME, defaulting to ~/.local/share.\n"
        });
        text.push_str("Run update-mime-database <data>/mime and update-desktop-database <data>/applications.\n");
        if options.binfmt {
            text.push_str("Install libexec/janex-binfmt at /usr/libexec/janex-binfmt, mode 0755, and etc/binfmt.d/janex.conf at /etc/binfmt.d/janex.conf.\nActivate the rule through systemd-binfmt or write its contents to mounted /proc/sys/fs/binfmt_misc/register.\n");
        }
    }
    text.push_str("\nExport does not create a Janex registration record. The installer owns these assets and their removal.\n");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_quoting_preserves_reserved_characters() {
        assert_eq!(windows_quote(""), "\"\"");
        assert_eq!(windows_quote("C:\\two words\\"), "\"C:\\two words\\\\\"");
        assert_eq!(windows_quote("x\\\"y"), "\"x\\\\\\\"y\"");
        assert_eq!(shell_quote("a'b $x"), "'a'\\''b $x'");
        assert_eq!(desktop_quote("$`\"\\%"), "\"\\\\$\\\\`\\\\\"\\\\\\\\%%\"");
    }

    #[test]
    fn desktop_and_binfmt_use_distinct_invocations() {
        let options = Options {
            executable: "/opt/Janex tools/janex".into(),
            trust_arguments: vec!["--allow-unsigned".into()],
            system: true,
            binfmt: true,
        };
        assert!(desktop(&options).contains("\"open\" \"--allow-unsigned\" \"--\" %f\n"));
        assert!(shell_handler(&options, "run").contains("'run' '--allow-unsigned' '--' \"$@\""));
        assert!(mime_xml().contains("JANEX\\000\\000\\000"));
    }

    #[cfg(unix)]
    #[test]
    fn shell_adapter_preserves_arguments_without_evaluating_their_contents() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("app ' $` test.janex");
        let options = Options {
            executable: "/usr/bin/printf".into(),
            trust_arguments: vec!["fixed ' argument".into()],
            system: true,
            binfmt: true,
        };
        let script = temp.path().join("handler");
        fs::write(&script, shell_handler(&options, "%s\\n")).unwrap();
        let output = Command::new("/bin/sh")
            .arg(script)
            .arg(&target)
            .args(["", "$(exit 7)", "--help"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "fixed ' argument\n--\n{}\n\n$(exit 7)\n--help\n",
                target.display()
            )
        );
    }
}

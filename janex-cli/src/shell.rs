// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Thin shell functions that evaluate only successful native environment responses.

use janex_host::{
    Error, Result,
    sdk::{Shell, quote_shell},
};
use std::{fs, io::Write};

/// Updates the three managed loader files without reading SDK state or editing shell profiles.
/// Each file is replaced atomically; an error can leave earlier files updated.
pub(super) fn install() -> Result<()> {
    let home = janex_java::runtime::java_path(&janex_platform::janex_home()?);
    let executable = janex_java::runtime::java_path(&std::env::current_exe()?);
    let executable = executable
        .to_str()
        .ok_or_else(|| Error::InvalidInput("Janex executable path is not Unicode".into()))?;
    let home_text = home
        .to_str()
        .ok_or_else(|| Error::InvalidInput("JANEX_HOME is not Unicode".into()))?;
    let directory = home.join("shell");
    let mut scripts = Vec::new();
    for (name, shell, template) in [
        ("init.sh", Shell::Sh, include_str!("shell/load.sh")),
        ("init.fish", Shell::Fish, include_str!("shell/load.fish")),
        (
            "init.ps1",
            Shell::PowerShell,
            include_str!("shell/load.ps1"),
        ),
    ] {
        let path = directory.join(name);
        let path_text = path.to_str().expect("Unicode script path");
        let code = render(
            template,
            &[
                ("@JANEX_EXE@", quote_shell(shell, executable)),
                ("@JANEX_HOME@", quote_shell(shell, home_text)),
            ],
        );
        scripts.push((
            path.clone(),
            code,
            format!(
                "{} {}",
                if matches!(shell, Shell::Fish) {
                    "source"
                } else {
                    "."
                },
                quote_shell(shell, path_text)
            ),
        ));
    }
    fs::create_dir_all(&directory)?;
    for (path, code, _) in &scripts {
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        file.write_all(code.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
    }
    println!("Initialized shell integration in {}", directory.display());
    for ((_, _, command), label) in scripts
        .iter()
        .zip(["sh / Bash / Zsh", "Fish", "PowerShell"])
    {
        println!("{label}:\n  {command}");
    }
    Ok(())
}

/// Substitutes template tokens once, leaving token-like text in literal paths untouched.
fn render(template: &str, values: &[(&str, String)]) -> String {
    let mut output = String::new();
    let mut remaining = template;
    while let Some((offset, token, value)) = values
        .iter()
        .filter_map(|(token, value)| remaining.find(token).map(|offset| (offset, *token, value)))
        .min_by_key(|(offset, _, _)| *offset)
    {
        output.push_str(&remaining[..offset]);
        output.push_str(value);
        remaining = &remaining[offset + token.len()..];
    }
    output.push_str(remaining);
    output
}

/// Generates a wrapper pinned to this executable, avoiding recursive PATH lookup.
pub(super) fn init(shell: Shell) -> Result<String> {
    let executable = janex_java::runtime::java_path(&std::env::current_exe()?);
    let bin = executable
        .parent()
        .and_then(|p| p.to_str())
        .ok_or_else(|| Error::InvalidInput("Janex executable directory is not Unicode".into()))?;
    let executable = executable
        .to_str()
        .ok_or_else(|| Error::InvalidInput("Janex executable path is not Unicode".into()))?;
    let app_home = janex_java::runtime::java_path(&janex_platform::janex_home()?);
    let app_home = app_home
        .to_str()
        .ok_or_else(|| Error::InvalidInput("Janex application directory is not Unicode".into()))?;
    let template = match shell {
        Shell::Sh => include_str!("shell/init.sh"),
        Shell::PowerShell => include_str!("shell/init.ps1"),
        Shell::Fish => include_str!("shell/init.fish"),
    };
    Ok(render(
        template,
        &[
            ("@JANEX_BIN@", quote_shell(shell, bin)),
            ("@JANEX_HOME@", quote_shell(shell, app_home)),
            ("@JANEX_EXE@", quote_shell(shell, executable)),
        ],
    ))
}

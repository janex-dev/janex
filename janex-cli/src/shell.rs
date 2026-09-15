// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Thin shell functions that evaluate only successful native environment responses.

use janex_host::{
    Error, Result,
    sdk::{Shell, quote_shell},
};

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
    let template = match shell {
        Shell::Sh => include_str!("shell/init.sh"),
        Shell::PowerShell => include_str!("shell/init.ps1"),
        Shell::Fish => include_str!("shell/init.fish"),
    };
    Ok(template
        .replace("@JANEX_BIN@", &quote_shell(shell, bin))
        .replace("@JANEX_EXE@", &quote_shell(shell, executable)))
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Thin shell functions that evaluate only successful native environment responses.

use janex_host::{
    Error, Result,
    sdk::{Shell, quote_shell},
};

/// Generates a wrapper pinned to this executable, avoiding recursive PATH lookup.
pub(super) fn activate(shell: Shell) -> Result<String> {
    let executable = janex_java::runtime::java_path(&std::env::current_exe()?);
    let executable = executable
        .to_str()
        .ok_or_else(|| Error::InvalidInput("Janex executable path is not Unicode".into()))?;
    let template = match shell {
        Shell::Sh => include_str!("shell/activate.sh"),
        Shell::PowerShell => include_str!("shell/activate.ps1"),
        Shell::Fish => include_str!("shell/activate.fish"),
    };
    Ok(template.replace("@JANEX_EXE@", &quote_shell(shell, executable)))
}

/// Removes the wrapper after the original environment has been restored.
pub(super) fn deactivate(shell: Shell) -> &'static str {
    match shell {
        Shell::Sh => "unset -f janex\nhash -r 2>/dev/null || :\n",
        Shell::PowerShell => "Remove-Item Function:janex -ErrorAction SilentlyContinue\n",
        Shell::Fish => "functions -e janex\n",
    }
}

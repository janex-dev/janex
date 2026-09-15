# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

& {
    $janexHome = if (Test-Path Env:JANEX_HOME) { $env:JANEX_HOME } else { Join-Path $HOME '.janex' }
    $janexName = if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) { 'janex.exe' } else { 'janex' }
    $janexCode = & (Join-Path (Join-Path $janexHome 'bin') $janexName) init powershell
    if ($LASTEXITCODE -eq 0) {
        . ([scriptblock]::Create(($janexCode -join "`n")))
    }
}

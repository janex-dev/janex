# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

& {
    $janexCode = & @JANEX_EXE@ shell init --shell powershell
    if ($LASTEXITCODE -eq 0) {
        if (-not (Test-Path Env:JANEX_HOME)) { $env:JANEX_HOME = @JANEX_HOME@ }
        . ([scriptblock]::Create(($janexCode -join "`n")))
    }
}

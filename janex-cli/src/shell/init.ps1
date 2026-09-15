# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

if (@JANEX_BIN@ -notin ($env:PATH -split [IO.Path]::PathSeparator)) {
    $env:PATH = @JANEX_BIN@ + $(if (Test-Path Env:PATH) { [IO.Path]::PathSeparator + $env:PATH })
}

function global:janex {
    $janexArguments = @($args)
    if ($janexArguments.Count -gt 0 -and $janexArguments[0] -in @('activate', 'use', 'deactivate') -and
        -not ($janexArguments | Where-Object { $_ -in @('--project', '--help', '-h', '--shell') -or $_ -like '--shell=*' })) {
        $janexCode = & @JANEX_EXE@ @janexArguments --shell powershell
        $janexExitCode = $LASTEXITCODE
        if ($janexExitCode -eq 0) {
            . ([scriptblock]::Create(($janexCode -join "`n")))
        }
        $global:LASTEXITCODE = $janexExitCode
    } else {
        if ($MyInvocation.ExpectingInput) {
            $input | & @JANEX_EXE@ @janexArguments
        } else {
            & @JANEX_EXE@ @janexArguments
        }
        $global:LASTEXITCODE = $LASTEXITCODE
    }
}

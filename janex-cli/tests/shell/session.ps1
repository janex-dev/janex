# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

$ErrorActionPreference = 'Stop'
$originalPath = $env:PATH
$originalJava = $env:JAVA_HOME
& $env:JANEX_TEST_EXE activate powershell | Out-String | Invoke-Expression
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_FIRST) { throw 'Initial project selection failed' }
if ((gradle) -ne 'fixture') { throw 'Direct tool lookup failed' }
janex use gradle@8.14.3
if ($LASTEXITCODE -ne 0 -or $env:GRADLE_HOME -ne $env:JANEX_TEST_SECOND) { throw 'Shell switch failed' }
if (($env:PATH -split ';') -contains "$env:JANEX_TEST_FIRST\bin") { throw 'Old SDK remains on PATH' }
$previousPath = $env:PATH
$previousState = $env:JANEX_SHELL_STATE
janex use gradle@8.14.2 maven@99.0.0 2>$null
if ($LASTEXITCODE -eq 0 -or $env:PATH -ne $previousPath -or $env:JANEX_SHELL_STATE -ne $previousState) { throw 'Failure changed environment' }
janex use --invalid-option 2>$null
if ($LASTEXITCODE -ne 2) { throw 'Native exit code was not preserved' }
janex use
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_FIRST) { throw 'Project reset failed' }
Set-Location $env:JANEX_TEST_OTHER
janex use
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_SECOND) { throw 'Generated home blocked project change' }
janex use --project gradle@8.14.2
if ($LASTEXITCODE -ne 0 -or $env:GRADLE_HOME -ne $env:JANEX_TEST_SECOND) { throw 'Project write changed shell' }
janex use
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_FIRST) { throw 'Updated project was not read' }
& $env:JANEX_TEST_EXE activate powershell | Out-String | Invoke-Expression
janex deactivate
if ($LASTEXITCODE -ne 0) { throw 'Deactivation failed' }
if ($env:PATH -ne $originalPath -or $env:JAVA_HOME -ne $originalJava) { throw 'Original environment was not restored' }
if (Test-Path Env:GRADLE_HOME) { throw 'Absent variable was not restored' }
if (Test-Path Env:JANEX_SHELL_STATE) { throw 'State was not removed' }
if (Test-Path Function:janex) { throw 'Function was not removed' }

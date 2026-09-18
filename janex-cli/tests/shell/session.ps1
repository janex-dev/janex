# Copyright (c) 2026 Glavo
# SPDX-License-Identifier: MPL-2.0

$ErrorActionPreference = 'Stop'
$originalJava = $env:JAVA_HOME
$janexHome = $env:JANEX_HOME
Remove-Item Env:JANEX_HOME
. "$janexHome/shell/init.ps1"
if ($env:JANEX_HOME -ne $janexHome) { throw 'Initialized home was not restored' }
if (Test-Path Env:JANEX_SHELL_STATE) { throw 'Initialization activated SDKs' }
if ($env:JAVA_HOME -ne $originalJava -or (Test-Path Env:GRADLE_HOME)) { throw 'Initialization changed SDK homes' }
$originalPath = $env:PATH
. "$env:JANEX_HOME/shell/init.ps1"
if ($env:PATH -ne $originalPath) { throw 'Initialization duplicated PATH entries' }
janex activate
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_FIRST) { throw 'Initial project selection failed' }
if ((gradle) -ne 'fixture') { throw 'Direct tool lookup failed' }
janex use sdk:gradle@8.14.3
if ($LASTEXITCODE -ne 0 -or $env:GRADLE_HOME -ne $env:JANEX_TEST_SECOND) { throw 'Shell switch failed' }
janex activate
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_SECOND) { throw 'Activation lost manual selection' }
if (($env:PATH -split ';') -contains "$env:JANEX_TEST_FIRST\bin") { throw 'Old SDK remains on PATH' }
$previousPath = $env:PATH
$previousState = $env:JANEX_SHELL_STATE
janex use sdk:gradle@8.14.2 sdk:maven@99.0.0 2>$null
if ($LASTEXITCODE -eq 0 -or $env:PATH -ne $previousPath -or $env:JANEX_SHELL_STATE -ne $previousState) { throw 'Failure changed environment' }
janex use --invalid-option 2>$null
if ($LASTEXITCODE -ne 2) { throw 'Native exit code was not preserved' }
janex use
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_FIRST) { throw 'Project reset failed' }
Set-Location $env:JANEX_TEST_OTHER
janex use
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_SECOND) { throw 'Generated home blocked project change' }
janex use --project sdk:gradle@8.14.2
if ($LASTEXITCODE -ne 0 -or $env:GRADLE_HOME -ne $env:JANEX_TEST_SECOND) { throw 'Project write changed shell' }
janex use
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_FIRST) { throw 'Updated project was not read' }
janex activate
janex deactivate
if ($LASTEXITCODE -ne 0) { throw 'Deactivation failed' }
if ($env:PATH -ne $originalPath -or $env:JAVA_HOME -ne $originalJava) { throw 'Original environment was not restored' }
if (Test-Path Env:GRADLE_HOME) { throw 'Absent variable was not restored' }
if (Test-Path Env:JANEX_SHELL_STATE) { throw 'State was not removed' }
if (-not (Test-Path Function:janex)) { throw 'Function was removed' }
janex --version
if ($LASTEXITCODE -ne 0) { throw 'Command forwarding failed' }
janex activate
if ($env:GRADLE_HOME -ne $env:JANEX_TEST_FIRST) { throw 'Reactivation failed' }
janex deactivate
if ($env:PATH -ne $originalPath) { throw 'Reactivation lost baseline' }

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

// The plugin's standalone build has its root one directory below the Cargo workspace.
val manifestPath = if (rootDir.resolve("Cargo.toml").isFile) "Cargo.toml" else "../Cargo.toml"
val manifest = providers.fileContents(rootProject.layout.projectDirectory.file(manifestPath)).asText.get()
val workspacePackage = Regex("""(?ms)^\[workspace\.package]\s*\n(.*?)(?=^\[|\z)""")
    .find(manifest)?.groupValues?.get(1)
    ?: error("Cargo.toml must contain [workspace.package]")

version = Regex("""(?m)^version\s*=\s*"([^"]+)"\s*(?:#.*)?$""")
    .find(workspacePackage)?.groupValues?.get(1)
    ?: error("Cargo.toml must define workspace.package.version as a quoted string")

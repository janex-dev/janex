// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

plugins {
    base
}

group = "org.glavo.janex"
version = "0.1.0"

// Cargo owns incremental compilation; these Exec tasks intentionally declare no outputs.
val cargoBuild = tasks.register<Exec>("cargoBuild") {
    group = "build"
    description = "Builds the Rust workspace after generating the bootstrap JAR."
    dependsOn(":janex-bootstrap:jar")
    workingDir(layout.projectDirectory)
    commandLine("cargo", "build", "--workspace", "--locked")
}

val cargoBuildRelease = tasks.register<Exec>("cargoBuildRelease") {
    group = "build"
    description = "Builds the Rust workspace with release optimizations."
    dependsOn(":janex-bootstrap:jar")
    workingDir(layout.projectDirectory)
    commandLine("cargo", "build", "--workspace", "--release", "--locked")
    mustRunAfter(cargoBuild)
}

val cargoFmtCheck = tasks.register<Exec>("cargoFmtCheck") {
    group = "verification"
    description = "Checks Rust formatting without modifying sources."
    workingDir(layout.projectDirectory)
    commandLine("cargo", "fmt", "--all", "--check")
}

val cargoClippy = tasks.register<Exec>("cargoClippy") {
    group = "verification"
    description = "Checks the Rust workspace and all targets with warnings denied."
    dependsOn(":janex-bootstrap:jar")
    workingDir(layout.projectDirectory)
    commandLine("cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings")
    mustRunAfter(cargoFmtCheck, cargoBuild, cargoBuildRelease)
}

val cargoTest = tasks.register<Exec>("cargoTest") {
    group = "verification"
    description = "Runs Rust tests, documentation examples, and Java integration tests."
    dependsOn(":janex-bootstrap:jar")
    workingDir(layout.projectDirectory)
    commandLine("cargo", "test", "--workspace", "--locked")
    mustRunAfter(cargoFmtCheck, cargoClippy, cargoBuild, cargoBuildRelease)
}

val cargoClean = tasks.register<Exec>("cargoClean") {
    group = "build"
    description = "Removes Cargo build outputs using Cargo's target-directory configuration."
    workingDir(layout.projectDirectory)
    commandLine("cargo", "clean")
}

tasks.assemble {
    dependsOn(":janex-bootstrap:assemble", cargoBuild)
    dependsOn(":janex-gradle-plugin:assemble")
}

tasks.register("assembleRelease") {
    group = "build"
    description = "Assembles Java artifacts and release Rust binaries."
    dependsOn(":janex-bootstrap:assemble", cargoBuildRelease)
}

tasks.register<Exec>("assembleWindowsX86Launcher") {
    group = "build"
    description = "Builds the release Windows x86 launcher for native or emulated execution."
    dependsOn(":janex-bootstrap:jar")
    workingDir(layout.projectDirectory)
    commandLine("cargo", "build", "--package", "janex-launcher", "--target", "i686-pc-windows-msvc", "--release", "--locked")
    mustRunAfter(cargoBuild, cargoBuildRelease, cargoClippy, cargoTest)
}

tasks.register<Exec>("checkWindowsX86Launcher") {
    group = "verification"
    description = "Tests the 32-bit Windows launcher with the available Java runtimes."
    dependsOn(":janex-bootstrap:jar")
    workingDir(layout.projectDirectory)
    commandLine("cargo", "test", "--package", "janex-launcher", "--package", "janex-platform", "--target", "i686-pc-windows-msvc", "--locked", "--", "--test-threads=1")
    mustRunAfter(cargoBuild, cargoBuildRelease, cargoClippy, cargoTest)
}

tasks.check {
    dependsOn(":janex-gradle-plugin:check")
    dependsOn(":janex-bootstrap:check")
    dependsOn(":janex-reader:check")
    dependsOn(cargoFmtCheck, cargoClippy, cargoTest)
}

tasks.clean {
    dependsOn(":janex-gradle-plugin:clean")
    dependsOn(":janex-bootstrap:clean", ":janex-reader:clean", cargoClean)
}

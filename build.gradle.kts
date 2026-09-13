// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

plugins {
    base
}

group = "org.janex"
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
}

tasks.register("assembleRelease") {
    group = "build"
    description = "Assembles Java artifacts and release Rust binaries."
    dependsOn(":janex-bootstrap:assemble", cargoBuildRelease)
}

tasks.check {
    dependsOn(":janex-bootstrap:check")
    dependsOn(":janex-reader:check")
    dependsOn(cargoFmtCheck, cargoClippy, cargoTest)
}

tasks.clean {
    dependsOn(":janex-bootstrap:clean", ":janex-reader:clean", cargoClean)
}

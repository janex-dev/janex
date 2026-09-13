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

val artifactTarget = providers.gradleProperty("janexTarget").getOrElse("")
val artifactPlatform = mapOf(
    "x86_64-unknown-linux-musl" to "linux-x86_64",
    "aarch64-unknown-linux-musl" to "linux-arm64",
    "x86_64-unknown-freebsd" to "freebsd-x86_64",
    "aarch64-unknown-freebsd" to "freebsd-arm64",
    "i686-pc-windows-msvc" to "windows-x86",
    "x86_64-pc-windows-msvc" to "windows-x86_64",
    "aarch64-pc-windows-msvc" to "windows-arm64",
    "x86_64-apple-darwin" to "macos-x86_64",
    "aarch64-apple-darwin" to "macos-arm64"
)[artifactTarget].orEmpty()
val artifactDirectory = providers.environmentVariable("CARGO_TARGET_DIR")
    .map { file(it).resolve("$artifactTarget/release") }
    .getOrElse(layout.projectDirectory.dir("target/$artifactTarget/release").asFile)
val artifactNames = if (artifactTarget.endsWith("-apple-darwin")) listOf("janex")
    else if (artifactTarget.endsWith("-windows-msvc")) listOf("janex.exe", "janex-launcher.exe")
    else listOf("janex", "janex-launcher")

val assembleArtifacts = tasks.register<Exec>("assembleArtifacts") {
    group = "build"
    description = "Builds distribution binaries for the selected Linux, FreeBSD, Windows, or macOS target."
    dependsOn(":janex-bootstrap:jar")
    val target = artifactTarget
    doFirst {
        require(target in setOf(
            "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl",
            "x86_64-unknown-freebsd", "aarch64-unknown-freebsd",
            "i686-pc-windows-msvc", "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc",
            "x86_64-apple-darwin", "aarch64-apple-darwin"
        )) {
            "Set -PjanexTarget to a supported Linux musl, FreeBSD, Windows MSVC, or macOS target: $target"
        }
    }
    workingDir(layout.projectDirectory)
    val command = mutableListOf(
        "cargo", if (target.endsWith("-musl") || target.endsWith("-freebsd")) "zigbuild" else "build",
        "--release", "--locked", "--target", target, "--package", "janex-cli", "--bins"
    )
    if (!target.endsWith("-apple-darwin")) {
        command.addAll(listOf("--package", "janex-launcher"))
    }
    if (target.endsWith("-musl")) {
        // Keep allocation symbols available for artifact verification.
        command.addAll(listOf("--config", "profile.release.strip='none'"))
    }
    if (target.endsWith("-windows-msvc")) {
        command.addAll(listOf("--config", "target.$target.rustflags=['-C','target-feature=+crt-static']"))
    }
    commandLine(command)
    mustRunAfter(cargoBuild, cargoBuildRelease, cargoClippy, cargoTest)
}

val stripArtifacts = tasks.register<Exec>("stripArtifacts") {
    description = "Removes Linux debug information while retaining allocation symbols for verification."
    dependsOn(assembleArtifacts)
    onlyIf { artifactTarget.endsWith("-musl") }
    val strip = if (artifactTarget.startsWith("aarch64-")) "aarch64-linux-gnu-strip" else "strip"
    commandLine(listOf(strip, "--strip-debug") + artifactNames.map { artifactDirectory.resolve(it).path })
}

val checkArtifacts = tasks.register<JavaExec>("checkArtifacts") {
    group = "verification"
    description = "Checks distribution binaries and supported Java launch modes on the current host."
    dependsOn(stripArtifacts, ":janex-bootstrap:testFixturesClasses")
    classpath(provider {
        project(":janex-bootstrap").extensions.getByType<SourceSetContainer>()
            .named("testFixtures").get().runtimeClasspath
    })
    mainClass = "org.glavo.janex.testing.ArtifactCheck"
    args(artifactTarget, artifactDirectory.absolutePath)
}

val artifactArchive = if (artifactTarget.endsWith("-windows-msvc")) {
    tasks.register<Zip>("packageArtifacts")
} else {
    tasks.register<Tar>("archiveArtifacts") {
        compression = Compression.NONE
        archiveExtension = "tar"
    }
}
artifactArchive.configure {
    group = "distribution"
    description = "Verifies and packages distribution binaries."
    dependsOn(checkArtifacts)
    archiveBaseName = "janex-$artifactPlatform"
    archiveVersion = ""
    destinationDirectory = layout.buildDirectory.dir("distributions")
    from(artifactDirectory) {
        include(artifactNames)
        filePermissions { unix("rwxr-xr-x") }
    }
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
}

if (!artifactTarget.endsWith("-windows-msvc")) {
    tasks.register<Exec>("packageArtifacts") {
        group = "distribution"
        description = "Verifies and packages distribution binaries as a TAR.XZ archive."
        dependsOn(artifactArchive)
        val archive = artifactArchive.flatMap { it.archiveFile }
        inputs.file(archive)
        outputs.file(layout.buildDirectory.file("distributions/janex-$artifactPlatform.tar.xz"))
        environment("XZ_DEFAULTS", "")
        environment("XZ_OPT", "")
        doFirst {
            commandLine("xz", "--threads=1", "-6", "--keep", "--force", archive.get().asFile.absolutePath)
        }
    }
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
    dependsOn(":janex-writer:check")
    dependsOn(cargoFmtCheck, cargoClippy, cargoTest)
}

tasks.clean {
    dependsOn(":janex-gradle-plugin:clean")
    dependsOn(":janex-bootstrap:clean", ":janex-reader:clean", ":janex-writer:clean", cargoClean)
}

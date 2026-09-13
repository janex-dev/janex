// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

plugins {
    `java-library`
    `java-test-fixtures`
}

group = rootProject.group
version = rootProject.version

java {
    toolchain.languageVersion = JavaLanguageVersion.of(25)
}

tasks.withType<JavaCompile>().configureEach {
    options.release = 8
    options.encoding = "UTF-8"
    options.isDebug = false
    options.compilerArgs.add("-Xlint:-options")
}

tasks.withType<Jar>().configureEach {
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
}

tasks.register<JavaExec>("checkReader") {
    group = "verification"
    description = "Checks binary, CBOR, and Java version boundaries."
    classpath = sourceSets.testFixtures.get().runtimeClasspath
    mainClass = "org.janex.format.ReaderTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
}

tasks.check {
    dependsOn("checkReader", "checkChecksums")
}

tasks.register<JavaExec>("checkChecksums") {
    group = "verification"
    description = "Checks checksum encodings, known answers, and streaming contracts."
    classpath = sourceSets.testFixtures.get().runtimeClasspath
    mainClass = "org.janex.format.ChecksumTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
}

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
    mainClass = "org.glavo.janex.reader.ReaderTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
}

tasks.check {
    dependsOn("checkReader", "checkChecksums", "checkZstandard")
}

tasks.register<JavaExec>("checkChecksums") {
    group = "verification"
    description = "Checks checksum encodings, known answers, and streaming contracts."
    classpath = sourceSets.testFixtures.get().runtimeClasspath
    mainClass = "org.glavo.janex.reader.ChecksumTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
}

tasks.register<JavaExec>("checkZstandard") {
    group = "verification"
    description = "Checks Zstandard format boundaries and array contracts."
    classpath = sourceSets.testFixtures.get().runtimeClasspath
    mainClass = "org.glavo.janex.reader.internal.codec.ZstandardTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
}

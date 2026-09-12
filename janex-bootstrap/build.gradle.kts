// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

import org.apache.tools.ant.filters.FixCrLfFilter
import org.gradle.api.tasks.bundling.ZipEntryCompression

plugins {
    `java-library`
    `java-test-fixtures`
}

group = rootProject.group
version = rootProject.version

java {
    toolchain.languageVersion = JavaLanguageVersion.of(25)
}

sourceSets.main {
    java.srcDir("src/vendor/java")
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
    entryCompression = ZipEntryCompression.STORED
    filePermissions { unix("rw-r--r--") }
    dirPermissions { unix("rwxr-xr-x") }
}

tasks.jar {
    archiveFileName = "janex-bootstrap.jar"
    from(files("LICENSE-APACHE-2.0", "NOTICE")) {
        into("META-INF")
        filteringCharset = "UTF-8"
        filter<FixCrLfFilter>("eol" to FixCrLfFilter.CrLf.newInstance("lf"))
    }
    from(rootProject.file("LICENSE")) {
        into("META-INF")
        rename { "LICENSE-MPL-2.0" }
        filteringCharset = "UTF-8"
        filter<FixCrLfFilter>("eol" to FixCrLfFilter.CrLf.newInstance("lf"))
    }
}

// Cargo embeds this recorded artifact without requiring a JDK during Rust builds.
val embeddedBootstrap = layout.projectDirectory.file("bootstrap.jar")
val generatedBootstrap = tasks.jar.flatMap { it.archiveFile }

val verifyEmbeddedBootstrap = tasks.register("verifyEmbeddedBootstrap") {
    group = "verification"
    description = "Checks that the embedded bootstrap JAR matches the Java sources."
    inputs.file(generatedBootstrap)
    inputs.file(embeddedBootstrap)
    mustRunAfter("updateEmbeddedBootstrap")
    val generated = generatedBootstrap
    val embedded = embeddedBootstrap
    doLast {
        check(generated.get().asFile.readBytes().contentEquals(embedded.asFile.readBytes())) {
            "Embedded bootstrap.jar differs; run :janex-bootstrap:updateEmbeddedBootstrap."
        }
    }
}

tasks.register("updateEmbeddedBootstrap") {
    group = "build"
    description = "Updates the bootstrap JAR embedded by the Rust launcher."
    inputs.file(generatedBootstrap)
    outputs.file(embeddedBootstrap)
    val generated = generatedBootstrap
    val embedded = embeddedBootstrap
    doLast {
        generated.get().asFile.copyTo(embedded.asFile, overwrite = true)
    }
}

tasks.check {
    dependsOn(verifyEmbeddedBootstrap, tasks.testFixturesClasses)
}

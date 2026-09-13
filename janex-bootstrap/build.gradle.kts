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

dependencies {
    implementation(project(":janex-reader"))
    testFixturesImplementation(project(":janex-reader"))
}

java {
    toolchain.languageVersion = JavaLanguageVersion.of(25)
}

tasks.withType<JavaCompile>().configureEach {
    options.release = 8
    options.encoding = "UTF-8"
    options.isDebug = false
    options.compilerArgs.add("-Xlint:-options")
}

val java9 = sourceSets.create("java9") {
    compileClasspath += sourceSets.main.get().output
}
tasks.named<JavaCompile>(java9.compileJavaTaskName) { options.release = 9 }

tasks.withType<Jar>().configureEach {
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
    entryCompression = ZipEntryCompression.STORED
    filePermissions { unix("rw-r--r--") }
    dirPermissions { unix("rwxr-xr-x") }
}

tasks.jar {
    dependsOn(configurations.runtimeClasspath)
    archiveFileName = "janex-bootstrap.jar"
    manifest.attributes(
        "Multi-Release" to "true",
        "Main-Class" to "org.glavo.janex.bootstrap.Standalone"
    )
    from(java9.output) { into("META-INF/versions/9") }
    from(configurations.runtimeClasspath.map { files -> files.map { zipTree(it) } }) {
        exclude("META-INF/MANIFEST.MF")
    }
    from(layout.projectDirectory.file("../LICENSE")) {
        into("META-INF")
        rename { "LICENSE-MPL-2.0" }
        filteringCharset = "UTF-8"
        filter<FixCrLfFilter>("eol" to FixCrLfFilter.CrLf.newInstance("lf"))
    }
}

tasks.check {
    dependsOn(tasks.testFixturesClasses, "checkDependencies")
}

tasks.register<JavaExec>("checkDependencies") {
    group = "verification"
    description = "Checks canonical dependency addresses and Maven repository mapping."
    classpath = sourceSets.testFixtures.get().runtimeClasspath
    mainClass = "org.glavo.janex.bootstrap.dependency.DependenciesTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
}

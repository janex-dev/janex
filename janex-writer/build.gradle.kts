// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

plugins {
    `java-library`
    `java-test-fixtures`
    `maven-publish`
}

group = "org.glavo.janex"
version = rootProject.version

dependencies {
    api(project(":janex-reader"))
    implementation(libs.zstd.jni)
    implementation(libs.bouncycastle.pkix)
    implementation(libs.bouncycastle.pgp)
    implementation(libs.asm.commons)
}

java {
    toolchain.languageVersion = JavaLanguageVersion.of(25)
    withSourcesJar()
    withJavadocJar()
}

tasks.withType<JavaCompile>().configureEach {
    options.release = 17
    options.encoding = "UTF-8"
}

tasks.withType<Jar>().configureEach {
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
}

tasks.processResources {
    from(project(":janex-bootstrap").tasks.named("jar")) {
        into("org/glavo/janex/writer")
    }
}

publishing {
    publications.create<MavenPublication>("library") { from(components["java"]) }
    repositories.maven {
        name = "local"
        url = (rootProject.findProject(":janex-gradle-plugin") ?: rootProject)
            .layout.buildDirectory.dir("repository").get().asFile.toURI()
    }
    providers.gradleProperty("janexPublishUrl").orNull?.let { repositoryUrl ->
        repositories.maven {
            name = "cnb"
            url = uri(repositoryUrl)
            credentials(PasswordCredentials::class)
        }
    }
}

tasks.register<JavaExec>("checkWriter") {
    group = "verification"
    description = "Checks Java packaging, integrity, resource layers, and failure boundaries."
    classpath = sourceSets.testFixtures.get().runtimeClasspath
    mainClass = "org.glavo.janex.writer.WriterTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
}

tasks.register<JavaExec>("checkSignatures") {
    group = "verification"
    description = "Checks Java publisher signing with independent public key fixtures."
    classpath = sourceSets.testFixtures.get().runtimeClasspath
    mainClass = "org.glavo.janex.writer.SignatureTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
    systemProperty("janex.test.fixtures", layout.projectDirectory.dir("../janex-signature/tests/fixtures").asFile.absolutePath)
}

tasks.register<JavaExec>("checkNativePrefix") {
    group = "verification"
    description = "Checks native launcher prefixes with a bounded Java heap."
    classpath = sourceSets.testFixtures.get().runtimeClasspath
    mainClass = "org.glavo.janex.writer.NativePrefixTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
    maxHeapSize = "128m"
}

tasks.register<JavaExec>("checkMinimization") {
    group = "verification"
    description = "Checks dependency minimization, keep rules, and resource filtering."
    classpath = sourceSets.testFixtures.get().runtimeClasspath
    mainClass = "org.glavo.janex.writer.MinimizationTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
}

tasks.check { dependsOn("checkWriter", "checkSignatures", "checkNativePrefix", "checkMinimization") }

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

plugins {
    `java-gradle-plugin`
    `maven-publish`
}

group = "org.glavo.janex"
version = "0.1.0"

val functionalTest = sourceSets.create("functionalTest")

java {
    toolchain.languageVersion = JavaLanguageVersion.of(25)
    withSourcesJar()
    withJavadocJar()
}

tasks.withType<JavaCompile>().configureEach {
    options.release = 17
    options.encoding = "UTF-8"
}

tasks.withType<Javadoc>().configureEach {
    options.encoding = "UTF-8"
}

tasks.withType<Jar>().configureEach {
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
}

gradlePlugin {
    testSourceSets(functionalTest)
    plugins {
        create("janex") {
            id = "org.glavo.janex"
            implementationClass = "org.glavo.janex.gradle.JanexPlugin"
            displayName = "Janex packaging"
            description = "Packages Java applications and runtime dependencies as Janex files."
        }
    }
}

publishing {
    repositories {
        maven {
            name = "local"
            url = layout.buildDirectory.dir("repository").get().asFile.toURI()
        }
        providers.gradleProperty("janexPublishUrl").orNull?.let { repositoryUrl ->
            maven {
                name = "cnb"
                url = uri(repositoryUrl)
                credentials(PasswordCredentials::class)
            }
        }
    }
}

val windows = System.getProperty("os.name").startsWith("Windows")
val janexExecutable = providers.gradleProperty("janexTestExecutable").orElse(
    layout.projectDirectory.file("../target/debug/janex${if (windows) ".exe" else ""}").asFile.absolutePath
)

tasks.register<JavaExec>("checkPlugin") {
    group = "verification"
    description = "Tests the Gradle plugin with TestKit and the native Janex CLI."
    dependsOn(tasks.named(functionalTest.classesTaskName), tasks.pluginUnderTestMetadata)
    dependsOn("publishAllPublicationsToLocalRepository")
    if (project != rootProject) {
        dependsOn(":cargoBuild")
    }
    classpath = functionalTest.runtimeClasspath
    mainClass = "org.glavo.janex.gradle.JanexPluginTest"
    javaLauncher = javaToolchains.launcherFor(java.toolchain)
    systemProperty("janex.test.executable", janexExecutable.get())
    systemProperty("janex.test.directory", layout.buildDirectory.dir("functional-tests").get().asFile.absolutePath)
    systemProperty("janex.test.repository", layout.buildDirectory.dir("repository").get().asFile.toURI().toString())
}

tasks.check {
    dependsOn("checkPlugin")
}

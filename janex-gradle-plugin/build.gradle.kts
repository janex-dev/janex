// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

plugins {
    id("com.gradle.plugin-publish") version "2.1.1"
    id("com.gradleup.shadow") version "9.6.1"
}

group = "org.glavo.janex"
if (project == rootProject) {
    apply(from = "../gradle/version.gradle.kts")
} else {
    version = rootProject.version
}

val functionalTest = sourceSets.create("functionalTest")

dependencies {
    implementation(project(":janex-writer"))
    implementation(project(":janex-reader"))
    shadow(libs.aircompressor)
    shadow(libs.bouncycastle.pkix)
    shadow(libs.bouncycastle.pgp)
    add(functionalTest.implementationConfigurationName, testFixtures(project(":janex-writer")))
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

tasks.withType<Javadoc>().configureEach {
    options.encoding = "UTF-8"
}

tasks.withType<Jar>().configureEach {
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
}

tasks.jar {
    archiveClassifier = "thin"
}

tasks.shadowJar {
    archiveClassifier = ""
    dependencies {
        include(project(":janex-reader"))
        include(project(":janex-writer"))
    }
}

tasks.pluginUnderTestMetadata {
    pluginClasspath.setFrom(tasks.shadowJar, configurations.shadow)
}

gradlePlugin {
    website = "https://github.com/janex-dev/janex"
    vcsUrl = "https://github.com/janex-dev/janex.git"
    testSourceSets(functionalTest)
    plugins {
        create("janex") {
            id = "org.glavo.janex"
            implementationClass = "org.glavo.janex.gradle.JanexPlugin"
            displayName = "Janex packaging"
            description = "Packages Java applications directly using the portable Java writer."
            tags = listOf("java", "packaging", "janex")
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
    systemProperty("janex.test.version", project.version.toString())
    systemProperty("janex.test.pluginJar", tasks.shadowJar.get().archiveFile.get().asFile.absolutePath)
    systemProperty("janex.test.publicationDirectory", layout.buildDirectory.dir("publications/pluginMaven").get().asFile.absolutePath)
    systemProperty("janex.test.fixtures", layout.projectDirectory.dir("../janex-signature/tests/fixtures").asFile.absolutePath)
    environment("JANEX_TEST_KEY_PASSWORD", "public-fixture-password")
}

tasks.check {
    dependsOn("checkPlugin")
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

plugins {
    application
    id("org.janex")
}

java {
    toolchain.languageVersion = JavaLanguageVersion.of(25)
}

tasks.withType<JavaCompile>().configureEach {
    options.release = 8
    options.encoding = "UTF-8"
}

application {
    mainClass = "org.janex.example.Hello"
}

janex {
    arguments.add("Hello from Janex!")
}

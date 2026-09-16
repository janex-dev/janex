// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

rootProject.name = "janex-gradle-plugin"

dependencyResolutionManagement {
    repositories { mavenCentral() }
    versionCatalogs {
        create("libs") { from(files("../gradle/libs.versions.toml")) }
    }
}
include("janex-reader", "janex-bootstrap", "janex-writer")
for (name in listOf("janex-reader", "janex-bootstrap", "janex-writer")) {
    project(":$name").projectDir = file("../$name")
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

rootProject.name = "janex"

dependencyResolutionManagement {
    repositories { mavenCentral() }
}

include("janex-bootstrap")
include("janex-reader")
include("janex-writer")
include("janex-gradle-plugin")

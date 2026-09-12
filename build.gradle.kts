// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

plugins {
    base
}

group = "org.janex"
version = "0.1.0"

tasks.assemble {
    dependsOn(":janex-bootstrap:assemble")
}

tasks.check {
    dependsOn(":janex-bootstrap:check")
    dependsOn(":janex-reader:check")
}

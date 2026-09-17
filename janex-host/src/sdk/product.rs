// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Product identities independent of catalog providers and installation platforms.

use crate::{Result, error::invalid};
use serde::Serialize;

/// A supported product and its provider-specific editions.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Product {
    /// Stable publisher/product identifier.
    pub id: &'static str,
    /// Tool family used for environment selection.
    pub family: &'static str,
    /// Supported named variants, with the default first; their meaning belongs to this product.
    pub variants: &'static [&'static str],
    /// Whether installations are specific to an OS, architecture and ABI.
    pub platform_specific: bool,
    /// Foojay distribution identifier, absent for dedicated providers.
    #[serde(skip)]
    pub(super) disco: Option<&'static str>,
}

/// Products currently supported by the built-in providers.
pub const PRODUCTS: &[Product] = &[
    Product {
        id: "bellsoft/liberica-jdk",
        family: "java",
        platform_specific: true,
        variants: &["standard", "full", "lite"],
        disco: None,
    },
    Product {
        id: "bellsoft/liberica-jre",
        family: "java",
        platform_specific: true,
        variants: &["standard", "full", "lite"],
        disco: None,
    },
    Product {
        id: "bellsoft/liberica-nik",
        family: "java",
        platform_specific: true,
        variants: &["standard", "full"],
        disco: None,
    },
    Product {
        id: "adoptium/temurin-jdk",
        family: "java",
        platform_specific: true,
        variants: &["standard"],
        disco: Some("temurin"),
    },
    Product {
        id: "adoptium/temurin-jre",
        family: "java",
        platform_specific: true,
        variants: &["standard"],
        disco: Some("temurin"),
    },
    Product {
        id: "azul/zulu-jdk",
        family: "java",
        platform_specific: true,
        variants: &["standard", "fx"],
        disco: Some("zulu"),
    },
    Product {
        id: "azul/zulu-jre",
        family: "java",
        platform_specific: true,
        variants: &["standard", "fx"],
        disco: Some("zulu"),
    },
    Product {
        id: "amazon/corretto-jdk",
        family: "java",
        platform_specific: true,
        variants: &["standard"],
        disco: Some("corretto"),
    },
    Product {
        id: "microsoft/openjdk",
        family: "java",
        platform_specific: true,
        variants: &["standard"],
        disco: Some("microsoft"),
    },
    Product {
        id: "oracle/jdk",
        family: "java",
        platform_specific: true,
        variants: &["standard"],
        disco: Some("oracle"),
    },
    Product {
        id: "ibm/semeru-jdk",
        family: "java",
        platform_specific: true,
        variants: &["standard"],
        disco: Some("semeru"),
    },
    Product {
        id: "sap/sapmachine-jdk",
        family: "java",
        platform_specific: true,
        variants: &["standard"],
        disco: Some("sap_machine"),
    },
    Product {
        id: "alibaba/dragonwell-jdk",
        family: "java",
        platform_specific: true,
        variants: &["standard"],
        disco: Some("dragonwell"),
    },
    Product {
        id: "graalvm/community-jdk",
        family: "java",
        platform_specific: true,
        variants: &["standard"],
        disco: Some("graalvm_community"),
    },
    Product {
        id: "gradle/gradle",
        family: "gradle",
        platform_specific: false,
        variants: &["bin", "all"],
        disco: None,
    },
    Product {
        id: "apache/maven",
        family: "maven",
        platform_specific: false,
        variants: &["standard"],
        disco: None,
    },
];

/// Resolves a product ID; tool names are accepted only as unambiguous product aliases.
pub(super) fn product(id: &str) -> Result<&'static Product> {
    let id = match id {
        "gradle" => "gradle/gradle",
        "maven" => "apache/maven",
        other => other,
    };
    PRODUCTS.iter().find(|p| p.id == id).ok_or_else(|| {
        invalid(format!(
            "unknown SDK product: {id}; use janex available java to list Java products"
        ))
    })
}

impl Product {
    /// Returns the Java archive package kind for this product.
    pub(super) fn kind(&self) -> &'static str {
        if self.id.ends_with("-jre") {
            "jre"
        } else {
            "jdk"
        }
    }

    /// Whether product versions describe Native Image Kit rather than its bundled JDK.
    pub(super) fn is_nik(&self) -> bool {
        self.id == "bellsoft/liberica-nik"
    }
}

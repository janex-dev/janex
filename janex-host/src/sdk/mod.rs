// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Native, persistent SDK installation and version selection.

mod archive;
mod bellsoft;
mod catalog;
mod platform;
mod product;
mod request;
mod selection;
mod shell;
mod state;
mod tools;

pub use catalog::{AvailableSdk, CatalogOptions};
pub use platform::SdkPlatform;
pub use product::{PRODUCTS, Product};
pub use request::SdkRequest;
pub(crate) use selection::application_runtimes;
pub use selection::project_java;
pub use selection::{SdkExecution, Shell};
pub use shell::quote as quote_shell;
pub use state::{Installation, SdkManager, SdkStatus, Selection};

use crate::{Result, error::invalid};
/// Tests stable Java-family product versions; one component selects a release series.
fn java_matches(wanted: &str, version: &str) -> bool {
    if wanted == "latest" {
        return numeric_version(version).is_ok();
    }
    let Ok((wanted, build)) = numeric_version(wanted) else {
        return false;
    };
    let Ok((actual, actual_build)) = numeric_version(version) else {
        return false;
    };
    if wanted.len() == 1 && build.is_none() {
        return actual.first() == wanted.first();
    }
    let size = wanted.len().max(actual.len());
    (0..size).all(|i| wanted.get(i).unwrap_or(&0) == actual.get(i).unwrap_or(&0))
        && build.is_none_or(|b| actual_build == Some(b))
}

/// Parses normalized GA numeric releases with an optional build number.
pub(crate) fn numeric_version(text: &str) -> Result<(Vec<u32>, Option<u32>)> {
    if text.is_empty() || text.len() > 80 {
        return Err(invalid("invalid Java version"));
    }
    let (release, build) = text
        .split_once('+')
        .map_or((text, None), |(a, b)| (a, Some(b)));
    let number = |s: &str| -> Result<u32> {
        if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
            return Err(invalid("expected a numeric GA Java version"));
        }
        s.parse()
            .map_err(|_| invalid("Java version component exceeds u32"))
    };
    let parts: Vec<_> = release.split('.').map(number).collect::<Result<_>>()?;
    if parts.len() > 5 || parts[0] < 8 {
        return Err(invalid("expected a Java 8-or-later numeric version"));
    }
    Ok((parts, build.map(number).transpose()?))
}

/// Orders GA versions numerically, including build numbers.
pub(crate) fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| {
        let (mut parts, build) = numeric_version(s).expect("validated Java version");
        parts.resize(5, 0);
        (parts, build.unwrap_or(0))
    };
    key(a).cmp(&key(b))
}

/// Returns the Java executable name on this operating system.
pub(crate) fn java_name() -> &'static str {
    if cfg!(windows) { "java.exe" } else { "java" }
}

/// Encodes a digest as lowercase hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_requirements_and_build_order() {
        let mut request = SdkRequest::parse("sdk:bellsoft/liberica-jdk@21").unwrap();
        assert!(request.matches("21.0.8+12"));
        assert!(!request.matches("22+1"));
        request.version = "21.0.8".into();
        assert!(request.matches("21.0.8+12"));
        assert!(!request.matches("21.0.8.1+1"));
        assert!(!request.matches("21.0.9+1"));
        request.version = "21.0.8+12".into();
        assert!(!request.matches("21.0.8+13"));
        assert!(version_order("21.0.10+1", "21.0.9+30").is_gt());
        assert!(version_order("21.0.8+12", "21.0.8+9").is_gt());
        assert!(
            SdkRequest::parse("sdk:bellsoft/liberica-jdk@latest")
                .unwrap()
                .matches("25.0.1+8")
        );
        for target in [
            "sdk:bellsoft/liberica-jdk@../21",
            "sdk:bellsoft/liberica-jdk@21-ea",
            "sdk:bellsoft/liberica-jdk@21+",
            "java:../bad@21",
            "example@21",
        ] {
            assert!(SdkRequest::parse(target).is_err(), "{target}");
        }
    }
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Conversions from container policy to independent capability inputs.

use crate::Result;
use janex_format::{
    binary::Limits,
    condition::{Context, RuntimeContext},
    resource::ResourceRoot,
    version::JavaVersion,
};
use janex_java::runtime::JavaRuntime;
use std::borrow::Cow;

/// Derives the Java path filename from a root name, borrowing names that already end in `.jar`.
pub(crate) fn jar_name(root: &ResourceRoot) -> Result<Cow<'_, str>> {
    let name = root.name()?.unwrap_or("resources.jar");
    Ok(if name.ends_with(".jar") {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(format!("{name}.jar"))
    })
}

/// Converts parsing bounds without transferring ownership of format policy.
pub(crate) fn java_limits(limits: Limits) -> janex_java::Limits {
    janex_java::Limits {
        max_bytes: limits.max_bytes,
        max_elements: limits.max_elements,
    }
}

/// Converts parsing bounds for signatures contained in a Janex file.
pub(crate) fn signature_limits(limits: Limits) -> janex_signature::Limits {
    janex_signature::Limits {
        max_bytes: limits.max_bytes,
        max_elements: limits.max_elements,
        max_depth: limits.max_depth,
    }
}

/// Applies the format's version rules to runtime facts and creates its condition context.
pub(crate) fn runtime_context(runtime: &JavaRuntime, invocation: Option<&str>) -> Result<Context> {
    Ok(Context {
        os: std::env::consts::OS.into(),
        arch: runtime.architecture.clone(),
        invocation: invocation.map(str::to_owned),
        runtime: Some(RuntimeContext {
            runtime_type: "janex.java".into(),
            java_version: Some(JavaVersion::parse(&runtime.version_text)?),
            vendor: runtime.vendor.clone(),
        }),
    })
}

#[cfg(test)]
mod tests {
    //! Host conversion retains strict format rules while capabilities expose runtime facts.

    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn runtime_facts_are_validated_before_format_condition_evaluation() {
        let mut runtime = JavaRuntime {
            architecture: "aarch64".into(),
            vm_name: None,
            executable: "java".into(),
            home: "jdk".into(),
            feature: 8,
            version_text: "1.8.0_402".into(),
            vendor: "Example Vendor".into(),
            modules: BTreeMap::new(),
        };
        let context = runtime_context(&runtime, Some("run")).unwrap();
        assert_eq!(context.invocation.as_deref(), Some("run"));
        assert_eq!(context.arch, "aarch64");
        let facts = context.runtime.unwrap();
        assert_eq!(facts.runtime_type, "janex.java");
        assert_eq!(facts.vendor, runtime.vendor);
        assert_eq!(
            facts.java_version.unwrap(),
            JavaVersion::parse("8.0.402").unwrap()
        );
        runtime.version_text = "25-invalid!".into();
        assert!(runtime_context(&runtime, Some("run")).is_err());
    }
}

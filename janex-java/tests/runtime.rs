// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Runtime probing against the JDK used by the integration suite.

use janex_java::runtime::{JavaOptions, JavaRuntime, candidates};

#[test]
fn discovers_and_probes_the_available_jdk_without_changing_caller_state() {
    let options = JavaOptions {
        java: Some("java".into()),
        java_home: None,
    };
    let candidates = candidates(&options).unwrap();
    assert_eq!(candidates.len(), 1);
    let runtime = JavaRuntime::probe(&candidates[0]).unwrap();
    assert!(runtime.executable.is_absolute());
    assert!(runtime.home.is_dir());
    assert!(!runtime.vendor.is_empty());
    assert!(runtime.feature >= 9);
    assert!(runtime.modules.contains_key("java.base"));
}

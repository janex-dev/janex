// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Java version timelines and caller-supplied condition evaluation.

use janex_format::{
    ErrorKind,
    cbor::Value,
    condition::{Condition, Context, RuntimeContext},
    version::{JavaRange, JavaVersion},
};

#[test]
fn aliases_and_comparison_ignore_build_information() {
    for (alias, canonical) in [
        ("1.8.0", "8"),
        ("1.8.0_0", "8"),
        ("8u402", "8.0.402"),
        ("1.8.0_402-b6", "8.0.402"),
        ("1.8.0-ea", "8-ea"),
        ("1.8.0_402-beta", "8.0.402-beta"),
        ("21.0.0", "21"),
        ("21.0.2+13-vendor", "21.0.2"),
        ("25+-vendor", "25"),
        ("25-ea-vendor", "25-ea"),
        ("25-0007", "25-7"),
    ] {
        assert_eq!(
            JavaVersion::parse(alias).unwrap(),
            JavaVersion::parse(canonical).unwrap(),
            "{alias}"
        );
    }
    let ordered = [
        "8", "8u402", "9", "21-2", "21-11", "21-alpha", "21-ea", "21", "21.0.1", "25",
    ];
    for pair in ordered.windows(2) {
        assert!(
            JavaVersion::parse(pair[0]).unwrap() < JavaVersion::parse(pair[1]).unwrap(),
            "{pair:?}"
        );
    }
    for invalid in [
        "",
        "7",
        "1.8",
        "1.9.0",
        "1.8.0+1",
        "1.8.0_402-",
        "1.8.0_402-ea-extra",
        "1.8.0_402-b06",
        "1.8.0_402-b06-extra",
        "8u01",
        "21.01",
        "21-",
        "21+",
        "21-ea+-vendor",
        "21-ea-extra+1",
        "21+01",
        "2147483648",
    ] {
        assert!(JavaVersion::parse(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn ranges_cover_intervals_exclusions_and_isolated_points() {
    for (range, included, excluded) in [
        ("*", vec!["8", "25-ea"], vec![]),
        (">=8|<9", vec!["8", "1.8.0_402"], vec!["8-ea", "9"]),
        (">=21", vec!["21", "25"], vec!["21-ea", "17"]),
        (">=21-ea", vec!["21-ea", "21"], vec!["21-alpha"]),
        ("!=17|!=21", vec!["8", "17.0.1", "25"], vec!["17", "21+7"]),
        (
            "<11|17|>=21",
            vec!["8", "17", "21", "25"],
            vec!["11", "18", "21-ea"],
        ),
        (
            ">8|!=11|<=17|>=21",
            vec!["9", "17", "21"],
            vec!["8", "11", "18"],
        ),
        ("8|17", vec!["8", "17"], vec!["8u402", "9", "25"]),
    ] {
        let range = JavaRange::parse(&format!("vers:jep322/{range}")).unwrap();
        for version in included {
            assert!(
                range.contains(&JavaVersion::parse(version).unwrap()),
                "{} should include {version}",
                range.as_str()
            );
        }
        for version in excluded {
            assert!(
                !range.contains(&JavaVersion::parse(version).unwrap()),
                "{} should exclude {version}",
                range.as_str()
            );
        }
    }
    for invalid in [
        "",
        "*|17",
        ">=17|>=21",
        "<=17|<21",
        "17|<21",
        "21|17",
        "1.8.0_402|8.0.402",
        "21|21+1",
        "21.0",
        ">= 21",
        "17||21",
    ] {
        assert!(
            JavaRange::parse(&format!("vers:jep322/{invalid}")).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn conditions_validate_before_matching_and_preserve_extensions() {
    let runtime = Value::map([
        (Value::uint(0), Value::text("janex.java")),
        (
            Value::uint(1),
            Value::map([
                (Value::uint(0), Value::text("vers:jep322/>=21")),
                (Value::uint(1), Value::text("Example Vendor")),
            ])
            .unwrap(),
        ),
    ])
    .unwrap();
    let value = Value::map([
        (
            Value::uint(1),
            Value::array([Value::text("linux"), Value::text("windows")]),
        ),
        (Value::uint(4), Value::text("run")),
        (Value::uint(5), runtime),
        (Value::uint(100), Value::null()),
    ])
    .unwrap();
    let condition = Condition::from_value(value.clone()).unwrap();
    assert_eq!(condition.value(), &value);
    let mut context = Context {
        os: "windows".into(),
        arch: "x86-64".into(),
        invocation: Some("run".into()),
        runtime: Some(RuntimeContext {
            runtime_type: "janex.java".into(),
            java_version: Some(JavaVersion::parse("25").unwrap()),
            vendor: "Example Vendor".into(),
        }),
    };
    assert!(condition.matches(&context).unwrap());
    context.invocation = None;
    assert!(!condition.matches(&context).unwrap());
    context.invocation = Some("run".into());
    context.runtime = None;
    assert!(!condition.matches(&context).unwrap());
    assert!(Condition::unconditional().matches(&context).unwrap());
    for invalid in [
        Value::map([(Value::uint(0), Value::null())]).unwrap(),
        Value::map([(Value::uint(3), Value::null())]).unwrap(),
        Value::map([(Value::uint(1), Value::array([]))]).unwrap(),
        Value::map([(Value::uint(2), Value::text(""))]).unwrap(),
        Value::map([(
            Value::uint(5),
            Value::map([
                (Value::uint(0), Value::text("janex.java")),
                (
                    Value::uint(1),
                    Value::map([(Value::uint(0), Value::text("vers:jep322/21|17"))]).unwrap(),
                ),
            ])
            .unwrap(),
        )])
        .unwrap(),
    ] {
        assert!(Condition::from_value(invalid).is_err());
    }
    let unknown = Condition::from_value(
        Value::map([(
            Value::uint(5),
            Value::map([
                (Value::uint(0), Value::text("example.runtime")),
                (Value::uint(1), Value::empty_map()),
            ])
            .unwrap(),
        )])
        .unwrap(),
    )
    .unwrap();
    assert!(!unknown.matches(&context).unwrap());
    context.runtime = Some(RuntimeContext {
        runtime_type: "example.runtime".into(),
        java_version: None,
        vendor: String::new(),
    });
    assert_eq!(
        unknown.matches(&context).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

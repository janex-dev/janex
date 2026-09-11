// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Application framing, selection, configuration merging, and external-reference validation.

use janex_format::{
    ErrorKind,
    application::{Application, PathEntry, read_applications, select_application},
    binary::Limits,
    blob::BlobRef,
    cbor::Value,
    condition::{Context, RuntimeContext},
    container::{APPLICATION, Reader, Writer},
    localized::LocalizedText,
    version::JavaVersion,
};
use std::io::Cursor;

/// Builds a deterministic integer-keyed map.
fn map(fields: impl IntoIterator<Item = (u64, Value)>) -> Value {
    Value::map(
        fields
            .into_iter()
            .map(|(key, value)| (Value::uint(key), value)),
    )
    .unwrap()
}

/// Creates application type information, retaining a sample extension.
fn type_info(id: &str, kind: &str) -> Value {
    map([
        (0, Value::text(id)),
        (1, Value::text(kind)),
        (100, Value::null()),
    ])
}

/// Wraps a Java launch configuration in its descriptor and application object.
fn java(config: Value) -> Value {
    map([(0, map([(0, config)]))])
}

/// Supplies a Java 25 command-line launch context.
fn context() -> Context {
    Context {
        os: "linux".into(),
        arch: "x86-64".into(),
        invocation: Some("run".into()),
        runtime: Some(RuntimeContext {
            runtime_type: "janex.java".into(),
            java_version: Some(JavaVersion::parse("25").unwrap()),
            vendor: "Example Vendor".into(),
        }),
    }
}

/// Creates a classpath entry point.
fn entry(name: &str) -> Value {
    map([(0, Value::text(name))])
}

/// Creates a local path entry in pool 1.
fn local(index: u64) -> Value {
    PathEntry::Local(BlobRef { pool: 1, index })
        .to_value(false)
        .unwrap()
}

#[test]
fn independent_application_bytes_and_selection() {
    let bytes = b"JANEXAPP\x0d\xa1\0\xa1\0\xa1\x01\xa1\0\x64Main";
    let application =
        Application::decode(bytes, type_info("main", "janex.java"), Limits::default()).unwrap();
    assert_eq!(application.encode().unwrap(), bytes);
    assert_eq!(
        application
            .evaluate_java(&context())
            .unwrap()
            .unwrap()
            .entry_point
            .main_class
            .as_deref(),
        Some("Main")
    );
    for end in 0..bytes.len() {
        assert!(
            Application::decode(
                &bytes[..end],
                type_info("main", "janex.java"),
                Limits::default()
            )
            .is_err()
        );
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(
        Application::decode(
            &trailing,
            type_info("main", "janex.java"),
            Limits::default()
        )
        .is_err()
    );
    for ids in [vec!["main"], vec!["main", "other"], vec!["main", "main"]] {
        let mut writer = Writer::new(Vec::new()).unwrap();
        for (index, id) in ids.iter().enumerate() {
            writer
                .write_section(
                    index as u64,
                    APPLICATION,
                    bytes,
                    Some(type_info(id, "janex.java")),
                )
                .unwrap();
        }
        let mut reader = Reader::open_auto(
            Cursor::new(writer.finish(Value::empty_map()).unwrap()),
            Limits::default(),
        )
        .unwrap();
        let apps = read_applications(&mut reader);
        if ids == ["main", "main"] {
            assert!(apps.is_err());
            continue;
        }
        let apps = apps.unwrap();
        assert_eq!(
            select_application(&apps, Some("main")).unwrap().id(),
            "main"
        );
        assert!(select_application(&apps, Some("absent")).is_err());
        assert_eq!(select_application(&apps, None).is_ok(), ids.len() == 1);
    }
    assert!(select_application(&[], None).is_err());
}

#[test]
fn overlays_append_clear_replace_and_prune_in_depth_first_order() {
    let child = map([
        (1, map([(1, Value::text("example.module"))])),
        (5, Value::array([Value::text("-Dchild=yes")])),
        (7, Value::array([Value::text("child")])),
    ]);
    let first = map([
        (1, Value::null()),
        (2, Value::null()),
        (3, Value::array([])),
        (4, Value::null()),
        (5, Value::null()),
        (6, Value::array([child])),
        (7, Value::null()),
    ]);
    let skipped = map([
        (0, map([(1, Value::text("windows"))])),
        (
            6,
            Value::array([map([
                (1, Value::null()),
                (7, Value::array([Value::text("must not appear")])),
            ])]),
        ),
    ]);
    let last = map([
        (2, Value::array([local(4)])),
        (3, Value::array([local(5)])),
        (
            4,
            Value::array([map([(0, local(6)), (1, Value::text(""))])]),
        ),
        (5, Value::array([Value::text("-Dlast=one value")])),
        (7, Value::array([Value::text(""), Value::text("last")])),
    ]);
    let value = java(map([
        (1, entry("Base")),
        (2, Value::array([local(1)])),
        (3, Value::array([local(2)])),
        (
            4,
            Value::array([map([(0, local(3)), (1, Value::text("base"))])]),
        ),
        (5, Value::array([Value::text("-Dbase=yes")])),
        (6, Value::array([first, skipped, last])),
        (7, Value::array([Value::text("base")])),
        (100, Value::text("nested extension")),
    ]));
    let app = Application::from_values(
        type_info("main", "janex.java"),
        value.clone(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(app.value(), &value);
    let launch = app.evaluate_java(&context()).unwrap().unwrap();
    assert_eq!(launch.entry_point.main_class, None);
    assert_eq!(
        launch.entry_point.main_module.as_deref(),
        Some("example.module")
    );
    assert!(matches!(
        &launch.module_path[..],
        [PathEntry::Local(BlobRef { pool: 1, index: 4 })]
    ));
    assert!(matches!(
        &launch.class_path[..],
        [
            PathEntry::Local(BlobRef { index: 2, .. }),
            PathEntry::Local(BlobRef { index: 5, .. })
        ]
    ));
    assert_eq!(launch.agents.len(), 1);
    assert_eq!(launch.agents[0].option, "");
    assert!(matches!(
        launch.agents[0].reference,
        PathEntry::Local(BlobRef { index: 6, .. })
    ));
    assert_eq!(launch.jvm_options, ["-Dchild=yes", "-Dlast=one value"]);
    assert_eq!(launch.arguments, ["child", "", "last"]);
}

#[test]
fn conditions_and_structure_are_checked_before_candidate_selection() {
    let root_mismatch = java(map([(0, map([(1, Value::text("windows"))]))]));
    let app = Application::from_values(
        type_info("main", "janex.java"),
        root_mismatch,
        Limits::default(),
    )
    .unwrap();
    assert!(app.evaluate_java(&context()).unwrap().is_none());
    let no_entry = Application::from_values(
        type_info("main", "janex.java"),
        java(Value::empty_map()),
        Limits::default(),
    )
    .unwrap();
    assert!(no_entry.evaluate_java(&context()).is_err());
    let clear_entry = java(map([
        (1, entry("Main")),
        (6, Value::array([map([(1, Value::null())])])),
    ]));
    assert!(
        Application::from_values(
            type_info("main", "janex.java"),
            clear_entry,
            Limits::default()
        )
        .unwrap()
        .evaluate_java(&context())
        .is_err()
    );
    for invalid in [
        map([(6, Value::null())]),
        map([(1, Value::empty_map())]),
        map([(1, entry(""))]),
        map([(5, Value::array([Value::uint(0)]))]),
        map([(4, Value::array([map([(0, local(0))])]))]),
        map([(
            0,
            map([(
                5,
                map([
                    (0, Value::text("janex.java")),
                    (1, map([(0, Value::text("vers:jep322/25|21"))])),
                ]),
            )]),
        )]),
    ] {
        let value = java(map([
            (0, map([(1, Value::text("windows"))])),
            (6, Value::array([invalid])),
        ]));
        assert!(
            Application::from_values(type_info("main", "janex.java"), value, Limits::default())
                .is_err()
        );
    }
    let config = java(map([
        (1, entry("Main")),
        (7, Value::array(vec![Value::text("a"); 3])),
        (
            6,
            Value::array([map([(7, Value::array(vec![Value::text("b"); 3]))])]),
        ),
    ]));
    let limits = Limits {
        max_elements: 4,
        ..Limits::default()
    };
    let app = Application::from_values(type_info("main", "janex.java"), config, limits).unwrap();
    assert_eq!(
        app.evaluate_java(&context()).unwrap_err().kind(),
        ErrorKind::Limit
    );
}

#[test]
fn external_paths_require_valid_uris_canonical_purls_and_correct_placement() {
    for uri in [
        "https://example.org/a.jar",
        "file:///tmp/a.jar",
        "pkg:maven/org.example/demo@1.0",
        "pkg:npm/%40angular/core@1.0",
    ] {
        let value = map([(0, Value::uint(1)), (1, Value::text(uri))]);
        assert!(PathEntry::from_value(&value, false).is_ok(), "{uri}");
    }
    for uri in [
        "relative.jar",
        "https://example.org/a b.jar",
        "https://example.org/%xz",
        "pkg:MAVEN/org.example/demo@1.0",
        "pkg:maven/org.example/demo@1.0?z=2&a=1",
        "pkg:janex/java-module/java.base?x=y",
        "pkg:janex/java-module/java.base#path",
        "pkg:janex/other/java.base",
        "pkg:janex/too/many/java.base",
    ] {
        let value = map([(0, Value::uint(1)), (1, Value::text(uri))]);
        assert!(PathEntry::from_value(&value, true).is_err(), "{uri}");
    }
    let value = map([
        (0, Value::uint(1)),
        (1, Value::text("pkg:janex/java-module/java.base@25")),
    ]);
    assert!(PathEntry::from_value(&value, false).is_err());
    let path = PathEntry::from_value(&value, true).unwrap();
    assert_eq!(
        path.module_requirement(),
        Some(("java.base".into(), Some("25".into())))
    );
}

#[test]
fn presentation_and_unknown_applications_preserve_original_fields() {
    let value = map([
        (0, Value::empty_map()),
        (3, Value::text("")),
        (5, Value::uint(1)),
        (
            4,
            map([
                (0, Value::text("example")),
                (1, Value::boolean(true)),
                (
                    2,
                    Value::array([map([
                        (0, Value::text("image/png")),
                        (1, BlobRef { pool: 1, index: 0 }.to_value()),
                    ])]),
                ),
            ]),
        ),
        (100, Value::array([Value::null()])),
    ]);
    let app = Application::from_values(
        type_info("id", "org.example.runtime"),
        value.clone(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(app.value(), &value);
    assert_eq!(app.title("en"), "example");
    assert_eq!(app.comment("en"), Some(""));
    assert!(app.windowed());
    assert_eq!(
        app.evaluate_java(&context()).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    for fields in [
        vec![(1, Value::text(""))],
        vec![(2, Value::text(""))],
        vec![(5, Value::uint(2))],
        vec![(4, map([(0, Value::text("../cmd"))]))],
        vec![(4, map([(1, Value::uint(1))]))],
        vec![(4, map([(2, Value::array([]))]))],
    ] {
        let mut fields = fields;
        fields.push((0, Value::empty_map()));
        assert!(
            Application::from_values(
                type_info("id", "org.example.runtime"),
                map(fields),
                Limits::default()
            )
            .is_err()
        );
    }
}

#[test]
fn localized_text_lookup_uses_truncation_and_deterministic_fallback() {
    let value = Value::map([
        (Value::text("en-US"), Value::text("English")),
        (Value::text("zh-Hant"), Value::text("Traditional")),
        (Value::text("und"), Value::text("Neutral")),
        (Value::text("bad_tag"), Value::text("Ignored")),
        (
            Value::text("x-toooolong"),
            Value::text("Ignored private tag"),
        ),
    ])
    .unwrap();
    let text = LocalizedText::from_value(value.clone(), true).unwrap();
    assert_eq!(text.value(), &value);
    assert_eq!(text.select("EN-us-x-test"), "English");
    assert_eq!(text.select("zh-Hant-TW"), "Traditional");
    assert_eq!(text.select("en-GB"), "English");
    assert_eq!(text.select("bad_tag"), "Neutral");
    assert_eq!(text.select("fr"), "Neutral");
    let fallback = LocalizedText::from_value(
        Value::map([
            (Value::text("en-US"), Value::text("English")),
            (Value::text("de"), Value::text("German")),
        ])
        .unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(fallback.select("fr"), "German");
    for value in [
        Value::empty_map(),
        Value::map([(Value::text("bad_tag"), Value::text("ignored"))]).unwrap(),
        Value::map([
            (Value::text("en"), Value::text("one")),
            (Value::text("EN"), Value::text("two")),
        ])
        .unwrap(),
        Value::map([(Value::text("en"), Value::uint(0))]).unwrap(),
    ] {
        assert!(LocalizedText::from_value(value, false).is_err());
    }
    assert_eq!(
        LocalizedText::from_value(Value::text(""), false)
            .unwrap()
            .select("fr"),
        ""
    );
    assert!(
        LocalizedText::from_value(
            Value::map([(
                Value::text("qaa-Qaaa-QQ"),
                Value::text("unregistered but well-formed")
            )])
            .unwrap(),
            true
        )
        .is_ok()
    );
}

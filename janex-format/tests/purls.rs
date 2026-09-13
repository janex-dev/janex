// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Canonical ECMA-427 component syntax, independent of library normalization.

use janex_format::purl;

#[test]
fn canonical_components_round_trip_without_losing_separator_data() {
    for uri in [
        "pkg:generic/name",
        "pkg:generic/./name",
        "pkg:generic/a%2Fb@v%2Fw",
        "pkg:generic/a%21b@v%26w?key=a%26b%3Dc#dir/file",
        "pkg:generic/name?repository_url=https:%2F%2Fexample.org%2Frepo",
        "pkg:janex/java-module/library@1%2F2",
        "pkg:npm/%40scope/name@1.0",
    ] {
        let decoded = purl::parse(uri).unwrap();
        assert_eq!(decoded.to_string(), uri);
    }
    let decoded = purl::parse("pkg:generic/a%2Fb@v%2Fw?key=a%26b%3Dc#dir/file").unwrap();
    assert_eq!(decoded.name(), "a/b");
    assert_eq!(decoded.version(), Some("v/w"));
    assert_eq!(decoded.qualifiers()["key"], "a&b=c");
    assert_eq!(decoded.subpath(), Some("dir/file"));
}

#[test]
fn noncanonical_or_ambiguous_component_spellings_are_rejected() {
    for uri in [
        "pkg:.type/name",
        "pkg:a+b/name",
        "pkg:1type/name",
        "pkg:TYPE/name",
        "pkg:generic/n?_=v",
        "pkg:generic/n?.a=v",
        "pkg:generic/n?A=v",
        "pkg:generic/n?a=v&a=w",
        "pkg:generic/n?z=v&a=w",
        "pkg:generic/n?a=",
        "pkg:generic/n?",
        "pkg:generic/n#",
        "pkg:generic/n@",
        "pkg:generic/n@v/w",
        "pkg:generic/n#../file",
        "pkg:generic/a%2Fb/n",
        "pkg:generic/n#dir%2Ffile",
        "pkg:generic/n%41",
        "pkg:generic/n%3A",
        "pkg:generic/n%2f",
        "pkg:generic/n%FF",
        "pkg:generic/a!b",
        "pkg:generic/n?v=a&b",
        "pkg:generic/n?v=a=b",
        "pkg:generic//name",
        "pkg://generic/name",
        "PKG:generic/name",
    ] {
        assert!(purl::parse(uri).is_err(), "{uri}");
    }
}

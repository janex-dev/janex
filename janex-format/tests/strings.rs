// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! String-pool invariants and independent name encoding vectors.

use janex_format::{
    ErrorKind,
    binary::{Decoder, Limits},
    strings::StringPool,
};

#[test]
fn pool_and_three_name_forms_share_class_basenames() {
    let mut pool = StringPool::decode(b"\x03\x00\x06Object\x06.class", Limits::default()).unwrap();
    assert_eq!(pool.intern("Object"), 1);
    assert_eq!(pool.get(0).unwrap(), "");
    assert!(!pool.is_empty());
    for (wire, expected) in [
        (&b"\x01"[..], "Object"),
        (&b"\x00\x03foo"[..], "foo"),
        (&b"\x00\x00\x02\x01\x02"[..], "Object.class"),
        (&b"\x00\x00\x03\x00\x01\x02"[..], "Object.class"),
    ] {
        let mut decoder = Decoder::new(wire, Limits::default()).unwrap();
        assert_eq!(pool.read_nonempty(&mut decoder).unwrap(), expected);
        decoder.finish().unwrap();
    }
    let mut encoded = Vec::new();
    pool.write_nonempty("Object.class", &mut encoded).unwrap();
    assert_eq!(encoded, b"\x00\x00\x02\x01\x02");
    assert_eq!(pool.find("Object.class"), None);
    assert_eq!(pool.encode().unwrap(), b"\x03\x00\x06Object\x06.class");
}

#[test]
fn invalid_pools_references_and_concatenation_limits() {
    for invalid in [
        &b"\x00"[..],
        &b"\x01\x01x"[..],
        &b"\x02\x00\x00"[..],
        &b"\x02\x00\x01\xff"[..],
        &b"\x01\x00\x00"[..],
    ] {
        assert!(StringPool::decode(invalid, Limits::default()).is_err());
    }
    let mut pool = StringPool::new();
    pool.intern("hello");
    for invalid in [
        &b"\x02"[..],
        &b"\x00\x00\x00"[..],
        &b"\x00\x00\x01\x01"[..],
        &b"\x00\x00\x02\x00\x00"[..],
        &b"\x00\x00\x02\x01\x02"[..],
    ] {
        assert!(
            pool.read_nonempty(&mut Decoder::new(invalid, Limits::default()).unwrap())
                .is_err()
        );
    }
    let limits = Limits {
        max_bytes: 8,
        ..Limits::default()
    };
    assert_eq!(
        pool.read_nonempty(&mut Decoder::new(b"\x00\x00\x02\x01\x01", limits).unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
    pool.intern("a large shared string");
    assert_eq!(
        pool.read_nonempty(&mut Decoder::new(b"\x02", limits).unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
}

// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Opaque data-pool invariants and independent UTF-8 name encoding vectors.

use janex_format::{
    ErrorKind,
    binary::{Decoder, Limits},
    data_pool::{DataPool, DataPoolBuilder},
};

#[test]
fn freezing_and_reopening_a_builder_preserves_indices_and_owned_bytes() {
    let mut builder = DataPoolBuilder::new();
    let mut input = vec![0xff, 0xc0, 0x80];
    assert_eq!(builder.intern(&input), 1);
    input.fill(0);
    assert_eq!(builder.intern("Object"), 2);
    assert_eq!(builder.intern("Object"), 2);
    let pool = builder.finish();
    let wire = pool.encode().unwrap();
    assert_eq!(pool.get(1).unwrap(), b"\xff\xc0\x80");
    assert!(pool.get(u64::MAX).is_err());
    assert!(pool.get(3).is_err());
    let mut builder = DataPoolBuilder::from(pool);
    assert_eq!(builder.intern("Object"), 2);
    assert_eq!(builder.intern(".class"), 3);
    let pool = builder.finish();
    assert_eq!(pool.get(3).unwrap(), b".class");
    assert_eq!(DataPool::decode(&wire, Limits::default()).unwrap().len(), 3);
    assert_eq!(
        DataPool::decode(&pool.encode().unwrap(), Limits::default())
            .unwrap()
            .get(3)
            .unwrap(),
        b".class"
    );
}

#[test]
fn opaque_entries_are_valid_until_interpreted_as_text() {
    let wire = b"\x03\x00\x01\xff\x02\xc0\x80";
    let pool = DataPool::decode(wire, Limits::default()).unwrap();
    assert_eq!(pool.get(1).unwrap(), b"\xff");
    assert_eq!(pool.get(2).unwrap(), b"\xc0\x80");
    assert_eq!(pool.encode().unwrap(), wire);
    for value in [b"\x01".as_slice(), b"\x00\x00\x02\x00\x02"] {
        assert!(
            janex_format::strings::read_nonempty(
                &pool,
                &mut Decoder::new(value, Limits::default()).unwrap()
            )
            .is_err()
        );
    }
    assert!(DataPool::decode(b"\x03\x00\x01\xff\x01\xff", Limits::default()).is_err());
}

#[test]
fn pool_and_three_name_forms_share_class_basenames() {
    let mut pool = DataPoolBuilder::from(
        DataPool::decode(b"\x03\x00\x06Object\x06.class", Limits::default()).unwrap(),
    );
    assert_eq!(pool.intern("Object"), 1);
    assert_eq!(pool.get(0).unwrap(), b"");
    assert!(!pool.is_empty());
    for (wire, expected) in [
        (&b"\x01"[..], "Object"),
        (&b"\x00\x03foo"[..], "foo"),
        (&b"\x00\x00\x02\x01\x02"[..], "Object.class"),
        (&b"\x00\x00\x03\x00\x01\x02"[..], "Object.class"),
    ] {
        let mut decoder = Decoder::new(wire, Limits::default()).unwrap();
        assert_eq!(
            janex_format::strings::read_nonempty(&pool, &mut decoder).unwrap(),
            expected
        );
        decoder.finish().unwrap();
    }
    let mut encoded = Vec::new();
    janex_format::strings::write_nonempty(&mut pool, "Object.class", &mut encoded).unwrap();
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
        &b"\x01\x00\x00"[..],
    ] {
        assert!(DataPool::decode(invalid, Limits::default()).is_err());
    }
    let mut pool = DataPoolBuilder::new();
    pool.intern("hello");
    for invalid in [
        &b"\x02"[..],
        &b"\x00\x00\x00"[..],
        &b"\x00\x00\x01\x01"[..],
        &b"\x00\x00\x02\x00\x00"[..],
        &b"\x00\x00\x02\x01\x02"[..],
    ] {
        assert!(
            janex_format::strings::read_nonempty(
                &pool,
                &mut Decoder::new(invalid, Limits::default()).unwrap()
            )
            .is_err()
        );
    }
    let limits = Limits {
        max_bytes: 8,
        ..Limits::default()
    };
    assert_eq!(
        janex_format::strings::read_nonempty(
            &pool,
            &mut Decoder::new(b"\x00\x00\x02\x01\x01", limits).unwrap()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Limit
    );
    pool.intern("a large shared string");
    assert_eq!(
        janex_format::strings::read_nonempty(&pool, &mut Decoder::new(b"\x02", limits).unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
}

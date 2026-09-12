// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Wire vectors and malformed-input tests independent of the container writer.

use janex_format::{
    ErrorKind,
    binary::{Decoder, Limits, write_vuint},
    cbor::{self, Value},
    checksum::{Algorithm, Checksum},
};

#[test]
fn signed_128_bit_integers_use_minimal_cbor_magnitudes() {
    for (value, wire) in [
        (0, vec![0]),
        (-1, vec![0x20]),
        (u64::MAX as i128, [vec![0x1b], vec![0xff; 8]].concat()),
        (-1 - u64::MAX as i128, [vec![0x3b], vec![0xff; 8]].concat()),
        (1i128 << 64, [vec![0xc2, 0x49, 1], vec![0; 8]].concat()),
        (
            -1 - (1i128 << 64),
            [vec![0xc3, 0x49, 1], vec![0; 8]].concat(),
        ),
        (i128::MAX, [vec![0xc2, 0x50, 0x7f], vec![0xff; 15]].concat()),
        (i128::MIN, [vec![0xc3, 0x50, 0x7f], vec![0xff; 15]].concat()),
    ] {
        assert_eq!(Value::integer(value).as_bytes(), wire);
        assert_eq!(
            Value::from_bytes(&wire, Limits::default())
                .unwrap()
                .as_i128()
                .unwrap(),
            value
        );
    }
    for wire in [
        vec![0xc2, 0x41, 0x01],
        [vec![0xc2, 0x48], vec![0xff; 8]].concat(),
        [vec![0xc3, 0x49], vec![0; 9]].concat(),
        [vec![0xc2, 0x50, 0x80], vec![0; 15]].concat(),
        [vec![0xc3, 0x50, 0x80], vec![0; 15]].concat(),
        [vec![0xc2, 0x51, 1], vec![0; 16]].concat(),
        vec![0xc4, 0],
        vec![0xf6],
    ] {
        assert!(
            Value::from_bytes(&wire, Limits::default())
                .unwrap()
                .as_i128()
                .is_err(),
            "{wire:?}"
        );
    }
}

#[test]
fn uleb128_wire_vectors() {
    for (value, expected) in [
        (0, vec![0]),
        (127, vec![127]),
        (128, vec![0x80, 1]),
        (624485, vec![0xe5, 0x8e, 0x26]),
        (
            u64::MAX,
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 1],
        ),
    ] {
        let mut bytes = Vec::new();
        write_vuint(&mut bytes, value).unwrap();
        assert_eq!(bytes, expected);
        let mut decoder = Decoder::new(&expected, Limits::default()).unwrap();
        assert_eq!(decoder.vuint().unwrap(), value);
        decoder.finish().unwrap();
    }
    assert_eq!(
        Decoder::new(&[0x80, 0], Limits::default())
            .unwrap()
            .vuint()
            .unwrap(),
        0
    );
    for bytes in [
        vec![0x80],
        vec![0xff; 10],
        vec![0x80; 11],
        vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 2],
    ] {
        assert!(
            Decoder::new(&bytes, Limits::default())
                .unwrap()
                .vuint()
                .is_err()
        );
    }
}

#[test]
fn sized_fields_enforce_bounds_and_utf8() {
    assert!(
        Decoder::new(&[5, 1, 2], Limits::default())
            .unwrap()
            .sized()
            .is_err()
    );
    assert!(
        Decoder::new(&[1, 0xff], Limits::default())
            .unwrap()
            .string()
            .is_err()
    );
    let limits = Limits {
        max_bytes: 8,
        ..Limits::default()
    };
    assert_eq!(
        Decoder::new(&[9], limits)
            .unwrap()
            .sized()
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
    assert!(Decoder::new(&[0; 9], limits).is_err());
}

#[test]
fn deterministic_cbor_rejects_nonpreferred_and_malformed_items() {
    for bytes in [
        vec![0x18, 0],
        vec![0x19, 0, 24],
        vec![0x9f, 0xff],
        vec![0xa2, 1, 0, 0, 0],
        vec![0xa2, 0, 0, 0, 1],
        vec![0x61, 0xff],
        vec![0x81],
        vec![0x1c],
        vec![0xf8, 24],
        vec![0xfa, 0x3f, 0x80, 0, 0],
        vec![0xfb, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0],
        vec![0xf9, 0x7e, 1],
    ] {
        assert!(
            Value::from_bytes(&bytes, Limits::default()).is_err(),
            "accepted {bytes:02x?}"
        );
    }
    for bytes in [
        vec![0xf9, 0x3c, 0],
        vec![0xf9, 0x80, 0],
        vec![0xf9, 0x7e, 0],
        vec![0xf8, 32],
        vec![0x20],
    ] {
        assert!(
            Value::from_bytes(&bytes, Limits::default()).is_ok(),
            "rejected {bytes:02x?}"
        );
    }
    assert!(Value::from_bytes(&[0, 1], Limits::default()).is_err());
}

#[test]
fn map_order_is_bytewise_not_length_first() {
    let map = Value::map([
        (Value::text(""), Value::uint(2)),
        (Value::uint(100), Value::uint(1)),
    ])
    .unwrap();
    assert_eq!(map.as_bytes(), &[0xa2, 0x18, 100, 1, 0x60, 2]);
    Value::from_bytes(map.as_bytes(), Limits::default()).unwrap();
    assert_eq!(map.required(100).unwrap().as_u64().unwrap(), 1);
    assert!(
        Value::map([
            (Value::uint(1), Value::null()),
            (Value::uint(1), Value::null())
        ])
        .is_err()
    );
}

#[test]
fn sized_cbor_empty_map_is_not_an_empty_payload_of_another_type() {
    let mut decoder = Decoder::new(&[0], Limits::default()).unwrap();
    assert!(cbor::read_sized(&mut decoder).unwrap().is_empty_map());
    assert!(cbor::read_sized(&mut Decoder::new(&[1, 0xa0], Limits::default()).unwrap()).is_err());
    for bytes in [[1, 0x80], [1, 0x60], [1, 0xf6]] {
        let value =
            cbor::read_sized(&mut Decoder::new(&bytes, Limits::default()).unwrap()).unwrap();
        let mut encoded = Vec::new();
        cbor::write_sized(&mut encoded, &value).unwrap();
        assert_eq!(encoded, bytes);
    }
    let mut encoded = Vec::new();
    cbor::write_sized(&mut encoded, &Value::empty_map()).unwrap();
    assert_eq!(encoded, [0]);
}

#[test]
fn cbor_limits_apply_before_large_collections_and_nested_values() {
    let limits = Limits {
        max_elements: 1,
        max_depth: 1,
        ..Limits::default()
    };
    assert_eq!(
        Value::from_bytes(&[0x82, 0, 0], limits).unwrap_err().kind(),
        ErrorKind::Limit
    );
    assert_eq!(
        Value::from_bytes(&[0x81, 0x81, 0], limits)
            .unwrap_err()
            .kind(),
        ErrorKind::Limit
    );
}

#[test]
fn digest_known_answers_and_wire_representation() {
    for (algorithm, id, secure, input, expected) in [
        (Algorithm::Xxh3_64, 0x11, false, "", "2d06800538d394c2"),
        (Algorithm::Xxh3_64, 0x11, false, "abc", "78af5f94892f3950"),
        (
            Algorithm::Xxh3_128,
            0x12,
            false,
            "",
            "99aa06d3014798d86001c324468d497f",
        ),
        (
            Algorithm::Xxh3_128,
            0x12,
            false,
            "abc",
            "06b05ab6733a618578af5f94892f3950",
        ),
        (
            Algorithm::Sha256,
            0x21,
            true,
            "abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            Algorithm::Sha512,
            0x22,
            true,
            "abc",
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
        (
            Algorithm::Sm3,
            0x31,
            true,
            "abc",
            "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0",
        ),
    ] {
        let checksum = Checksum::compute(algorithm, input.as_bytes()).unwrap();
        let actual: String = checksum
            .digest()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(checksum.encode()[0], id);
        assert_eq!(Algorithm::from_id(id).unwrap(), algorithm);
        assert_eq!(algorithm.is_secure(), secure);
        assert_eq!(checksum.digest().len(), algorithm.digest_length());
        assert_eq!(Checksum::decode(&checksum.encode()).unwrap(), checksum);
        let mut truncated = checksum.encode();
        truncated.pop();
        assert!(Checksum::decode(&truncated).is_err());
        let mut extended = checksum.encode();
        extended.push(0);
        assert!(Checksum::decode(&extended).is_err());
        checksum.verify(input.as_bytes()).unwrap();
        assert_eq!(
            checksum.verify(&b"wrong"[..]).unwrap_err().kind(),
            ErrorKind::Verification
        );
        let cbor = Value::bytes(&checksum.encode());
        assert_eq!(
            Checksum::decode(cbor.as_byte_string().unwrap()).unwrap(),
            checksum
        );
    }
    assert!(Checksum::decode(&[0]).is_err());
    assert!(Checksum::decode(&[0x12, 0]).is_err());
    for id in [1, 2, 3, 4, 5, 0x10, 0x13, 0x20, 0x23, 0x30, 0x32, 0xff] {
        assert_eq!(
            Checksum::decode(&[id, 0]).unwrap_err().kind(),
            ErrorKind::Unsupported,
            "accepted undefined algorithm {id:#04x}",
        );
    }
}

#[test]
fn xxh3_streaming_matches_independent_c_vectors() {
    // Generated with xxHash 0.8.2's XXH3_64bits and XXH3_128bits C APIs.
    // Input byte i is i % 251; digests use the canonical big-endian representation.
    for (length, expected64, expected128) in [
        (
            16,
            0x8355e3a6f61770dbu64,
            0x72950631827607e2842812cc870dcae2u128,
        ),
        (17, 0x9ef341a99de37328, 0x685bc458b37d057fc06e233df7729217),
        (128, 0x85c6174c7ff4c46b, 0x14792fc3af88dc6c05321a0b64d67b41),
        (129, 0xec7642b431ba3e5a, 0xdd5e74ac6b45f54ebc30b63382b09a3b),
        (240, 0x375a384d957fe865, 0x65b5be86da5540e7c92b68e16f83bbb6),
        (241, 0x02e8cd95421c6d02, 0x1da1cb61bcb8a2a102e8cd95421c6d02),
        (1024, 0xe5d78bafa45b2aa5, 0xd0ac1f7b93bf57b9e5d78bafa45b2aa5),
        (
            65537,
            0x70331d53d92bbc56,
            0x0924e7a3a30e818770331d53d92bbc56,
        ),
    ] {
        let input: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
        for (algorithm, expected) in [
            (Algorithm::Xxh3_64, expected64.to_be_bytes().to_vec()),
            (Algorithm::Xxh3_128, expected128.to_be_bytes().to_vec()),
        ] {
            assert_eq!(
                Checksum::compute(algorithm, input.as_slice())
                    .unwrap()
                    .digest(),
                expected,
                "{algorithm:?}, length {length}",
            );
            for split in [1, length / 2, length - 1] {
                let reader = std::io::Read::chain(&input[..split], &input[split..]);
                assert_eq!(
                    Checksum::compute(algorithm, reader).unwrap().digest(),
                    expected,
                    "{algorithm:?}, length {length}, split {split}",
                );
            }
        }
    }
}

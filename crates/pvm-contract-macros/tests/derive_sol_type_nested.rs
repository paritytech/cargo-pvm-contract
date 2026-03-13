#[cfg(feature = "abi-reflection")]
extern crate alloc;

use pvm_contract_macros::SolType;
use pvm_contract_types::{SolDecode, SolEncode, StaticEncodedLen};
use ruint::aliases::U256;

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct Point {
    x: U256,
    y: U256,
}

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct Line {
    start: Point,
    end: Point,
}

#[test]
fn nested_custom_type_roundtrip() {
    assert_eq!(Line::ENCODED_SIZE, 128);

    let line = Line {
        start: Point {
            x: U256::from(1u64),
            y: U256::from(2u64),
        },
        end: Point {
            x: U256::from(3u64),
            y: U256::from(4u64),
        },
    };

    let mut buf = [0u8; 128];
    line.encode_to(&mut buf);
    assert_eq!(Line::decode(&buf), line);
}

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct NamedPoint {
    point: Point,
    name: String,
}

#[test]
fn nested_custom_type_with_dynamic_field_roundtrip() {
    let val = NamedPoint {
        point: Point {
            x: U256::from(10u64),
            y: U256::from(20u64),
        },
        name: String::from("origin"),
    };

    let len = val.encode_len();
    let mut buf = vec![0u8; len];
    val.encode_to(&mut buf);
    assert_eq!(NamedPoint::decode(&buf), val);
}

use pvm_contract_types::Address;

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct AllPrimitives {
    a_u8: u8,
    a_u16: u16,
    a_u32: u32,
    a_u64: u64,
    a_u128: u128,
    a_u256: U256,
    a_i8: i8,
    a_i16: i16,
    a_i32: i32,
    a_i64: i64,
    a_i128: i128,
    a_bool: bool,
    a_address: Address,
}

#[test]
fn all_primitive_types_roundtrip() {
    // 13 fields × 32 bytes = 416
    assert_eq!(AllPrimitives::ENCODED_SIZE, 416);

    let val = AllPrimitives {
        a_u8: 255,
        a_u16: 65535,
        a_u32: 1_000_000,
        a_u64: 1_000_000_000_000,
        a_u128: 340_282_366_920_938_463_463_374_607_431_768_211_455,
        a_u256: U256::MAX,
        a_i8: -128,
        a_i16: -32768,
        a_i32: -2_147_483_648,
        a_i64: -9_223_372_036_854_775_808,
        a_i128: -170_141_183_460_469_231_731_687_303_715_884_105_728,
        a_bool: true,
        a_address: Address([0xab; 20]),
    };

    let mut buf = [0u8; 416];
    val.encode_to(&mut buf);
    assert_eq!(AllPrimitives::decode(&buf), val);
}

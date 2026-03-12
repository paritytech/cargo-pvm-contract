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

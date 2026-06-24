extern crate alloc;

use alloc::vec;

use alloy_core::sol_types::SolValue;
use proptest::prelude::*;
use pvm_contract_sdk::SolDecode;
use pvm_contract_sdk::SolEncode;
use pvm_contract_sdk::SolType;

#[derive(Debug, PartialEq, Eq, Clone, Copy, SolType)]
enum Color {
    Red,
    Green,
    Blue,
}

const COLOR_COUNT: u8 = 3;

fn color_of(discriminant: u8) -> Color {
    match discriminant {
        0 => Color::Red,
        1 => Color::Green,
        _ => Color::Blue,
    }
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct Tagged {
    color: Color,
    value: u64,
}

fn word_with_byte(b: u8) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[31] = b;
    w
}

#[test]
fn enum_sol_name_is_uint8() {
    assert_eq!(<Color as SolEncode>::SOL_NAME, "uint8");
    const { assert!(!<Color as SolEncode>::IS_DYNAMIC) };
    assert_eq!(<Color as SolEncode>::HEAD_SIZE, 32);
}

#[test]
fn enum_roundtrip_matches_alloy_uint8_encoding_proptest() {
    proptest!(|(discriminant in 0u8..COLOR_COUNT)| {
        let variant = color_of(discriminant);
        let mut buf = vec![0u8; variant.encode_len()];
        variant.encode_to(&mut buf);
        let alloy = alloy_core::primitives::U256::from(discriminant).abi_encode();
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(buf[31], discriminant);
        prop_assert_eq!(Color::decode(&buf).unwrap(), variant);
    });
}

#[test]
fn enum_decode_rejects_every_out_of_range_discriminant_proptest() {
    proptest!(|(discriminant in COLOR_COUNT..=u8::MAX)| {
        prop_assert!(Color::decode(&word_with_byte(discriminant)).is_err());
    });
}

#[test]
fn enum_decode_rejects_truncated_input() {
    assert!(Color::decode(&[0u8; 31]).is_err());
}

#[test]
fn struct_with_enum_field_uses_uint8_in_signature() {
    assert_eq!(<Tagged as SolEncode>::SOL_NAME, "(uint8,uint64)");
    const { assert!(!<Tagged as SolEncode>::IS_DYNAMIC) };
}

#[test]
fn struct_with_enum_field_roundtrips() {
    let value = Tagged {
        color: Color::Green,
        value: 7,
    };
    let mut buf = [0u8; 64];
    value.encode_to(&mut buf);
    assert_eq!(Tagged::decode(&buf).unwrap(), value);
}

#[test]
fn struct_with_enum_field_rejects_out_of_range_field() {
    let mut buf = [0u8; 64];
    buf[31] = 9;
    assert!(Tagged::decode(&buf).is_err());
}

#[test]
fn tuple_with_enum_roundtrips_and_rejects_out_of_range() {
    let value = (Color::Blue, 42u64);
    let mut buf = [0u8; 64];
    value.encode_to(&mut buf);
    assert_eq!(<(Color, u64)>::decode(&buf).unwrap(), value);

    buf[31] = 200;
    assert!(<(Color, u64)>::decode(&buf).is_err());
}

#[test]
fn fixed_array_of_enum_roundtrips_and_rejects_out_of_range_element() {
    let value = [Color::Red, Color::Blue];
    let mut buf = [0u8; 64];
    value.encode_to(&mut buf);
    assert_eq!(<[Color; 2]>::decode(&buf).unwrap(), value);

    buf[63] = 5;
    assert!(<[Color; 2]>::decode(&buf).is_err());
}

#[test]
fn vec_of_enum_roundtrips_and_rejects_out_of_range_element() {
    let value = alloc::vec![Color::Red, Color::Green, Color::Blue];
    let len = value.encode_len();
    let mut buf = alloc::vec![0u8; len];
    value.encode_to(&mut buf);
    assert_eq!(<alloc::vec::Vec<Color>>::decode(&buf).unwrap(), value);

    let last = buf.len() - 1;
    buf[last] = 7;
    assert!(<alloc::vec::Vec<Color>>::decode(&buf).is_err());
}

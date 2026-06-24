extern crate alloc;

use pvm_contract_sdk::SolDecode;
use pvm_contract_sdk::SolEncode;
use pvm_contract_sdk::SolType;

#[derive(Debug, PartialEq, Eq, Clone, Copy, SolType)]
enum Color {
    Red,
    Green,
    Blue,
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
fn enum_roundtrip_every_variant() {
    for (variant, discriminant) in [(Color::Red, 0u8), (Color::Green, 1), (Color::Blue, 2)] {
        let mut buf = [0u8; 32];
        variant.encode_to(&mut buf);
        assert_eq!(buf, word_with_byte(discriminant));
        assert_eq!(Color::decode(&buf).unwrap(), variant);
    }
}

#[test]
fn enum_decode_accepts_last_valid_discriminant() {
    assert_eq!(Color::decode(&word_with_byte(2)).unwrap(), Color::Blue);
}

#[test]
fn enum_decode_rejects_first_out_of_range_discriminant() {
    assert!(Color::decode(&word_with_byte(3)).is_err());
}

#[test]
fn enum_decode_rejects_max_byte_discriminant() {
    assert!(Color::decode(&word_with_byte(255)).is_err());
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

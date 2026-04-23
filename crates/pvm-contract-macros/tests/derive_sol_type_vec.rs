use pvm_contract_sdk::SolType;
use pvm_contract_sdk::U256;
use pvm_contract_sdk::{SolDecode, SolEncode};

#[derive(Debug, PartialEq, Eq, SolType)]
struct WithVecU256 {
    items: Vec<U256>,
}

#[test]
fn test_derive_with_vec_u256() {
    let s = WithVecU256 {
        items: vec![U256::from(1u64), U256::from(2u64)],
    };

    let len = s.encode_body_len();
    assert_eq!(len, 128);

    let mut buf = vec![0u8; len];
    s.encode_body_to(&mut buf);

    let offset_bytes = &buf[0..32];
    let offset_val = u64::from_be_bytes([
        offset_bytes[24],
        offset_bytes[25],
        offset_bytes[26],
        offset_bytes[27],
        offset_bytes[28],
        offset_bytes[29],
        offset_bytes[30],
        offset_bytes[31],
    ]);
    assert_eq!(offset_val, 32);

    let length_bytes = &buf[32..64];
    let length_val = u64::from_be_bytes([
        length_bytes[24],
        length_bytes[25],
        length_bytes[26],
        length_bytes[27],
        length_bytes[28],
        length_bytes[29],
        length_bytes[30],
        length_bytes[31],
    ]);
    assert_eq!(length_val, 2);

    let elem1_bytes = &buf[64..96];
    assert_eq!(elem1_bytes[31], 1);
    assert!(elem1_bytes[0..31].iter().all(|&b| b == 0));

    let elem2_bytes = &buf[96..128];
    assert_eq!(elem2_bytes[31], 2);
    assert!(elem2_bytes[0..31].iter().all(|&b| b == 0));

    let decoded = WithVecU256::decode_at(&buf, 0);
    assert_eq!(decoded, s);
}

#[test]
fn test_derive_with_empty_vec() {
    let s = WithVecU256 { items: vec![] };

    let len = s.encode_body_len();
    assert_eq!(len, 64);

    let mut buf = vec![0u8; len];
    s.encode_body_to(&mut buf);

    let offset_bytes = &buf[0..32];
    let offset_val = u64::from_be_bytes([
        offset_bytes[24],
        offset_bytes[25],
        offset_bytes[26],
        offset_bytes[27],
        offset_bytes[28],
        offset_bytes[29],
        offset_bytes[30],
        offset_bytes[31],
    ]);
    assert_eq!(offset_val, 32);

    let length_bytes = &buf[32..64];
    assert!(length_bytes.iter().all(|&b| b == 0));
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct MixedFields {
    id: u32,
    items: Vec<U256>,
}

#[test]
fn test_derive_with_mixed_fields() {
    let s = MixedFields {
        id: 42u32,
        items: vec![U256::from(100u64), U256::from(200u64)],
    };

    let len = s.encode_body_len();
    assert_eq!(len, 160);

    let mut buf = vec![0u8; len];
    s.encode_body_to(&mut buf);

    let id_bytes = &buf[0..32];
    assert_eq!(id_bytes[28..32], [0, 0, 0, 42]);
    assert!(id_bytes[0..28].iter().all(|&b| b == 0));

    let offset_bytes = &buf[32..64];
    let offset_val = u64::from_be_bytes([
        offset_bytes[24],
        offset_bytes[25],
        offset_bytes[26],
        offset_bytes[27],
        offset_bytes[28],
        offset_bytes[29],
        offset_bytes[30],
        offset_bytes[31],
    ]);
    assert_eq!(offset_val, 64);

    let length_bytes = &buf[64..96];
    let length_val = u64::from_be_bytes([
        length_bytes[24],
        length_bytes[25],
        length_bytes[26],
        length_bytes[27],
        length_bytes[28],
        length_bytes[29],
        length_bytes[30],
        length_bytes[31],
    ]);
    assert_eq!(length_val, 2);

    let elem1_bytes = &buf[96..128];
    assert_eq!(elem1_bytes[31], 100);

    let elem2_bytes = &buf[128..160];
    assert_eq!(elem2_bytes[31], 200);

    let decoded = MixedFields::decode_at(&buf, 0);
    assert_eq!(decoded, s);
}

#[test]
fn test_dynamic_struct_no_static_encoded_len() {
    let s = WithVecU256 {
        items: vec![U256::from(1u64)],
    };

    let len = s.encode_body_len();
    assert_eq!(len, 96);
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct MultipleVecs {
    first: Vec<U256>,
    second: Vec<U256>,
}

#[test]
fn test_derive_with_multiple_vec_fields() {
    let s = MultipleVecs {
        first: vec![U256::from(1u64)],
        second: vec![U256::from(2u64), U256::from(3u64)],
    };

    let len = s.encode_body_len();
    assert_eq!(len, 224);

    let mut buf = vec![0u8; len];
    s.encode_body_to(&mut buf);

    let first_offset_bytes = &buf[0..32];
    let first_offset = u64::from_be_bytes([
        first_offset_bytes[24],
        first_offset_bytes[25],
        first_offset_bytes[26],
        first_offset_bytes[27],
        first_offset_bytes[28],
        first_offset_bytes[29],
        first_offset_bytes[30],
        first_offset_bytes[31],
    ]);
    assert_eq!(first_offset, 64);

    let second_offset_bytes = &buf[32..64];
    let second_offset = u64::from_be_bytes([
        second_offset_bytes[24],
        second_offset_bytes[25],
        second_offset_bytes[26],
        second_offset_bytes[27],
        second_offset_bytes[28],
        second_offset_bytes[29],
        second_offset_bytes[30],
        second_offset_bytes[31],
    ]);
    assert_eq!(second_offset, 128);

    let first_len_bytes = &buf[64..96];
    let first_len = u64::from_be_bytes([
        first_len_bytes[24],
        first_len_bytes[25],
        first_len_bytes[26],
        first_len_bytes[27],
        first_len_bytes[28],
        first_len_bytes[29],
        first_len_bytes[30],
        first_len_bytes[31],
    ]);
    assert_eq!(first_len, 1);

    let second_len_bytes = &buf[128..160];
    let second_len = u64::from_be_bytes([
        second_len_bytes[24],
        second_len_bytes[25],
        second_len_bytes[26],
        second_len_bytes[27],
        second_len_bytes[28],
        second_len_bytes[29],
        second_len_bytes[30],
        second_len_bytes[31],
    ]);
    assert_eq!(second_len, 2);

    let decoded = MultipleVecs::decode_at(&buf, 0);
    assert_eq!(decoded, s);
}

#[test]
fn test_derive_sol_type_name_signature() {
    assert_eq!(<MixedFields as SolEncode>::SOL_NAME, "(uint32,uint256[])");
    assert_eq!(<WithVecU256 as SolEncode>::SOL_NAME, "(uint256[])");
}

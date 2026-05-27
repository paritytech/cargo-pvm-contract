#![cfg(feature = "alloc")]

extern crate alloc;

use super::*;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;
use alloy_core::primitives::{Address as AlloyAddress, FixedBytes};
use alloy_core::sol_types::SolValue;
use proptest::prelude::*;

#[test]
fn encode_decode_uint256_proptest() {
    proptest!(|(v: [u64; 4])| {
        let val = U256::from_limbs(v);
        let mut buf = vec![0u8; 32];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &val.abi_encode());
        prop_assert_eq!(U256::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_u128_proptest() {
    proptest!(|(val: u128)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u128::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_u64_proptest() {
    proptest!(|(val: u64)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u64::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_u32_proptest() {
    proptest!(|(val: u32)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u32::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_u16_proptest() {
    proptest!(|(val: u16)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u16::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_u8_proptest() {
    proptest!(|(val: u8)| {
        // alloy doesn't implement SolValue for u8 (ambiguous: uint8 vs bytes1)
        let alloy = U256::from(val).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u8::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_i128_proptest() {
    proptest!(|(val: i128)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i128::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_i64_proptest() {
    proptest!(|(val: i64)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i64::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_i32_proptest() {
    proptest!(|(val: i32)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i32::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_i16_proptest() {
    proptest!(|(val: i16)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i16::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_i8_proptest() {
    proptest!(|(val: i8)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i8::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_signed_negative_values() {
    // Use -2 instead of -1 so fill bytes and value bytes are distinguishable
    let val_i128: i128 = -2;
    let mut buf = vec![0u8; 32];
    val_i128.encode_to(&mut buf);
    assert_eq!(&buf[..16], &[0xff; 16]); // sign-extension fill
    assert_eq!(&buf[16..32], val_i128.to_be_bytes()); // value bytes
    assert_eq!(i128::decode(&buf).unwrap(), val_i128);

    let val_i64: i64 = -2;
    let mut buf = vec![0u8; 32];
    val_i64.encode_to(&mut buf);
    assert_eq!(&buf[..24], &[0xff; 24]);
    assert_eq!(&buf[24..32], val_i64.to_be_bytes());
    assert_eq!(i64::decode(&buf).unwrap(), val_i64);

    let val_i32: i32 = -2;
    let mut buf = vec![0u8; 32];
    val_i32.encode_to(&mut buf);
    assert_eq!(&buf[..28], &[0xff; 28]);
    assert_eq!(&buf[28..32], val_i32.to_be_bytes());
    assert_eq!(i32::decode(&buf).unwrap(), val_i32);

    let val_i16: i16 = -2;
    let mut buf = vec![0u8; 32];
    val_i16.encode_to(&mut buf);
    assert_eq!(&buf[..30], &[0xff; 30]);
    assert_eq!(&buf[30..32], val_i16.to_be_bytes());
    assert_eq!(i16::decode(&buf).unwrap(), val_i16);

    let val_i8: i8 = -2;
    let mut buf = vec![0u8; 32];
    val_i8.encode_to(&mut buf);
    assert_eq!(&buf[..31], &[0xff; 31]);
    assert_eq!(&buf[31..32], &val_i8.to_be_bytes());
    assert_eq!(i8::decode(&buf).unwrap(), val_i8);
}

#[test]
fn encode_decode_bool_proptest() {
    proptest!(|(val: bool)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(bool::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_address_proptest() {
    proptest!(|(val: [u8; 20])| {
        let addr = Address::from(val);
        let alloy = AlloyAddress::from(val).abi_encode();
        let mut buf = vec![0u8; addr.encode_len()];
        addr.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Address::decode(&buf).unwrap(), addr);
    });
}

#[test]
fn encode_decode_bytes32_proptest() {
    proptest!(|(val: [u8; 32])| {
        let alloy = FixedBytes::from(val).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(<[u8; 32]>::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_string_proptest() {
    proptest!(|(val: alloc::string::String)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(alloc::string::String::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_str_proptest() {
    proptest!(|(val: alloc::string::String)| {
        let str_val = val.as_str();
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; str_val.encode_len()];
        str_val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
    });
}

#[test]
fn encode_decode_vec_u256_proptest() {
    proptest!(|(vals in proptest::collection::vec(any::<[u64; 4]>(), 0..8))| {
        let val = vals.into_iter().map(U256::from_limbs).collect::<Vec<_>>();
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Vec::<U256>::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_vec_string_proptest() {
    proptest!(|(val in proptest::collection::vec(any::<alloc::string::String>(), 0..8))| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Vec::<alloc::string::String>::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_vec_address_proptest() {
    proptest!(|(raw in proptest::collection::vec(any::<[u8; 20]>(), 0..8))| {
        let val: Vec<Address> = raw.iter().map(|a| Address::from(*a)).collect();
        let alloy = raw
            .iter()
            .map(|a| AlloyAddress::from(*a))
            .collect::<Vec<_>>()
            .abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Vec::<Address>::decode(&buf).unwrap(), val);
    });
}

// [T; N] and tuples have blanket SolEncode/SolDecode impls.
// The tests below exercise these container types directly.
// ========================================================================
// Fixed array [T; N]
// ========================================================================

#[test]
fn encode_decode_fixed_array_of_primitives() {
    let val = [10u32, 20u32, 30u32];
    let alloy = [10u32, 20u32, 30u32].abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[u32; 3]>::decode(&buf).unwrap(), val);
}

// ========================================================================
// Tuple types
// ========================================================================

#[test]
fn encode_decode_tuple_mixed_types() {
    let val = (42u64, true, Address([0xAB; 20]));
    let alloy = (42u64, true, AlloyAddress::from([0xAB; 20])).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<(u64, bool, Address)>::decode(&buf).unwrap(), val);
}

// ========================================================================
// Dynamic tuples — structs with mixed static/dynamic fields
// ========================================================================

#[test]
fn encode_decode_tuple_u64_string() {
    let val = (42u64, "hello".to_string());
    let alloy = (42u64, "hello".to_string()).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<(u64, alloc::string::String)>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_tuple_string_u64() {
    let val = ("world".to_string(), 99u64);
    let alloy = ("world".to_string(), 99u64).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<(alloc::string::String, u64)>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_tuple_string_string() {
    let val = ("foo".to_string(), "bar".to_string());
    let alloy = ("foo".to_string(), "bar".to_string()).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(
        <(alloc::string::String, alloc::string::String)>::decode(&buf).unwrap(),
        val
    );
}

#[test]
fn encode_decode_tuple_u64_string_bool() {
    let val = (42u64, "hello".to_string(), true);
    let alloy = (42u64, "hello".to_string(), true).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(
        <(u64, alloc::string::String, bool)>::decode(&buf).unwrap(),
        val
    );
}

#[test]
fn encode_decode_tuple_u64_string_proptest() {
    proptest!(|(id: u64, name: alloc::string::String)| {
        let val = (id, name.clone());
        let alloy = (id, name).abi_encode_params();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(<(u64, alloc::string::String)>::decode(&buf).unwrap(), val);
    });
}

// ========================================================================
// SOL_NAME tests for primitives and built-in types
// ========================================================================

#[test]
fn sol_type_name_primitives() {
    assert_eq!(<U256 as SolEncode>::SOL_NAME, "uint256");
    assert_eq!(<u128 as SolEncode>::SOL_NAME, "uint128");
    assert_eq!(<u64 as SolEncode>::SOL_NAME, "uint64");
    assert_eq!(<u32 as SolEncode>::SOL_NAME, "uint32");
    assert_eq!(<u16 as SolEncode>::SOL_NAME, "uint16");
    assert_eq!(<u8 as SolEncode>::SOL_NAME, "uint8");
    assert_eq!(<I256 as SolEncode>::SOL_NAME, "int256");
    assert_eq!(<i128 as SolEncode>::SOL_NAME, "int128");
    assert_eq!(<i64 as SolEncode>::SOL_NAME, "int64");
    assert_eq!(<i32 as SolEncode>::SOL_NAME, "int32");
    assert_eq!(<i16 as SolEncode>::SOL_NAME, "int16");
    assert_eq!(<i8 as SolEncode>::SOL_NAME, "int8");
    assert_eq!(<bool as SolEncode>::SOL_NAME, "bool");
    assert_eq!(<Address as SolEncode>::SOL_NAME, "address");
    assert_eq!(<[u8; 32] as SolEncode>::SOL_NAME, "bytes32");
    assert_eq!(<[u8; 20] as SolEncode>::SOL_NAME, "bytes20");
    assert_eq!(<[u8; 4] as SolEncode>::SOL_NAME, "bytes4");
    // Fixed-size arrays of primitives.
    assert_eq!(<[u64; 3] as SolEncode>::SOL_NAME, "uint64[3]");
    assert_eq!(<[i32; 3] as SolEncode>::SOL_NAME, "int32[3]");
    assert_eq!(<[i128; 2] as SolEncode>::SOL_NAME, "int128[2]");
    assert_eq!(<[I256; 2] as SolEncode>::SOL_NAME, "int256[2]");
}

#[test]
fn sol_type_name_dynamic_types() {
    assert_eq!(<&str as SolEncode>::SOL_NAME, "string");
    assert_eq!(<alloc::string::String as SolEncode>::SOL_NAME, "string");
    assert_eq!(<Vec<Address> as SolEncode>::SOL_NAME, "address[]");
    // Tuples, which may be static or dynamic depending on their elements.
    assert_eq!(
        <(u64, bool, Address) as SolEncode>::SOL_NAME,
        "(uint64,bool,address)"
    );
    assert_eq!(
        <(u64, alloc::string::String) as SolEncode>::SOL_NAME,
        "(uint64,string)"
    );
}

// ========================================================================
// Vec<T> SOL_NAME verification for various element types
// ========================================================================

#[test]
fn vec_sol_name_for_primitive_types() {
    assert_eq!(<Vec<u8> as SolEncode>::SOL_NAME, "uint8[]");
    assert_eq!(<Vec<u16> as SolEncode>::SOL_NAME, "uint16[]");
    assert_eq!(<Vec<u32> as SolEncode>::SOL_NAME, "uint32[]");
    assert_eq!(<Vec<u64> as SolEncode>::SOL_NAME, "uint64[]");
    assert_eq!(<Vec<u128> as SolEncode>::SOL_NAME, "uint128[]");
    assert_eq!(<Vec<U256> as SolEncode>::SOL_NAME, "uint256[]");
    assert_eq!(<Vec<i8> as SolEncode>::SOL_NAME, "int8[]");
    assert_eq!(<Vec<i16> as SolEncode>::SOL_NAME, "int16[]");
    assert_eq!(<Vec<i32> as SolEncode>::SOL_NAME, "int32[]");
    assert_eq!(<Vec<i64> as SolEncode>::SOL_NAME, "int64[]");
    assert_eq!(<Vec<i128> as SolEncode>::SOL_NAME, "int128[]");
    assert_eq!(<Vec<I256> as SolEncode>::SOL_NAME, "int256[]");
    assert_eq!(<Vec<bool> as SolEncode>::SOL_NAME, "bool[]");
    assert_eq!(<Vec<Address> as SolEncode>::SOL_NAME, "address[]");
    assert_eq!(<Vec<[u8; 32]> as SolEncode>::SOL_NAME, "bytes32[]");
    assert_eq!(<[[u64; 2]; 3]>::SOL_NAME, "uint64[2][3]");
    assert_eq!(<[[alloc::string::String; 1]; 2]>::SOL_NAME, "string[1][2]");
}

#[test]
fn vec_sol_name_for_string() {
    assert_eq!(
        <Vec<alloc::string::String> as SolEncode>::SOL_NAME,
        "string[]"
    );
}

// ========================================================================
// Signed integer boundary value tests
// ========================================================================

#[test]
fn encode_decode_signed_boundary_values() {
    // i8 boundaries
    let mut buf = vec![0u8; 32];
    i8::MIN.encode_to(&mut buf);
    assert_eq!(i8::decode(&buf).unwrap(), i8::MIN);
    i8::MAX.encode_to(&mut buf);
    assert_eq!(i8::decode(&buf).unwrap(), i8::MAX);

    // i16 boundaries
    i16::MIN.encode_to(&mut buf);
    assert_eq!(i16::decode(&buf).unwrap(), i16::MIN);
    i16::MAX.encode_to(&mut buf);
    assert_eq!(i16::decode(&buf).unwrap(), i16::MAX);

    // i32 boundaries
    i32::MIN.encode_to(&mut buf);
    assert_eq!(i32::decode(&buf).unwrap(), i32::MIN);
    i32::MAX.encode_to(&mut buf);
    assert_eq!(i32::decode(&buf).unwrap(), i32::MAX);

    // i64 boundaries
    i64::MIN.encode_to(&mut buf);
    assert_eq!(i64::decode(&buf).unwrap(), i64::MIN);
    i64::MAX.encode_to(&mut buf);
    assert_eq!(i64::decode(&buf).unwrap(), i64::MAX);

    // i128 boundaries
    i128::MIN.encode_to(&mut buf);
    assert_eq!(i128::decode(&buf).unwrap(), i128::MIN);
    i128::MAX.encode_to(&mut buf);
    assert_eq!(i128::decode(&buf).unwrap(), i128::MAX);
}

#[test]
fn encode_decode_signed_boundary_values_match_alloy() {
    for val in [i8::MIN, i8::MAX, 0i8, -1i8] {
        let mut buf = vec![0u8; 32];
        val.encode_to(&mut buf);
        assert_eq!(&buf, &val.abi_encode(), "i8 mismatch for {val}");
    }
    for val in [i16::MIN, i16::MAX, 0i16, -1i16] {
        let mut buf = vec![0u8; 32];
        val.encode_to(&mut buf);
        assert_eq!(&buf, &val.abi_encode(), "i16 mismatch for {val}");
    }
    for val in [i32::MIN, i32::MAX, 0i32, -1i32] {
        let mut buf = vec![0u8; 32];
        val.encode_to(&mut buf);
        assert_eq!(&buf, &val.abi_encode(), "i32 mismatch for {val}");
    }
    for val in [i64::MIN, i64::MAX, 0i64, -1i64] {
        let mut buf = vec![0u8; 32];
        val.encode_to(&mut buf);
        assert_eq!(&buf, &val.abi_encode(), "i64 mismatch for {val}");
    }
    for val in [i128::MIN, i128::MAX, 0i128, -1i128] {
        let mut buf = vec![0u8; 32];
        val.encode_to(&mut buf);
        assert_eq!(&buf, &val.abi_encode(), "i128 mismatch for {val}");
    }
}

// ========================================================================
// Unsigned boundary value tests
// ========================================================================

#[test]
fn encode_decode_unsigned_boundary_values() {
    let mut buf = vec![0u8; 32];

    for val in [u8::MIN, u8::MAX] {
        val.encode_to(&mut buf);
        assert_eq!(
            u8::decode(&buf).unwrap(),
            val,
            "u8 roundtrip failed for {val}"
        );
    }
    for val in [u16::MIN, u16::MAX] {
        val.encode_to(&mut buf);
        assert_eq!(
            u16::decode(&buf).unwrap(),
            val,
            "u16 roundtrip failed for {val}"
        );
    }
    for val in [u32::MIN, u32::MAX] {
        val.encode_to(&mut buf);
        assert_eq!(
            u32::decode(&buf).unwrap(),
            val,
            "u32 roundtrip failed for {val}"
        );
    }
    for val in [u64::MIN, u64::MAX] {
        val.encode_to(&mut buf);
        assert_eq!(
            u64::decode(&buf).unwrap(),
            val,
            "u64 roundtrip failed for {val}"
        );
    }
    for val in [u128::MIN, u128::MAX] {
        val.encode_to(&mut buf);
        assert_eq!(
            u128::decode(&buf).unwrap(),
            val,
            "u128 roundtrip failed for {val}"
        );
    }

    let u256_max = U256::MAX;
    u256_max.encode_to(&mut buf);
    assert_eq!(U256::decode(&buf).unwrap(), u256_max);
    let u256_zero = U256::ZERO;
    u256_zero.encode_to(&mut buf);
    assert_eq!(U256::decode(&buf).unwrap(), u256_zero);
}

// ========================================================================
// Empty Vec edge cases
// ========================================================================

#[test]
fn encode_decode_empty_vec_u256() {
    let val: Vec<U256> = vec![];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(Vec::<U256>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_empty_vec_string() {
    let val: Vec<alloc::string::String> = vec![];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(Vec::<alloc::string::String>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_empty_vec_address() {
    let val: Vec<Address> = vec![];
    let alloy_val: Vec<AlloyAddress> = vec![];
    let alloy = alloy_val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(Vec::<Address>::decode(&buf).unwrap(), val);
}

// ========================================================================
// Single-element Vec
// ========================================================================

#[test]
fn encode_decode_single_element_vec() {
    let val = vec![42u64];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(Vec::<u64>::decode(&buf).unwrap(), val);
}

// ========================================================================
// Empty and single-char string edge cases
// ========================================================================

#[test]
fn encode_decode_empty_string() {
    let val = alloc::string::String::new();
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(alloc::string::String::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_single_char_string() {
    let val = alloc::string::String::from("a");
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(alloc::string::String::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_string_exactly_32_bytes() {
    let val = alloc::string::String::from("abcdefghijklmnopqrstuvwxyz012345");
    assert_eq!(val.len(), 32);
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(alloc::string::String::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_string_33_bytes_crosses_padding_boundary() {
    let val = alloc::string::String::from("abcdefghijklmnopqrstuvwxyz0123456");
    assert_eq!(val.len(), 33);
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(alloc::string::String::decode(&buf).unwrap(), val);
}

// ========================================================================
// const_selector cross-validation against known Solidity selectors
// ========================================================================

#[test]
fn const_selector_matches_known_solidity_selectors() {
    // These are well-known ERC-20 selectors verifiable via solc or etherscan
    assert_eq!(
        const_selector("transfer(address,uint256)"),
        [0xa9, 0x05, 0x9c, 0xbb]
    );
    assert_eq!(
        const_selector("balanceOf(address)"),
        [0x70, 0xa0, 0x82, 0x31]
    );
    assert_eq!(const_selector("totalSupply()"), [0x18, 0x16, 0x0d, 0xdd]);
    assert_eq!(
        const_selector("approve(address,uint256)"),
        [0x09, 0x5e, 0xa7, 0xb3]
    );
    assert_eq!(
        const_selector("transferFrom(address,address,uint256)"),
        [0x23, 0xb8, 0x72, 0xdd]
    );
    assert_eq!(
        const_selector("allowance(address,address)"),
        [0xdd, 0x62, 0xed, 0x3e]
    );
}

// ========================================================================
// HEAD_SIZE consistency tests
// ========================================================================

#[test]
fn head_size_matches_encode_len_for_static_types() {
    assert_eq!(<u8 as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<u16 as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<u32 as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<u64 as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<u128 as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<U256 as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<bool as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<Address as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<[u8; 32] as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<[u8; 20] as SolEncode>::HEAD_SIZE, 32);
    assert_eq!(<[u64; 3] as SolEncode>::HEAD_SIZE, 3 * 32);
}

#[test]
fn is_dynamic_flag_correct_for_builtins() {
    // Static types
    const { assert!(!<u8 as SolEncode>::IS_DYNAMIC) };
    const { assert!(!<u64 as SolEncode>::IS_DYNAMIC) };
    const { assert!(!<U256 as SolEncode>::IS_DYNAMIC) };
    const { assert!(!<bool as SolEncode>::IS_DYNAMIC) };
    const { assert!(!<Address as SolEncode>::IS_DYNAMIC) };
    const { assert!(!<[u8; 32] as SolEncode>::IS_DYNAMIC) };

    // Dynamic types
    const { assert!(<alloc::string::String as SolEncode>::IS_DYNAMIC) };
    const { assert!(<&str as SolEncode>::IS_DYNAMIC) };
    const { assert!(<Vec<u64> as SolEncode>::IS_DYNAMIC) };
    const { assert!(<Vec<alloc::string::String> as SolEncode>::IS_DYNAMIC) };
}

// ========================================================================
// Address newtype roundtrip with zero and max
// ========================================================================

#[test]
fn encode_decode_address_zero() {
    let val = Address::ZERO;
    let mut buf = vec![0u8; 32];
    val.encode_to(&mut buf);
    assert_eq!(Address::decode(&buf).unwrap(), val);
    assert_eq!(&buf[..12], &[0u8; 12]); // left-padded with zeros
}

#[test]
fn encode_decode_address_max() {
    let val = Address([0xFF; 20]);
    let alloy = AlloyAddress::from([0xFF; 20]).abi_encode();
    let mut buf = vec![0u8; 32];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[..]);
    assert_eq!(Address::decode(&buf).unwrap(), val);
}

// ========================================================================
// bytes32 edge cases
// ========================================================================

#[test]
fn encode_decode_bytes32_zero() {
    let val = [0u8; 32];
    let mut buf = vec![0u8; 32];
    val.encode_to(&mut buf);
    assert_eq!(<[u8; 32]>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_bytes32_max() {
    let val = [0xFF_u8; 32];
    let alloy = FixedBytes::from(val).abi_encode();
    let mut buf = vec![0u8; 32];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[..]);
    assert_eq!(<[u8; 32]>::decode(&buf).unwrap(), val);
}

#[test]
fn string_decode_at_nonzero_offset() {
    // Encode (u64, String) as a tuple — this produces correct ABI layout
    let val = (42u64, "hello".to_string());
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);

    // Simulate what dispatch codegen does: decode each param with decode_at
    let decoded_u64 = u64::decode_at(&buf, 0).unwrap();
    assert_eq!(decoded_u64, 42u64);

    let decoded_string = alloc::string::String::decode_at(&buf, 32).unwrap();
    assert_eq!(decoded_string, "hello");
}

#[test]
fn bytes_decode_at_nonzero_offset() {
    use super::alloc_types::Bytes;

    let val = (99u64, Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);

    let decoded_u64 = u64::decode_at(&buf, 0).unwrap();
    assert_eq!(decoded_u64, 99u64);

    let decoded_bytes = Bytes::decode_at(&buf, 32).unwrap();
    assert_eq!(decoded_bytes, Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]));
}

#[test]
fn encode_decode_bytes_proptest() {
    use super::alloc_types::Bytes;
    use alloy_core::primitives::Bytes as AlloyBytes;

    proptest!(|(data: alloc::vec::Vec<u8>)| {
        let val = Bytes(data.clone());
        let alloy = AlloyBytes::from(data).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Bytes::decode(&buf).unwrap(), val);
    });
}

#[test]
fn encode_decode_bytes_empty() {
    use super::alloc_types::Bytes;
    use alloy_core::primitives::Bytes as AlloyBytes;

    let val = Bytes(vec![]);
    let alloy = AlloyBytes::from(vec![]).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(Bytes::decode(&buf).unwrap(), val);
}

#[test]
fn vec_decode_at_nonzero_offset() {
    let val = (7u64, vec![U256::from(10), U256::from(20)]);
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);

    let decoded_u64 = u64::decode_at(&buf, 0).unwrap();
    assert_eq!(decoded_u64, 7u64);

    let decoded_vec = Vec::<U256>::decode_at(&buf, 32).unwrap();
    assert_eq!(decoded_vec, vec![U256::from(10), U256::from(20)]);
}

// --- Signed integer array tests ---

#[test]
fn encode_decode_fixed_array_of_signed_integers() {
    use alloy_core::sol_types::SolValue;
    let val: [i32; 3] = [-1, 0, 42];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[i32; 3]>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_fixed_array_of_i64() {
    use alloy_core::sol_types::SolValue;
    let val: [i64; 2] = [i64::MIN, i64::MAX];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[i64; 2]>::decode(&buf).unwrap(), val);
}

// --- Small bytesN tests ---

#[test]
fn encode_decode_bytes1_left_aligned() {
    let val: [u8; 1] = [0xAB];
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    // bytesN is left-aligned: 0xAB followed by 31 zero bytes
    assert_eq!(buf[0], 0xAB);
    assert!(buf[1..32].iter().all(|&b| b == 0));
    assert_eq!(<[u8; 1]>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_bytes4() {
    use alloy_core::primitives::FixedBytes;
    use alloy_core::sol_types::SolValue;
    let val: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
    let alloy = FixedBytes::<4>::from(val).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[u8; 4]>::decode(&buf).unwrap(), val);
}

// --- Nested Vec<Vec<T>> tests ---

#[test]
fn encode_decode_vec_of_vec_u64() {
    let val: Vec<Vec<u64>> = vec![vec![1, 2, 3], vec![4, 5], vec![]];
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(Vec::<Vec<u64>>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_vec_of_vec_empty() {
    let val: Vec<Vec<u64>> = vec![];
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(Vec::<Vec<u64>>::decode(&buf).unwrap(), val);
}

// --- Large tuple tests ---

#[test]
fn encode_decode_tuple_8_static_fields() {
    let val: (u8, u16, u32, u64, u128, bool, u8, u32) = (1, 2, 3, 4, 5, true, 7, 8);
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(
        <(u8, u16, u32, u64, u128, bool, u8, u32)>::decode(&buf).unwrap(),
        val
    );
}

#[test]
fn encode_decode_tuple_mixed_static_dynamic_large() {
    use alloc::string::String;
    let val: (u64, String, bool, String, u32, Address, String, u8) = (
        42,
        "hello".to_string(),
        true,
        "world".to_string(),
        99,
        Address([0xAA; 20]),
        "!".to_string(),
        255,
    );
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(
        <(u64, String, bool, String, u32, Address, String, u8)>::decode(&buf).unwrap(),
        val
    );
}

// --- Bytes decode_at test ---

#[test]
fn bytes_in_tuple_decode_at_nonzero_offset() {
    use super::alloc_types::Bytes;
    let val = (7u64, Bytes(vec![0x01, 0x02, 0x03]));
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);

    let decoded_u64 = u64::decode_at(&buf, 0).unwrap();
    assert_eq!(decoded_u64, 7u64);

    let decoded_bytes = Bytes::decode_at(&buf, 32).unwrap();
    assert_eq!(decoded_bytes, Bytes(vec![0x01, 0x02, 0x03]));
}

// --- ConstStr tests ---

#[test]
fn const_str_new_concatenates() {
    const S: ConstStr = ConstStr::new("hello", " world");
    assert_eq!(S.as_str(), "hello world");
}

#[test]
fn const_str_new_empty_strings() {
    const S: ConstStr = ConstStr::new("", "");
    assert_eq!(S.as_str(), "");
}

#[test]
fn const_str_append() {
    const S: ConstStr = ConstStr::new("(uint64", ",bool)");
    assert_eq!(S.as_str(), "(uint64,bool)");

    const S2: ConstStr = ConstStr::new("foo", "").append("bar");
    assert_eq!(S2.as_str(), "foobar");
}

#[test]
fn const_str_append_usize() {
    const S: ConstStr = ConstStr::new("uint", "").append_usize(256);
    assert_eq!(S.as_str(), "uint256");
}

#[test]
fn const_str_append_usize_zero() {
    const S: ConstStr = ConstStr::new("val", "").append_usize(0);
    assert_eq!(S.as_str(), "val0");
}

#[test]
fn const_str_append_usize_large() {
    const S: ConstStr = ConstStr::new("[", "").append_usize(12345).append("]");
    assert_eq!(S.as_str(), "[12345]");
}

// --- Additional tuple arity tests ---

#[test]
fn encode_decode_tuple_4_fields() {
    let val: (u64, bool, u32, Address) = (1, true, 42, Address([0xBB; 20]));
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(<(u64, bool, u32, Address)>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_tuple_12_fields() {
    let val: (u8, u16, u32, u64, u128, bool, u8, u16, u32, u64, u128, bool) =
        (1, 2, 3, 4, 5, true, 7, 8, 9, 10, 11, false);
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(
        <(u8, u16, u32, u64, u128, bool, u8, u16, u32, u64, u128, bool)>::decode(&buf).unwrap(),
        val
    );
}

// ========================================================================
// Fixed array [T; N] — comprehensive tests
// ========================================================================

#[test]
fn encode_decode_fixed_array_of_vec() {
    // [Vec<u32>; 2] — dynamic element in fixed array
    let val = [vec![1u32, 2, 3], vec![4u32, 5]];
    let alloy = val.clone().abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[Vec<u32>; 2]>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_fixed_array_of_tuples() {
    let val = [(1u64, true), (2u64, false), (3u64, true)];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[(u64, bool); 3]>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_fixed_array_in_tuple() {
    // Tuple containing a fixed array: (u64, [u32; 3])
    let val = (7u64, [10u32, 20, 30]);
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<(u64, [u32; 3])>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_fixed_array_dynamic_in_tuple() {
    // Tuple with dynamic fixed array: (u64, [String; 2])
    let val = (42u64, ["abc".to_string(), "xyz".to_string()]);
    let alloy = (42u64, ["abc".to_string(), "xyz".to_string()]).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(
        <(u64, [alloc::string::String; 2])>::decode(&buf).unwrap(),
        val
    );
}

// ========================================================================
// Deeply nested mixed static/dynamic tuple encoding
// ========================================================================

#[test]
fn encode_decode_tuple_nested_with_dynamic_arrays() {
    // (uint64, string[2], (uint64, string[2]))
    type T = (
        u64,
        [alloc::string::String; 2],
        (u64, [alloc::string::String; 2]),
    );

    let val: T = (
        8u64,
        ["hello".to_string(), "world".to_string()],
        (7u64, ["foo".to_string(), "bar".to_string()]),
    );
    let alloy = val.clone().abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(T::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_tuple_static_and_dynamic_interleaved() {
    // (bool, string, uint64, string, address)
    // Tests: alternating static/dynamic fields in a 5-tuple
    type T = (
        bool,
        alloc::string::String,
        u64,
        alloc::string::String,
        Address,
    );

    let val: T = (
        true,
        "hello".to_string(),
        42u64,
        "world".to_string(),
        Address([0xAA; 20]),
    );
    let alloy = (
        true,
        "hello".to_string(),
        42u64,
        "world".to_string(),
        AlloyAddress::from([0xAA; 20]),
    )
        .abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(T::decode(&buf).unwrap(), val);
}

// ========================================================================
// Nested fixed arrays — [T; N] as element of [_; M]
// Requires [T; N]: SolArrayElement to compile.
// ========================================================================

#[test]
fn encode_decode_nested_fixed_array_static() {
    // uint64[2][3] — static nested fixed array
    let val: [[u64; 2]; 3] = [[1, 2], [3, 4], [5, 6]];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[[u64; 2]; 3]>::decode(&buf).unwrap(), val);
}

#[test]
fn encode_decode_nested_fixed_array_dynamic() {
    // string[1][2] — dynamic nested fixed array
    let val: [[alloc::string::String; 1]; 2] = [["alpha".to_string()], ["beta".to_string()]];
    let alloy = val.clone().abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    // alloy wraps dynamic types with a top-level offset prefix
    assert_eq!(&buf, &alloy);
    assert_eq!(
        <[[alloc::string::String; 1]; 2]>::decode(&buf).unwrap(),
        val
    );
}

// ========================================================================
// encode_to — smart top-level ABI encoding
//
// For non-tuples: enc((T)) — matches alloy's (val,).abi_encode_params().
// Dynamic non-tuples get a 32-byte offset prefix, static pass through.
// For tuples (IS_TUPLE=true): flat body — matches alloy's tuple.abi_encode_params().
// ========================================================================

#[test]
fn encode_to_string() {
    let val = "hello".to_string();
    let alloy = (val.clone(),).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
}

#[test]
fn encode_to_vec() {
    let val = vec![1u64, 2, 3];
    let alloy = (val.clone(),).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
}

#[test]
fn encode_to_dynamic_tuple() {
    // IS_TUPLE=true: encode_to produces flat tuple body (multi-return)
    let val = (7u64, "test".to_string());
    let alloy = (7u64, "test".to_string()).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
}

#[test]
fn encode_to_dynamic_fixed_array() {
    let val = ["foo".to_string(), "bar".to_string()];
    let alloy = (val.clone(),).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
}

// ========================================================================
// Return value encoding — verified against alloy sol! function signatures
//
// These tests define actual Solidity function signatures using alloy's
// sol! macro and compare our encode_to output with alloy's abi_encode_returns.
// This tests the full ABI return encoding pipeline, not just generic encoding.
// ========================================================================

use alloy_core::sol_types::SolCall;

alloy_core::sol! {
    function getStaticValue() external view returns (uint64);
    function getString() external view returns (string);
    function getMultiReturn() external view returns (uint64, string);
    function getArray() external view returns (string[2]);
    function getVec() external view returns (uint64[]);

    // Multiple returns with mixed types
    function getMixed() external view returns (bool, string, uint64);

    // Array return
    function getFixedArray() external view returns (uint64[3]);

    // Dynamic array in multi-return
    function getIdAndTags() external view returns (uint64, string[]);

    // Static struct in multi-return
    struct SolPoint {
        uint64 x;
        uint64 y;
    }
    function getPointAndFlag() external view returns (SolPoint, bool);

    // Multiple dynamic returns
    function getTwoStrings() external view returns (string, string);
}

#[test]
fn return_encoding_static_value() {
    // function getStaticValue() returns (uint64)
    let our = {
        let val = 42u64;
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getStaticValueCall::abi_encode_returns(&42u64);
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_string() {
    // function getString() returns (string)
    let our = {
        let val = "hello".to_string();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getStringCall::abi_encode_returns(&"hello".to_string());
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_multi_return() {
    // function getMultiReturn() returns (uint64, string)
    // Macro flattens this into a tuple — our encode_to with IS_TUPLE=true
    let our = {
        let val = (42u64, "hello".to_string());
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getMultiReturnCall::abi_encode_returns(&getMultiReturnReturn {
        _0: 42u64,
        _1: "hello".to_string(),
    });
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_static_fixed_array() {
    // returns (uint64[3]) — single static fixed array
    let val = [10u64, 20, 30];
    let our = {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getFixedArrayCall::abi_encode_returns(&[10u64, 20, 30]);
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_dynamic_vec_in_multi_return() {
    // returns (uint64, string[]) — static + dynamic vec as multi-return
    let val = (42u64, vec!["a".to_string(), "b".to_string()]);
    let our = {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getIdAndTagsCall::abi_encode_returns(&getIdAndTagsReturn {
        _0: 42,
        _1: vec!["a".to_string(), "b".to_string()],
    });
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_static_struct_in_multi_return() {
    // returns (SolPoint, bool) — static struct + bool as multi-return
    let val = ((5u64, 10u64), true);
    let our = {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getPointAndFlagCall::abi_encode_returns(&getPointAndFlagReturn {
        _0: SolPoint { x: 5, y: 10 },
        _1: true,
    });
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_two_strings() {
    // returns (string, string) — multiple dynamic returns
    let val = ("foo".to_string(), "bar".to_string());
    let our = {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getTwoStringsCall::abi_encode_returns(&getTwoStringsReturn {
        _0: "foo".to_string(),
        _1: "bar".to_string(),
    });
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_mixed_multi_return() {
    // returns (bool, string, uint64) — 3 returns with mixed static/dynamic
    let val = (true, "hello".to_string(), 99u64);
    let our = {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getMixedCall::abi_encode_returns(&getMixedReturn {
        _0: true,
        _1: "hello".to_string(),
        _2: 99,
    });
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_vec() {
    // function getVec() returns (uint64[])
    let our = {
        let val = vec![1u64, 2, 3];
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getVecCall::abi_encode_returns(&vec![1u64, 2, 3]);
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_roundtrip() {
    // Verify decode(encode_to(val)) == val for all return types
    let s = "hello".to_string();
    let mut buf = vec![0u8; s.encode_len()];
    s.encode_to(&mut buf);
    assert_eq!(alloc::string::String::decode(&buf).unwrap(), s);

    let v = vec![1u64, 2, 3];
    let mut buf = vec![0u8; v.encode_len()];
    v.encode_to(&mut buf);
    assert_eq!(Vec::<u64>::decode(&buf).unwrap(), v);

    let t = (42u64, "world".to_string());
    let mut buf = vec![0u8; t.encode_len()];
    t.encode_to(&mut buf);
    assert_eq!(<(u64, alloc::string::String)>::decode(&buf).unwrap(), t);
}

// ---------------------------------------------------------------------------
// Error encoding tests (SolError, SolRevert, Panic, RevertString)
// ---------------------------------------------------------------------------

#[test]
fn revert_string_selector_is_correct() {
    // keccak256("Error(string)")[0:4] = 0x08c379a0
    assert_eq!(RevertString::SELECTOR, [0x08, 0xc3, 0x79, 0xa0]);
    assert_eq!(RevertString::SELECTOR, const_selector("Error(string)"));
}

#[test]
fn revert_string_encoding_matches_solidity() {
    let error = RevertString("insufficient balance");
    let mut buf = [0u8; 256];
    let len = error.revert_data(&mut buf);
    let encoded = &buf[..len];

    // Selector
    assert_eq!(&encoded[0..4], &[0x08, 0xc3, 0x79, 0xa0]);

    // Cross-validate with alloy (alloy prepends "revert: " to decoded strings)
    let decoded = alloy_core::sol_types::decode_revert_reason(encoded);
    assert_eq!(decoded, Some("revert: insufficient balance".to_string()));
}

#[test]
fn revert_string_empty() {
    let error = RevertString("");
    let mut buf = [0u8; 256];
    let params_len = error.encode_params(&mut buf);
    // offset(32) + length(32) + no padded data = 64 bytes
    assert_eq!(params_len, 64);
    // length field should be 0
    assert_eq!(
        u64::from_be_bytes(buf[32 + 24..32 + 32].try_into().unwrap()),
        0
    );
}

#[test]
fn revert_string_exact_32_bytes() {
    let msg = "abcdefghijklmnopqrstuvwxyz012345"; // 32 chars
    let error = RevertString(msg);
    let mut buf = [0u8; 256];
    let params_len = error.encode_params(&mut buf);
    // offset(32) + length(32) + data(32, no padding needed) = 96
    assert_eq!(params_len, 96);
}

#[test]
fn revert_string_33_bytes_pads_to_64() {
    let msg = "abcdefghijklmnopqrstuvwxyz0123456"; // 33 chars
    let error = RevertString(msg);
    let mut buf = [0u8; 256];
    let params_len = error.encode_params(&mut buf);
    // offset(32) + length(32) + data(64, padded from 33 to 64) = 128
    assert_eq!(params_len, 128);
    // Padding bytes must be zero
    assert!(buf[64 + 33..64 + 64].iter().all(|&b| b == 0));
}

#[test]
fn revert_string_encoded_size_matches_encode_params() {
    for msg in ["", "x", "hello world", "abcdefghijklmnopqrstuvwxyz012345"] {
        let error = RevertString(msg);
        let mut buf = [0u8; 256];
        let params_len = error.encode_params(&mut buf);
        assert_eq!(error.encoded_size(), 4 + params_len);
    }
}

#[test]
fn panic_selector_is_correct() {
    // keccak256("Panic(uint256)")[0:4] = 0x4e487b71
    assert_eq!(Panic::SELECTOR, [0x4e, 0x48, 0x7b, 0x71]);
    assert_eq!(Panic::SELECTOR, const_selector("Panic(uint256)"));
}

#[test]
fn panic_overflow_encoding() {
    let error = Panic::Overflow;
    let mut buf = [0u8; 256];
    let len = error.revert_data(&mut buf);
    let encoded = &buf[..len];

    assert_eq!(len, 36); // 4 selector + 32 uint256
    assert_eq!(&encoded[0..4], &Panic::SELECTOR);
    // Panic code 0x11, big-endian in 32 bytes
    assert!(encoded[4..35].iter().all(|&b| b == 0));
    assert_eq!(encoded[35], 0x11);
}

#[test]
fn panic_division_by_zero_encoding() {
    let error = Panic::DivisionByZero;
    let mut buf = [0u8; 256];
    let len = error.revert_data(&mut buf);
    let encoded = &buf[..len];

    assert_eq!(len, 36);
    assert_eq!(encoded[35], 0x12);
}

#[test]
fn panic_encoded_size_matches() {
    assert_eq!(Panic::Overflow.encoded_size(), 36);
    assert_eq!(Panic::DivisionByZero.encoded_size(), 36);
}

#[test]
fn sol_default_error_from_panic() {
    let err: SolDefaultError = Panic::Overflow.into();
    let mut buf = [0u8; 256];
    let len = err.revert_data(&mut buf);
    assert_eq!(&buf[0..4], &Panic::SELECTOR);
    assert_eq!(len, 36);
}

#[test]
fn sol_default_error_from_revert_string() {
    let err: SolDefaultError = RevertString("fail").into();
    let mut buf = [0u8; 256];
    let len = err.revert_data(&mut buf);
    assert_eq!(&buf[0..4], &RevertString::SELECTOR);
    assert!(len > 4);

    let decoded = alloy_core::sol_types::decode_revert_reason(&buf[..len]);
    assert_eq!(decoded, Some("revert: fail".to_string()));
}

#[test]
fn sol_revert_enum_dispatches_correctly() {
    struct ErrA;
    impl SolError for ErrA {
        const SELECTOR: [u8; 4] = [0xAA, 0, 0, 0];
        const SIGNATURE: &'static str = "ErrA()";
        fn encode_params(&self, _buf: &mut [u8]) -> usize {
            0
        }
        fn encoded_size(&self) -> usize {
            4
        }
    }

    struct ErrB;
    impl SolError for ErrB {
        const SELECTOR: [u8; 4] = [0xBB, 0, 0, 0];
        const SIGNATURE: &'static str = "ErrB()";
        fn encode_params(&self, _buf: &mut [u8]) -> usize {
            0
        }
        fn encoded_size(&self) -> usize {
            4
        }
    }

    sol_revert_enum! {
        enum TestError {
            A(ErrA),
            B(ErrB),
        }
    }

    let mut buf = [0u8; 256];

    // Custom errors
    let err: TestError = ErrA.into();
    let len = err.revert_data(&mut buf);
    assert_eq!(len, 4);
    assert_eq!(buf[0], 0xAA);

    let err: TestError = ErrB.into();
    let len = err.revert_data(&mut buf);
    assert_eq!(len, 4);
    assert_eq!(buf[0], 0xBB);

    // Auto-injected Panic
    let err: TestError = Panic::Overflow.into();
    let len = err.revert_data(&mut buf);
    assert_eq!(len, 36);
    assert_eq!(&buf[0..4], &Panic::SELECTOR);
    assert_eq!(buf[35], 0x11);

    // Auto-injected RevertString
    let err: TestError = RevertString("fail").into();
    let len = err.revert_data(&mut buf);
    assert_eq!(&buf[0..4], &RevertString::SELECTOR);
    assert!(len > 4);
}

#[test]
fn sol_revert_enum_question_mark_propagation() {
    struct CustomErr;
    impl SolError for CustomErr {
        const SELECTOR: [u8; 4] = [0xCC, 0, 0, 0];
        const SIGNATURE: &'static str = "CustomErr()";
        fn encode_params(&self, _buf: &mut [u8]) -> usize {
            0
        }
        fn encoded_size(&self) -> usize {
            4
        }
    }

    sol_revert_enum! {
        enum MyError {
            Custom(CustomErr),
        }
    }

    // Verify From impls work for ? propagation
    fn returns_custom() -> Result<(), MyError> {
        Err(CustomErr)?
    }
    fn returns_panic() -> Result<(), MyError> {
        Err(Panic::Overflow)?
    }
    fn returns_revert() -> Result<(), MyError> {
        Err(RevertString("nope"))?
    }

    assert!(returns_custom().is_err());
    assert!(returns_panic().is_err());
    assert!(returns_revert().is_err());
}

#[test]
fn revert_string_truncates_long_message() {
    // With a 100-byte buffer: max_data_space = 36, rounded down to 32.
    let long_msg = "a".repeat(200);
    let error = RevertString(&long_msg);
    let mut buf = [0u8; 100];
    let len = error.encode_params(&mut buf);

    // Should not panic, and should fit in buffer
    assert!(len <= 100);

    // Verify the encoded length field matches the truncated string
    let encoded_len = u64::from_be_bytes(buf[32 + 24..32 + 32].try_into().unwrap()) as usize;
    assert!(encoded_len < long_msg.len());
    assert!(encoded_len <= 32); // 100 - 64 = 36, rounded down to 32
}

#[test]
fn revert_string_fits_in_256_byte_revert_buffer() {
    // Simulate the full revert_data path with a 256-byte buffer
    // (4 selector + up to 252 params)
    let msg = "x".repeat(180); // long but should fit
    let error = RevertString(&msg);
    let mut buf = [0u8; 256];
    let len = error.revert_data(&mut buf);
    assert!(len <= 256);
    assert_eq!(&buf[0..4], &RevertString::SELECTOR);

    // Decode with alloy to verify it's valid
    let decoded = alloy_core::sol_types::decode_revert_reason(&buf[..len]);
    assert!(decoded.is_some());
}

#[test]
fn revert_string_very_long_truncates_in_revert_buffer() {
    // A 300-char string must be truncated to fit in 256-byte revert buffer
    let msg = "y".repeat(300);
    let error = RevertString(&msg);
    let mut buf = [0u8; 256];
    let len = error.revert_data(&mut buf);
    assert!(len <= 256);

    // The encoded string length should be less than 300
    let encoded_str_len =
        u64::from_be_bytes(buf[4 + 32 + 24..4 + 32 + 32].try_into().unwrap()) as usize;
    assert!(encoded_str_len < 300);
}

#[test]
fn sol_default_error_question_mark_propagation() {
    fn checked_sub(a: u64, b: u64) -> Result<u64, SolDefaultError> {
        a.checked_sub(b).ok_or(Panic::Overflow.into())
    }

    fn do_transfer(balance: u64, amount: u64) -> Result<u64, SolDefaultError> {
        let new_balance = checked_sub(balance, amount)?;
        Ok(new_balance)
    }

    match do_transfer(100, 50) {
        Ok(val) => assert_eq!(val, 50),
        Err(_) => panic!("expected success"),
    }
    assert!(do_transfer(10, 20).is_err());

    // Verify the error encodes correctly
    match do_transfer(10, 20) {
        Err(err) => {
            let mut buf = [0u8; 256];
            let len = err.revert_data(&mut buf);
            assert_eq!(&buf[0..4], &Panic::SELECTOR);
            assert_eq!(buf[35], 0x11); // Overflow code
            assert_eq!(len, 36);
        }
        Ok(_) => panic!("expected error"),
    }
}

// ========================================================================
// I256 ABI tests — wire-format compatibility with alloy's `int256` encoding.
// ========================================================================

use alloy_core::primitives::I256 as AlloyI256;

#[test]
fn i256_constants_match_alloy() {
    assert_eq!(
        I256::ZERO.to_be_bytes(),
        AlloyI256::ZERO.to_be_bytes::<32>()
    );
    assert_eq!(I256::ONE.to_be_bytes(), AlloyI256::ONE.to_be_bytes::<32>());
    assert_eq!(
        I256::MINUS_ONE.to_be_bytes(),
        AlloyI256::MINUS_ONE.to_be_bytes::<32>()
    );
    assert_eq!(I256::MIN.to_be_bytes(), AlloyI256::MIN.to_be_bytes::<32>());
    assert_eq!(I256::MAX.to_be_bytes(), AlloyI256::MAX.to_be_bytes::<32>());
}

/// Build an `I256` and an `AlloyI256` from the same big-endian bytes so
/// every test encodes the two implementations from identical
/// two's-complement bit patterns.
fn i256_from_bytes(bytes: [u8; 32]) -> (I256, AlloyI256) {
    (I256::from_be_slice(&bytes), AlloyI256::from_be_bytes(bytes))
}

fn i256_limbs_to_bytes(limbs: [u64; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    // Little-endian limbs → big-endian bytes (most significant limb first).
    for (i, limb) in limbs.iter().rev().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&limb.to_be_bytes());
    }
    out
}

#[test]
fn encode_i256() {
    for bytes in [
        [0u8; 32],
        [0xffu8; 32],
        i256_limbs_to_bytes([1, 0, 0, 0]),
        i256_limbs_to_bytes([0, 0, 0, 1u64 << 63]), // I256::MIN
        i256_limbs_to_bytes([u64::MAX, u64::MAX, u64::MAX, u64::MAX >> 1]), // I256::MAX
    ] {
        let (ours, alloy) = i256_from_bytes(bytes);
        let mut buf = vec![0u8; 32];
        ours.encode_to(&mut buf);
        assert_eq!(&buf, &alloy.abi_encode());
    }
}

#[test]
fn encode_decode_i256_proptest() {
    proptest!(|(limbs: [u64; 4])| {
        let bytes = i256_limbs_to_bytes(limbs);
        let (ours, alloy) = i256_from_bytes(bytes);
        let mut buf = vec![0u8; 32];
        ours.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy.abi_encode());
        prop_assert_eq!(I256::decode(&buf).unwrap(), ours);
    });
}

#[test]
fn i256_display_matches_signed_decimal() {
    use alloc::format;
    assert_eq!(format!("{}", I256::ZERO), "0");
    assert_eq!(format!("{}", I256::ONE), "1");
    assert_eq!(format!("{}", I256::MINUS_ONE), "-1");
    assert_eq!(format!("{}", I256::from(-42i32)), "-42");
    assert_eq!(format!("{}", I256::from(i64::MIN)), "-9223372036854775808");
}

// ---------------------------------------------------------------------------
// SolEvent trait tests
// ---------------------------------------------------------------------------

#[test]
fn const_keccak256_matches_keccak256() {
    use alloy_core::primitives::keccak256;

    let sig = "Transfer(address,address,uint256)";
    let expected = keccak256(sig.as_bytes());
    let got = const_keccak256(sig.as_bytes());
    assert_eq!(
        got, expected.0,
        "const_keccak256 should match alloy keccak256"
    );
}

#[test]
fn sol_event_transfer_topics_pack_addresses_correctly() {
    struct Transfer {
        from: Address,
        to: Address,
        _value: U256,
    }

    impl SolEvent for Transfer {
        const TOPIC: [u8; 32] = const_keccak256(b"Transfer(address,address,uint256)");
        const NAME: &'static str = "Transfer";
        const SIGNATURE: &'static str = "Transfer(address,address,uint256)";
        const INDEXED_COUNT: usize = 2;

        fn topics(&self) -> EventTopics {
            let mut from_topic = [0u8; 32];
            from_topic[12..32].copy_from_slice(&self.from.0);

            let mut to_topic = [0u8; 32];
            to_topic[12..32].copy_from_slice(&self.to.0);

            let mut t = EventTopics::new();
            t.push(Self::TOPIC);
            t.push(from_topic);
            t.push(to_topic);
            t
        }

        fn data_len(&self) -> usize {
            32
        }
        fn data_to(&self, buf: &mut [u8]) {
            self._value.encode_to(buf);
        }
    }

    let from = Address([0xAA; 20]);
    let to = Address([0xBB; 20]);
    let event = Transfer {
        from,
        to,
        _value: U256::ZERO,
    };

    let topics = event.topics();
    assert_eq!(topics.len(), 3, "topic0 + 2 indexed");

    // topic0 is signature hash
    assert_eq!(topics[0], Transfer::TOPIC);

    // indexed address is right-aligned: 12 zero bytes + 20 address bytes
    assert_eq!(&topics[1][..12], &[0u8; 12]);
    assert_eq!(&topics[1][12..], &[0xAA; 20]);
    assert_eq!(&topics[2][..12], &[0u8; 12]);
    assert_eq!(&topics[2][12..], &[0xBB; 20]);
}

#[test]
fn i256_from_str_display_round_trip() {
    use alloc::format;
    use core::str::FromStr;
    for val in [
        I256::ZERO,
        I256::ONE,
        I256::MINUS_ONE,
        I256::from(-12345i64),
        I256::MAX,
        I256::MIN + I256::ONE,
    ] {
        let s = format!("{val}");
        assert_eq!(I256::from_str(&s).unwrap(), val, "round-trip for {val}");
    }
}

#[test]
fn non_payable_value_received_selector() {
    assert_eq!(
        framework_errors::NON_PAYABLE_VALUE_RECEIVED,
        const_selector("NonPayableValueReceived()"),
    );
    assert!(framework_errors::NAMES.contains(&"NonPayableValueReceived"));
}

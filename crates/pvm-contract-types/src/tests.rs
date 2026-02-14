#![cfg(feature = "alloc")]

extern crate alloc;

use super::*;
use alloc::vec;
use alloc::vec::Vec;
use alloy_core::primitives::{Address as AlloyAddress, FixedBytes};
use alloy_core::sol_types::SolValue;
use proptest::prelude::*;
use pvm_contract_macros::SolType;

#[test]
fn encode_decode_uint256_proptest() {
    proptest!(|(v: [u64; 4])| {
        let val = U256::from_limbs(v);
        let mut buf = vec![0u8; 32];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &val.abi_encode());
        prop_assert_eq!(U256::decode(&buf), val);
    });
}

#[test]
fn encode_decode_u128_proptest() {
    proptest!(|(val: u128)| {
        let alloy = U256::from(val).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u128::decode(&buf), val);
    });
}

#[test]
fn encode_decode_u64_proptest() {
    proptest!(|(val: u64)| {
        let alloy = U256::from(val).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u64::decode(&buf), val);
    });
}

#[test]
fn encode_decode_u32_proptest() {
    proptest!(|(val: u32)| {
        let alloy = U256::from(val).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u32::decode(&buf), val);
    });
}

#[test]
fn encode_decode_u16_proptest() {
    proptest!(|(val: u16)| {
        let alloy = U256::from(val).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u16::decode(&buf), val);
    });
}

#[test]
fn encode_decode_u8_proptest() {
    proptest!(|(val: u8)| {
        let alloy = U256::from(val).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u8::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i128_proptest() {
    proptest!(|(val: i128)| {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(i128::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i64_proptest() {
    proptest!(|(val: i64)| {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(i64::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i32_proptest() {
    proptest!(|(val: i32)| {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(i32::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i16_proptest() {
    proptest!(|(val: i16)| {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(i16::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i8_proptest() {
    proptest!(|(val: i8)| {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(i8::decode(&buf), val);
    });
}

#[test]
fn encode_decode_signed_negative_values() {
    let val_i128: i128 = -1;
    let mut buf = vec![0u8; 32];
    val_i128.encode_to(&mut buf);
    assert_eq!(&buf[..16], &[0xff; 16]);
    assert_eq!(i128::decode(&buf), val_i128);

    let val_i64: i64 = -1;
    let mut buf = vec![0u8; 32];
    val_i64.encode_to(&mut buf);
    assert_eq!(&buf[..24], &[0xff; 24]);
    assert_eq!(i64::decode(&buf), val_i64);

    let val_i32: i32 = -1;
    let mut buf = vec![0u8; 32];
    val_i32.encode_to(&mut buf);
    assert_eq!(&buf[..28], &[0xff; 28]);
    assert_eq!(i32::decode(&buf), val_i32);

    let val_i16: i16 = -1;
    let mut buf = vec![0u8; 32];
    val_i16.encode_to(&mut buf);
    assert_eq!(&buf[..30], &[0xff; 30]);
    assert_eq!(i16::decode(&buf), val_i16);

    let val_i8: i8 = -1;
    let mut buf = vec![0u8; 32];
    val_i8.encode_to(&mut buf);
    assert_eq!(&buf[..31], &[0xff; 31]);
    assert_eq!(i8::decode(&buf), val_i8);
}

#[test]
fn encode_decode_user_type_static_proptest() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct UserStatic {
        id: u64,
        active: bool,
    }

    proptest!(|(id: u64, active: bool)| {
        let val = UserStatic { id, active };
        let alloy = (id, active).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(UserStatic::decode(&buf), val);
    });
}

#[test]
fn encode_decode_user_type_dynamic_proptest() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct UserDynamic {
        id: u64,
        name: alloc::string::String,
    }

    proptest!(|(id: u64, name: alloc::string::String)| {
        let val = UserDynamic { id, name };
        let alloy = (val.id, val.name.clone()).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy[32..]);
        prop_assert_eq!(UserDynamic::decode(&buf), val);
    });
}

#[test]
fn encode_decode_vector_of_user_type_static_proptest() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct UserStatic {
        id: u64,
        active: bool,
    }

    proptest!(|(users in proptest::collection::vec((any::<u64>(), any::<bool>()), 0..8))| {
        let val = users
            .iter()
            .copied()
            .map(|(id, active)| UserStatic { id, active })
            .collect::<Vec<_>>();
        let alloy = users.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Vec::<UserStatic>::decode(&buf), val);
    });
}

#[test]
fn encode_decode_vector_of_user_type_dynamic_proptest() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct UserDynamic {
        id: u64,
        name: alloc::string::String,
    }

    proptest!(|(users in proptest::collection::vec((any::<u64>(), any::<alloc::string::String>()), 0..8))| {
        let val = users
            .iter()
            .map(|(id, name)| UserDynamic {
                id: *id,
                name: name.clone(),
            })
            .collect::<Vec<_>>();
        let alloy = users.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Vec::<UserDynamic>::decode(&buf), val);
    });
}

#[test]
fn encode_decode_bool_proptest() {
    proptest!(|(val: bool)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(bool::decode(&buf), val);
    });
}

#[test]
fn encode_decode_address_proptest() {
    proptest!(|(val: [u8; 20])| {
        let alloy = AlloyAddress::from(val).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(<[u8; 20]>::decode(&buf), val);
    });
}

#[test]
fn encode_decode_address_newtype_proptest() {
    proptest!(|(val: [u8; 20])| {
        let addr = Address::from(val);
        let alloy = AlloyAddress::from(val).abi_encode();
        let mut buf = vec![0u8; addr.encode_len()];
        addr.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Address::decode(&buf), addr);
    });
}

#[test]
fn encode_decode_bytes32_proptest() {
    proptest!(|(val: [u8; 32])| {
        let alloy = FixedBytes::from(val).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(<[u8; 32]>::decode(&buf), val);
    });
}

#[test]
fn encode_decode_string_proptest() {
    proptest!(|(val: alloc::string::String)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(alloc::string::String::decode(&buf), val);
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
        prop_assert_eq!(Vec::<U256>::decode(&buf), val);
    });
}

#[test]
fn encode_decode_vec_string_proptest() {
    proptest!(|(val in proptest::collection::vec(any::<alloc::string::String>(), 0..8))| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Vec::<alloc::string::String>::decode(&buf), val);
    });
}

#[test]
fn encode_decode_vec_address_proptest() {
    proptest!(|(val in proptest::collection::vec(any::<[u8; 20]>(), 0..8))| {
        let alloy = val
            .iter()
            .map(|a| AlloyAddress::from(*a))
            .collect::<Vec<_>>()
            .abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Vec::<[u8; 20]>::decode(&buf), val);
    });
}

#[cfg(feature = "abi-reflection")]
#[test]
fn sol_type_name_primitives() {
    assert_eq!(<U256 as SolEncode>::sol_name(), "uint256");
    assert_eq!(<u128 as SolEncode>::sol_name(), "uint128");
    assert_eq!(<u64 as SolEncode>::sol_name(), "uint64");
    assert_eq!(<u32 as SolEncode>::sol_name(), "uint32");
    assert_eq!(<u16 as SolEncode>::sol_name(), "uint16");
    assert_eq!(<u8 as SolEncode>::sol_name(), "uint8");
    assert_eq!(<i128 as SolEncode>::sol_name(), "int128");
    assert_eq!(<i64 as SolEncode>::sol_name(), "int64");
    assert_eq!(<i32 as SolEncode>::sol_name(), "int32");
    assert_eq!(<i16 as SolEncode>::sol_name(), "int16");
    assert_eq!(<i8 as SolEncode>::sol_name(), "int8");
    assert_eq!(<bool as SolEncode>::sol_name(), "bool");
    assert_eq!(<[u8; 20] as SolEncode>::sol_name(), "address");
    assert_eq!(<Address as SolEncode>::sol_name(), "address");
    assert_eq!(<[u8; 32] as SolEncode>::sol_name(), "bytes32");
}

#[cfg(feature = "abi-reflection")]
#[test]
fn sol_type_name_dynamic_types() {
    assert_eq!(<&str as SolEncode>::sol_name(), "string");
    assert_eq!(<alloc::string::String as SolEncode>::sol_name(), "string");
    assert_eq!(<Vec<Address> as SolEncode>::sol_name(), "address[]");
}

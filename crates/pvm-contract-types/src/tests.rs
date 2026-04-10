#![cfg(feature = "alloc")]

extern crate alloc;

use super::*;
use alloc::string::ToString;
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
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u128::decode(&buf), val);
    });
}

#[test]
fn encode_decode_u64_proptest() {
    proptest!(|(val: u64)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u64::decode(&buf), val);
    });
}

#[test]
fn encode_decode_u32_proptest() {
    proptest!(|(val: u32)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u32::decode(&buf), val);
    });
}

#[test]
fn encode_decode_u16_proptest() {
    proptest!(|(val: u16)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(u16::decode(&buf), val);
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
        prop_assert_eq!(u8::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i128_proptest() {
    proptest!(|(val: i128)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i128::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i64_proptest() {
    proptest!(|(val: i64)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i64::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i32_proptest() {
    proptest!(|(val: i32)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i32::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i16_proptest() {
    proptest!(|(val: i16)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i16::decode(&buf), val);
    });
}

#[test]
fn encode_decode_i8_proptest() {
    proptest!(|(val: i8)| {
        let alloy = val.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(i8::decode(&buf), val);
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
    assert_eq!(i128::decode(&buf), val_i128);

    let val_i64: i64 = -2;
    let mut buf = vec![0u8; 32];
    val_i64.encode_to(&mut buf);
    assert_eq!(&buf[..24], &[0xff; 24]);
    assert_eq!(&buf[24..32], val_i64.to_be_bytes());
    assert_eq!(i64::decode(&buf), val_i64);

    let val_i32: i32 = -2;
    let mut buf = vec![0u8; 32];
    val_i32.encode_to(&mut buf);
    assert_eq!(&buf[..28], &[0xff; 28]);
    assert_eq!(&buf[28..32], val_i32.to_be_bytes());
    assert_eq!(i32::decode(&buf), val_i32);

    let val_i16: i16 = -2;
    let mut buf = vec![0u8; 32];
    val_i16.encode_to(&mut buf);
    assert_eq!(&buf[..30], &[0xff; 30]);
    assert_eq!(&buf[30..32], val_i16.to_be_bytes());
    assert_eq!(i16::decode(&buf), val_i16);

    let val_i8: i8 = -2;
    let mut buf = vec![0u8; 32];
    val_i8.encode_to(&mut buf);
    assert_eq!(&buf[..31], &[0xff; 31]);
    assert_eq!(&buf[31..32], &val_i8.to_be_bytes());
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
        let addr = Address::from(val);
        let alloy = AlloyAddress::from(val).abi_encode();
        let mut buf = vec![0u8; addr.encode_len()];
        addr.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Address::decode(&buf), addr);
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
        prop_assert_eq!(Vec::<Address>::decode(&buf), val);
    });
}

// ========================================================================
// Nested custom types — struct containing struct
// ========================================================================

#[test]
fn encode_decode_nested_struct_proptest() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Line {
        a: Point,
        b: Point,
    }

    proptest!(|(x1: u64, y1: u64, x2: u64, y2: u64)| {
        let val = Line {
            a: Point { x: x1, y: y1 },
            b: Point { x: x2, y: y2 },
        };
        // Nested struct encodes as nested tuple: ((uint64,uint64),(uint64,uint64))
        let alloy = ((x1, y1), (x2, y2)).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Line::decode(&buf), val);
    });
}

#[test]
fn encode_decode_triple_nested_struct() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Inner {
        val: u32,
    }

    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Middle {
        inner: Inner,
        extra: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Outer {
        mid: Middle,
        flag: bool,
    }

    let val = Outer {
        mid: Middle {
            inner: Inner { val: 42 },
            extra: 100,
        },
        flag: true,
    };
    // Outer = (((uint32), uint64), bool) — nested tuple encoding
    let alloy = ((42u32, 100u64), true).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(Outer::decode(&buf), val);
}

// ========================================================================
// Dynamic struct with custom static field — the NamedPoint pattern
// ========================================================================

#[test]
fn encode_decode_dynamic_struct_with_custom_field_proptest() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct NamedPoint {
        point: Point,
        name: alloc::string::String,
    }

    proptest!(|(x: u64, y: u64, name: alloc::string::String)| {
        let val = NamedPoint {
            point: Point { x, y },
            name: name.clone(),
        };
        // ABI: ((uint64,uint64), string) — dynamic tuple with static nested struct
        let alloy = ((x, y), name).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        // Our encoding omits the top-level tuple offset (32 bytes) that alloy prepends
        // for dynamic tuples, so compare against alloy[32..]
        prop_assert_eq!(&buf, &alloy[32..]);
        prop_assert_eq!(NamedPoint::decode(&buf), val);
    });
}

#[test]
fn encode_decode_dynamic_struct_multiple_dynamic_fields() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Profile {
        id: u64,
        name: alloc::string::String,
        bio: alloc::string::String,
    }

    proptest!(|(id: u64, name: alloc::string::String, bio: alloc::string::String)| {
        let val = Profile { id, name: name.clone(), bio: bio.clone() };
        let alloy = (id, name, bio).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy[32..]);
        prop_assert_eq!(Profile::decode(&buf), val);
    });
}

#[test]
fn encode_decode_dynamic_struct_string_then_static() {
    // String field before static field — tests offset calculation order
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Record {
        label: alloc::string::String,
        value: u64,
    }

    proptest!(|(label: alloc::string::String, value: u64)| {
        let val = Record { label: label.clone(), value };
        let alloy = (label, value).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy[32..]);
        prop_assert_eq!(Record::decode(&buf), val);
    });
}

// ========================================================================
// Many-field static struct — stress test offset tracking
// ========================================================================

#[test]
fn encode_decode_many_field_static_struct() {
    // Uses Address wrapper for address fields (raw [u8; 20] is now uint8[20]).
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Wide {
        a: u8,
        b: u16,
        c: u32,
        d: u64,
        e: u128,
        f: bool,
        g: Address,
    }

    proptest!(|(a: u8, b: u16, c: u32, d: u64, e: u128, f: bool, g: [u8; 20])| {
        let val = Wide {
            a, b, c, d, e, f,
          g: Address(g),
        };
        // alloy uses AlloyAddress (not our Address), so we just verify roundtrip
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(Wide::decode(&buf), val);
        // Cross-validate field count: 7 fields × 32 bytes each = 224
        prop_assert_eq!(buf.len(), 7 * 32);
    });
}

// [T; N] and tuples have blanket SolEncode/SolDecode impls.
// The tests below exercise these container types directly.
// ========================================================================
// Fixed array [T; N]
// ========================================================================

#[test]
fn encode_decode_fixed_array_of_struct() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    let val = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
    let alloy = [(1u64, 2u64), (3u64, 4u64)].abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[Point; 2]>::decode(&buf), val);
}

#[test]
fn encode_decode_fixed_array_of_primitives() {
    let val = [10u32, 20u32, 30u32];
    let alloy = [10u32, 20u32, 30u32].abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[u32; 3]>::decode(&buf), val);
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
    assert_eq!(<(u64, bool, Address)>::decode(&buf), val);
}

#[test]
fn encode_decode_tuple_with_struct() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    let val = (Point { x: 1, y: 2 }, 42u32);
    let alloy = ((1u64, 2u64), 42u32).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<(Point, u32)>::decode(&buf), val);
}

#[test]
fn encode_decode_tuple_struct_and_string() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    let val = (Point { x: 7, y: 13 }, "hello world".to_string());
    let alloy = ((7u64, 13u64), "hello world".to_string()).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    // alloy wraps dynamic tuples with a top-level offset prefix
    assert_eq!(&buf, &alloy[32..]);
    assert_eq!(<(Point, alloc::string::String)>::decode(&buf), val);
}

// ========================================================================
// Dynamic tuples — structs with mixed static/dynamic fields
// ========================================================================

#[test]
fn encode_decode_tuple_u64_string() {
    let val = (42u64, "hello".to_string());
    let alloy = (42u64, "hello".to_string()).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    // alloy wraps dynamic tuples with a top-level offset prefix
    assert_eq!(&buf, &alloy[32..]);
    assert_eq!(<(u64, alloc::string::String)>::decode(&buf), val);
}

#[test]
fn encode_decode_tuple_string_u64() {
    let val = ("world".to_string(), 99u64);
    let alloy = ("world".to_string(), 99u64).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[32..]);
    assert_eq!(<(alloc::string::String, u64)>::decode(&buf), val);
}

#[test]
fn encode_decode_tuple_string_string() {
    let val = ("foo".to_string(), "bar".to_string());
    let alloy = ("foo".to_string(), "bar".to_string()).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[32..]);
    assert_eq!(
        <(alloc::string::String, alloc::string::String)>::decode(&buf),
        val
    );
}

#[test]
fn encode_decode_tuple_u64_string_bool() {
    let val = (42u64, "hello".to_string(), true);
    let alloy = (42u64, "hello".to_string(), true).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[32..]);
    assert_eq!(<(u64, alloc::string::String, bool)>::decode(&buf), val);
}

#[test]
fn encode_decode_tuple_u64_string_proptest() {
    proptest!(|(id: u64, name: alloc::string::String)| {
        let val = (id, name.clone());
        let alloy = (id, name).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy[32..]);
        prop_assert_eq!(<(u64, alloc::string::String)>::decode(&buf), val);
    });
}

// ========================================================================
// Dynamic fixed arrays — struct containing [String; N]
// ========================================================================

#[test]
fn encode_decode_fixed_array_of_strings() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct TwoNames {
        items: [alloc::string::String; 2],
    }

    let val = TwoNames {
        items: ["alpha".to_string(), "beta".to_string()],
    };
    let alloy = (["alpha".to_string(), "beta".to_string()],).abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[32..]);
    assert_eq!(TwoNames::decode(&buf), val);
}

#[test]
fn encode_decode_fixed_array_of_strings_proptest() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct TwoNames {
        items: [alloc::string::String; 2],
    }

    proptest!(|(a: alloc::string::String, b: alloc::string::String)| {
        let val = TwoNames { items: [a.clone(), b.clone()] };
        let alloy = ([a, b],).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy[32..]);
        prop_assert_eq!(TwoNames::decode(&buf), val);
    });
}

// ========================================================================
// Dynamic struct containing [Struct; N] (the "Polygon" pattern)
// ========================================================================

#[test]
fn encode_decode_dynamic_struct_with_fixed_array_of_structs() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Polygon {
        vertices: [Point; 3],
        name: alloc::string::String,
    }

    let val = Polygon {
        vertices: [
            Point { x: 1, y: 2 },
            Point { x: 3, y: 4 },
            Point { x: 5, y: 6 },
        ],
        name: "triangle".to_string(),
    };
    let alloy = (
        [(1u64, 2u64), (3u64, 4u64), (5u64, 6u64)],
        "triangle".to_string(),
    )
        .abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[32..]);
    assert_eq!(Polygon::decode(&buf), val);
}

// ========================================================================
// Vec<Struct> with nested custom types
// ========================================================================

#[test]
fn encode_decode_vec_of_nested_struct_proptest() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Line {
        a: Point,
        b: Point,
    }

    proptest!(|(lines in proptest::collection::vec(
        (any::<u64>(), any::<u64>(), any::<u64>(), any::<u64>()),
        0..4
    ))| {
        let val: Vec<Line> = lines
            .iter()
            .map(|&(x1, y1, x2, y2)| Line {
                a: Point { x: x1, y: y1 },
                b: Point { x: x2, y: y2 },
            })
            .collect();
        let alloy_tuples: Vec<((u64, u64), (u64, u64))> = lines
            .iter()
            .map(|&(x1, y1, x2, y2)| ((x1, y1), (x2, y2)))
            .collect();
        let alloy = alloy_tuples.abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
        prop_assert_eq!(Vec::<Line>::decode(&buf), val);
    });
}

// ========================================================================
// SOL_NAME tests for custom types
// ========================================================================

#[test]
fn sol_type_name_custom_structs() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Line {
        a: Point,
        b: Point,
    }

    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct NamedPoint {
        point: Point,
        name: alloc::string::String,
    }

    assert_eq!(<Point as SolEncode>::SOL_NAME, "(uint64,uint64)");
    assert_eq!(
        <Line as SolEncode>::SOL_NAME,
        "((uint64,uint64),(uint64,uint64))"
    );
    assert_eq!(
        <NamedPoint as SolEncode>::SOL_NAME,
        "((uint64,uint64),string)"
    );
}

#[test]
fn sol_type_name_primitives() {
    assert_eq!(<U256 as SolEncode>::SOL_NAME, "uint256");
    assert_eq!(<u128 as SolEncode>::SOL_NAME, "uint128");
    assert_eq!(<u64 as SolEncode>::SOL_NAME, "uint64");
    assert_eq!(<u32 as SolEncode>::SOL_NAME, "uint32");
    assert_eq!(<u16 as SolEncode>::SOL_NAME, "uint16");
    assert_eq!(<u8 as SolEncode>::SOL_NAME, "uint8");
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
    assert_eq!(<[u64; 3] as SolEncode>::SOL_NAME, "uint64[3]");
}

#[test]
fn sol_type_name_dynamic_types() {
    assert_eq!(<&str as SolEncode>::SOL_NAME, "string");
    assert_eq!(<alloc::string::String as SolEncode>::SOL_NAME, "string");
    assert_eq!(<Vec<Address> as SolEncode>::SOL_NAME, "address[]");
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
    assert_eq!(<Vec<bool> as SolEncode>::SOL_NAME, "bool[]");
    assert_eq!(<Vec<Address> as SolEncode>::SOL_NAME, "address[]");
    assert_eq!(<Vec<[u8; 32]> as SolEncode>::SOL_NAME, "bytes32[]");
}

#[test]
fn vec_sol_name_for_custom_struct() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    assert_eq!(<Vec<Point> as SolEncode>::SOL_NAME, "(uint64,uint64)[]");
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
    assert_eq!(i8::decode(&buf), i8::MIN);
    i8::MAX.encode_to(&mut buf);
    assert_eq!(i8::decode(&buf), i8::MAX);

    // i16 boundaries
    i16::MIN.encode_to(&mut buf);
    assert_eq!(i16::decode(&buf), i16::MIN);
    i16::MAX.encode_to(&mut buf);
    assert_eq!(i16::decode(&buf), i16::MAX);

    // i32 boundaries
    i32::MIN.encode_to(&mut buf);
    assert_eq!(i32::decode(&buf), i32::MIN);
    i32::MAX.encode_to(&mut buf);
    assert_eq!(i32::decode(&buf), i32::MAX);

    // i64 boundaries
    i64::MIN.encode_to(&mut buf);
    assert_eq!(i64::decode(&buf), i64::MIN);
    i64::MAX.encode_to(&mut buf);
    assert_eq!(i64::decode(&buf), i64::MAX);

    // i128 boundaries
    i128::MIN.encode_to(&mut buf);
    assert_eq!(i128::decode(&buf), i128::MIN);
    i128::MAX.encode_to(&mut buf);
    assert_eq!(i128::decode(&buf), i128::MAX);
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
        assert_eq!(u8::decode(&buf), val, "u8 roundtrip failed for {val}");
    }
    for val in [u16::MIN, u16::MAX] {
        val.encode_to(&mut buf);
        assert_eq!(u16::decode(&buf), val, "u16 roundtrip failed for {val}");
    }
    for val in [u32::MIN, u32::MAX] {
        val.encode_to(&mut buf);
        assert_eq!(u32::decode(&buf), val, "u32 roundtrip failed for {val}");
    }
    for val in [u64::MIN, u64::MAX] {
        val.encode_to(&mut buf);
        assert_eq!(u64::decode(&buf), val, "u64 roundtrip failed for {val}");
    }
    for val in [u128::MIN, u128::MAX] {
        val.encode_to(&mut buf);
        assert_eq!(u128::decode(&buf), val, "u128 roundtrip failed for {val}");
    }

    let u256_max = U256::MAX;
    u256_max.encode_to(&mut buf);
    assert_eq!(U256::decode(&buf), u256_max);
    let u256_zero = U256::ZERO;
    u256_zero.encode_to(&mut buf);
    assert_eq!(U256::decode(&buf), u256_zero);
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
    assert_eq!(Vec::<U256>::decode(&buf), val);
}

#[test]
fn encode_decode_empty_vec_string() {
    let val: Vec<alloc::string::String> = vec![];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(Vec::<alloc::string::String>::decode(&buf), val);
}

#[test]
fn encode_decode_empty_vec_address() {
    let val: Vec<Address> = vec![];
    let alloy_val: Vec<AlloyAddress> = vec![];
    let alloy = alloy_val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(Vec::<Address>::decode(&buf), val);
}

#[test]
fn encode_decode_empty_vec_of_struct() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    let val: Vec<Point> = vec![];
    let alloy_val: Vec<(u64, u64)> = vec![];
    let alloy = alloy_val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(Vec::<Point>::decode(&buf), val);
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
    assert_eq!(Vec::<u64>::decode(&buf), val);
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
    assert_eq!(alloc::string::String::decode(&buf), val);
}

#[test]
fn encode_decode_single_char_string() {
    let val = alloc::string::String::from("a");
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(alloc::string::String::decode(&buf), val);
}

#[test]
fn encode_decode_string_exactly_32_bytes() {
    let val = alloc::string::String::from("abcdefghijklmnopqrstuvwxyz012345");
    assert_eq!(val.len(), 32);
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(alloc::string::String::decode(&buf), val);
}

#[test]
fn encode_decode_string_33_bytes_crosses_padding_boundary() {
    let val = alloc::string::String::from("abcdefghijklmnopqrstuvwxyz0123456");
    assert_eq!(val.len(), 33);
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(alloc::string::String::decode(&buf), val);
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
// Struct with Vec field (dynamic struct containing a dynamic collection)
// ========================================================================

#[test]
fn encode_decode_struct_with_vec_field() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct TokenList {
        owner: Address,
        amounts: Vec<U256>,
    }

    let val = TokenList {
        owner: Address([0xAB; 20]),
        amounts: vec![U256::from(100u64), U256::from(200u64)],
    };
    let alloy = (
        AlloyAddress::from([0xAB; 20]),
        vec![U256::from(100u64), U256::from(200u64)],
    )
        .abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[32..]);
    assert_eq!(TokenList::decode(&buf), val);
}

#[test]
fn encode_decode_struct_with_vec_and_string() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Record {
        id: u64,
        tags: Vec<alloc::string::String>,
        note: alloc::string::String,
    }

    let val = Record {
        id: 42,
        tags: vec!["a".to_string(), "bb".to_string()],
        note: "hello".to_string(),
    };
    let alloy = (
        42u64,
        vec!["a".to_string(), "bb".to_string()],
        "hello".to_string(),
    )
        .abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[32..]);
    assert_eq!(Record::decode(&buf), val);
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
fn is_dynamic_flag_correct() {
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

    // Custom structs (static)
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct StaticPair {
        x: u64,
        y: u64,
    }
    const { assert!(!<StaticPair as SolEncode>::IS_DYNAMIC) };

    // Custom structs (dynamic)
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct DynamicRecord {
        id: u64,
        name: alloc::string::String,
    }
    const { assert!(<DynamicRecord as SolEncode>::IS_DYNAMIC) };

    // Nested static struct
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct NestedStatic {
        pair: StaticPair,
        flag: bool,
    }
    const { assert!(!<NestedStatic as SolEncode>::IS_DYNAMIC) };

    // Nested struct with dynamic inner
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct NestedDynamic {
        record: DynamicRecord,
        count: u64,
    }
    const { assert!(<NestedDynamic as SolEncode>::IS_DYNAMIC) };
}

// ========================================================================
// Address newtype roundtrip with zero and max
// ========================================================================

#[test]
fn encode_decode_address_zero() {
    let val = Address::ZERO;
    let mut buf = vec![0u8; 32];
    val.encode_to(&mut buf);
    assert_eq!(Address::decode(&buf), val);
    assert_eq!(&buf[..12], &[0u8; 12]); // left-padded with zeros
}

#[test]
fn encode_decode_address_max() {
    let val = Address([0xFF; 20]);
    let alloy = AlloyAddress::from([0xFF; 20]).abi_encode();
    let mut buf = vec![0u8; 32];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[..]);
    assert_eq!(Address::decode(&buf), val);
}

// ========================================================================
// bytes32 edge cases
// ========================================================================

#[test]
fn encode_decode_bytes32_zero() {
    let val = [0u8; 32];
    let mut buf = vec![0u8; 32];
    val.encode_to(&mut buf);
    assert_eq!(<[u8; 32]>::decode(&buf), val);
}

#[test]
fn encode_decode_bytes32_max() {
    let val = [0xFF_u8; 32];
    let alloy = FixedBytes::from(val).abi_encode();
    let mut buf = vec![0u8; 32];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy[..]);
    assert_eq!(<[u8; 32]>::decode(&buf), val);
}

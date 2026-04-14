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
        // Struct encode_to wraps as enc((struct)) — same as alloy's abi_encode on equivalent tuple
        let alloy = (val.id, val.name.clone()).abi_encode();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
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
        prop_assert_eq!(&buf, &alloy);
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
        prop_assert_eq!(&buf, &alloy);
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
        prop_assert_eq!(&buf, &alloy);
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
    let alloy = ((7u64, 13u64), "hello world".to_string()).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<(Point, alloc::string::String)>::decode(&buf), val);
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
    assert_eq!(<(u64, alloc::string::String)>::decode(&buf), val);
}

#[test]
fn encode_decode_tuple_string_u64() {
    let val = ("world".to_string(), 99u64);
    let alloy = ("world".to_string(), 99u64).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<(alloc::string::String, u64)>::decode(&buf), val);
}

#[test]
fn encode_decode_tuple_string_string() {
    let val = ("foo".to_string(), "bar".to_string());
    let alloy = ("foo".to_string(), "bar".to_string()).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(
        <(alloc::string::String, alloc::string::String)>::decode(&buf),
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
    assert_eq!(<(u64, alloc::string::String, bool)>::decode(&buf), val);
}

#[test]
fn encode_decode_tuple_u64_string_proptest() {
    proptest!(|(id: u64, name: alloc::string::String)| {
        let val = (id, name.clone());
        let alloy = (id, name).abi_encode_params();
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        prop_assert_eq!(&buf, &alloy);
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
    assert_eq!(&buf, &alloy);
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
        prop_assert_eq!(&buf, &alloy);
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
    assert_eq!(&buf, &alloy);
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
    assert_eq!(&buf, &alloy);
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
    assert_eq!(&buf, &alloy);
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

#[test]
fn string_decode_at_nonzero_offset() {
    // Encode (u64, String) as a tuple — this produces correct ABI layout
    let val = (42u64, "hello".to_string());
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);

    // Simulate what dispatch codegen does: decode each param with decode_at
    let decoded_u64 = u64::decode_at(&buf, 0);
    assert_eq!(decoded_u64, 42u64);

    let decoded_string = alloc::string::String::decode_at(&buf, 32);
    assert_eq!(decoded_string, "hello");
}

#[test]
fn bytes_decode_at_nonzero_offset() {
    use super::alloc_types::Bytes;

    let val = (99u64, Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);

    let decoded_u64 = u64::decode_at(&buf, 0);
    assert_eq!(decoded_u64, 99u64);

    let decoded_bytes = Bytes::decode_at(&buf, 32);
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
        prop_assert_eq!(Bytes::decode(&buf), val);
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
    assert_eq!(Bytes::decode(&buf), val);
}

#[test]
fn vec_decode_at_nonzero_offset() {
    let val = (7u64, vec![U256::from(10), U256::from(20)]);
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);

    let decoded_u64 = u64::decode_at(&buf, 0);
    assert_eq!(decoded_u64, 7u64);

    let decoded_vec = Vec::<U256>::decode_at(&buf, 32);
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
    assert_eq!(<[i32; 3]>::decode(&buf), val);
}

#[test]
fn encode_decode_fixed_array_of_i64() {
    use alloy_core::sol_types::SolValue;
    let val: [i64; 2] = [i64::MIN, i64::MAX];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[i64; 2]>::decode(&buf), val);
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
    assert_eq!(<[u8; 1]>::decode(&buf), val);
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
    assert_eq!(<[u8; 4]>::decode(&buf), val);
}

// --- Nested Vec<Vec<T>> tests ---

#[test]
fn encode_decode_vec_of_vec_u64() {
    let val: Vec<Vec<u64>> = vec![vec![1, 2, 3], vec![4, 5], vec![]];
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(Vec::<Vec<u64>>::decode(&buf), val);
}

#[test]
fn encode_decode_vec_of_vec_empty() {
    let val: Vec<Vec<u64>> = vec![];
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(Vec::<Vec<u64>>::decode(&buf), val);
}

// --- Large tuple tests ---

#[test]
fn encode_decode_tuple_8_static_fields() {
    let val: (u8, u16, u32, u64, u128, bool, u8, u32) = (1, 2, 3, 4, 5, true, 7, 8);
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(
        <(u8, u16, u32, u64, u128, bool, u8, u32)>::decode(&buf),
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
        <(u64, String, bool, String, u32, Address, String, u8)>::decode(&buf),
        val
    );
}

// --- Bytes in struct ---

#[test]
fn encode_decode_struct_with_bytes_field() {
    use super::alloc_types::Bytes;

    #[derive(Debug, PartialEq, SolType)]
    struct WithBytes {
        id: u64,
        data: Bytes,
    }

    let val = WithBytes {
        id: 42,
        data: Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]),
    };
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(WithBytes::decode(&buf), val);
}

#[test]
fn encode_decode_struct_with_empty_bytes() {
    use super::alloc_types::Bytes;

    #[derive(Debug, PartialEq, SolType)]
    struct WithEmptyBytes {
        id: u64,
        data: Bytes,
    }

    let val = WithEmptyBytes {
        id: 0,
        data: Bytes(vec![]),
    };
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(WithEmptyBytes::decode(&buf), val);
}

// --- Bytes decode_at test ---

#[test]
fn bytes_in_tuple_decode_at_nonzero_offset() {
    use super::alloc_types::Bytes;
    let val = (7u64, Bytes(vec![0x01, 0x02, 0x03]));
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);

    let decoded_u64 = u64::decode_at(&buf, 0);
    assert_eq!(decoded_u64, 7u64);

    let decoded_bytes = Bytes::decode_at(&buf, 32);
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
    assert_eq!(<(u64, bool, u32, Address)>::decode(&buf), val);
}

#[test]
fn encode_decode_tuple_12_fields() {
    let val: (u8, u16, u32, u64, u128, bool, u8, u16, u32, u64, u128, bool) =
        (1, 2, 3, 4, 5, true, 7, 8, 9, 10, 11, false);
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(
        <(u8, u16, u32, u64, u128, bool, u8, u16, u32, u64, u128, bool)>::decode(&buf),
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
    assert_eq!(<[Vec<u32>; 2]>::decode(&buf), val);
}

#[test]
fn encode_decode_fixed_array_of_tuples() {
    let val = [(1u64, true), (2u64, false), (3u64, true)];
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<[(u64, bool); 3]>::decode(&buf), val);
}

#[test]
fn encode_decode_fixed_array_in_tuple() {
    // Tuple containing a fixed array: (u64, [u32; 3])
    let val = (7u64, [10u32, 20, 30]);
    let alloy = val.abi_encode();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<(u64, [u32; 3])>::decode(&buf), val);
}

#[test]
fn encode_decode_fixed_array_dynamic_in_tuple() {
    // Tuple with dynamic fixed array: (u64, [String; 2])
    let val = (42u64, ["abc".to_string(), "xyz".to_string()]);
    let alloy = (42u64, ["abc".to_string(), "xyz".to_string()]).abi_encode_params();
    let mut buf = vec![0u8; val.encode_len()];
    val.encode_to(&mut buf);
    assert_eq!(&buf, &alloy);
    assert_eq!(<(u64, [alloc::string::String; 2])>::decode(&buf), val);
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
    assert_eq!(T::decode(&buf), val);
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
    assert_eq!(T::decode(&buf), val);
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
    assert_eq!(<[[u64; 2]; 3]>::decode(&buf), val);
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
    assert_eq!(<[[alloc::string::String; 1]; 2]>::decode(&buf), val);
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
fn encode_to_dynamic_struct() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Profile {
        id: u64,
        name: alloc::string::String,
    }

    let val = Profile {
        id: 42,
        name: "alice".to_string(),
    };
    let alloy = ((42u64, "alice".to_string()),).abi_encode_params();
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

#[test]
fn encode_to_static_struct() {
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    let val = Point { x: 1, y: 2 };
    let alloy = ((1u64, 2u64),).abi_encode_params();
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
    function getStaticStruct() external view returns (uint64, uint64);
    function getDynamicStruct() external view returns (uint64, string);
    function getArray() external view returns (string[2]);
    function getVec() external view returns (uint64[]);
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
fn return_encoding_static_struct() {
    // function getStaticStruct() returns (uint64, uint64)
    // This is multi-return with 2 static values — same as a static struct
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    // As multi-return (tuple)
    let our_tuple = {
        let val = (1u64, 2u64);
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    // As single struct return
    let our_struct = {
        let val = Point { x: 1, y: 2 };
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getStaticStructCall::abi_encode_returns(&getStaticStructReturn { _0: 1, _1: 2 });
    // Multi-return tuple matches alloy
    assert_eq!(our_tuple, alloy);
    // Static struct also matches (no offset for static types)
    assert_eq!(our_struct, alloy);
}

#[test]
fn return_encoding_dynamic_struct() {
    // function getDynamicStruct() returns (uint64, string)
    // As multi-return: tuple with IS_TUPLE=true → flat body
    // As single struct return: IS_TUPLE=false → offset-wrapped
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Profile {
        id: u64,
        name: alloc::string::String,
    }

    let our_tuple = {
        let val = (42u64, "alice".to_string());
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getDynamicStructCall::abi_encode_returns(&getDynamicStructReturn {
        _0: 42,
        _1: "alice".to_string(),
    });
    // Multi-return tuple matches alloy's return encoding
    assert_eq!(our_tuple, alloy);

    // Single struct return is DIFFERENT — has outer offset wrapper
    let our_struct = {
        let val = Profile {
            id: 42,
            name: "alice".to_string(),
        };
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    // Struct wraps: [offset=32][body], which is 32 bytes longer
    assert_eq!(our_struct.len(), alloy.len() + 32);
    // The body after the offset matches the multi-return encoding
    assert_eq!(&our_struct[32..], &alloy);
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
    assert_eq!(alloc::string::String::decode(&buf), s);

    let v = vec![1u64, 2, 3];
    let mut buf = vec![0u8; v.encode_len()];
    v.encode_to(&mut buf);
    assert_eq!(Vec::<u64>::decode(&buf), v);

    let t = (42u64, "world".to_string());
    let mut buf = vec![0u8; t.encode_len()];
    t.encode_to(&mut buf);
    assert_eq!(<(u64, alloc::string::String)>::decode(&buf), t);
}

// ========================================================================
// Advanced return encoding — corner cases with nested types
// ========================================================================

alloy_core::sol! {
    // Struct as single return — wraps with offset
    struct SolProfile {
        uint64 id;
        string name;
    }
    function getProfile() external view returns (SolProfile);

    // Multiple returns with mixed types
    function getMixed() external view returns (bool, string, uint64);

    // Nested struct in multi-return
    struct SolPoint {
        uint64 x;
        uint64 y;
    }
    function getPointAndName() external view returns (SolPoint, string);

    // Array return
    function getFixedArray() external view returns (uint64[3]);

    // Dynamic array in multi-return
    function getIdAndTags() external view returns (uint64, string[]);

    // Static struct in multi-return
    function getPointAndFlag() external view returns (SolPoint, bool);

    // Multiple dynamic returns
    function getTwoStrings() external view returns (string, string);

    // Nested: struct containing dynamic field as single return
    struct SolRecord {
        uint64 id;
        string name;
        bool active;
    }
    function getRecord() external view returns (SolRecord);
}

#[test]
fn return_encoding_single_dynamic_struct() {
    // returns (SolProfile) — single struct return, gets offset wrapper
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Profile {
        id: u64,
        name: alloc::string::String,
    }

    let val = Profile {
        id: 42,
        name: "alice".to_string(),
    };
    let our = {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getProfileCall::abi_encode_returns(&SolProfile {
        id: 42,
        name: "alice".to_string(),
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
fn return_encoding_struct_in_multi_return() {
    // returns (SolPoint, string) — static struct + dynamic string as multi-return
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Point {
        x: u64,
        y: u64,
    }

    // The macro flattens multi-return — but here the struct is one field
    // In Solidity, SolPoint is a tuple (uint64, uint64) inlined in the return
    let val = (Point { x: 1, y: 2 }, "label".to_string());
    let our = {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getPointAndNameCall::abi_encode_returns(&getPointAndNameReturn {
        _0: SolPoint { x: 1, y: 2 },
        _1: "label".to_string(),
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
fn return_encoding_complex_struct() {
    // returns (SolRecord) — struct with 3 fields (static, dynamic, static) as single return
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Record {
        id: u64,
        name: alloc::string::String,
        active: bool,
    }

    let val = Record {
        id: 1,
        name: "test".to_string(),
        active: true,
    };
    let our = {
        let mut buf = vec![0u8; val.encode_len()];
        val.encode_to(&mut buf);
        buf
    };
    let alloy = getRecordCall::abi_encode_returns(&SolRecord {
        id: 1,
        name: "test".to_string(),
        active: true,
    });
    assert_eq!(our, alloy);
}

#[test]
fn return_encoding_roundtrip_advanced() {
    // decode(encode_to(val)) == val for all advanced types
    #[derive(Clone, Debug, PartialEq, Eq, SolType)]
    struct Profile {
        id: u64,
        name: alloc::string::String,
    }

    let p = Profile {
        id: 7,
        name: "bob".to_string(),
    };
    let mut buf = vec![0u8; p.encode_len()];
    p.encode_to(&mut buf);
    assert_eq!(Profile::decode(&buf), p);

    let t = (true, "x".to_string(), 9u64);
    let mut buf = vec![0u8; t.encode_len()];
    t.encode_to(&mut buf);
    assert_eq!(<(bool, alloc::string::String, u64)>::decode(&buf), t);

    let nested = ((1u64, 2u64), "z".to_string());
    let mut buf = vec![0u8; nested.encode_len()];
    nested.encode_to(&mut buf);
    assert_eq!(<((u64, u64), alloc::string::String)>::decode(&buf), nested);
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
        prop_assert_eq!(I256::decode(&buf), ours);
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

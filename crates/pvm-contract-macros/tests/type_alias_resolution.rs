//! Tests for type alias and nested custom type resolution in ABI generation.
//!
//! Validates that trait-based type resolution correctly handles:
//! - Type aliases (`type Count = u64`) — resolved via `SolEncode::SOL_NAME` / `HEAD_SIZE`
//! - Nested custom structs (`Line { a: Point, b: Point }`) — correct offset tracking
//! - Containers with custom types (`[Point; 2]`, `(Count, u32)`) — inline expansion
//! - Mixed structs (concrete + alias + custom fields)

use pvm_contract_sdk::SolType;
use pvm_contract_sdk::U256;
use pvm_contract_sdk::{SolDecode, SolEncode};

// ===== Type aliases =====
type Count = u64;
type TokenAmount = U256;
type Owner = pvm_contract_sdk::Address;
type Coord = u64;

// ===== Structs with type alias fields =====

#[derive(Debug, PartialEq, Eq, SolType)]
struct SimpleAlias {
    count: Count,
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct MultiAlias {
    owner: Owner,
    amount: TokenAmount,
}

// ===== Nested custom types =====

#[derive(Debug, PartialEq, Eq, Clone, Copy, SolType)]
struct Point {
    x: u64,
    y: u64,
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct Line {
    a: Point,
    b: Point,
}

// ===== Nested custom + alias combo =====

#[derive(Debug, PartialEq, Eq, Clone, Copy, SolType)]
struct AliasedPoint {
    x: Coord,
    y: Coord,
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct Triangle {
    a: AliasedPoint,
    b: AliasedPoint,
    c: AliasedPoint,
}

// ===== Mixed: concrete + alias + custom =====

#[derive(Debug, PartialEq, Eq, SolType)]
struct MixedStruct {
    id: u32,
    count: Count,
    origin: Point,
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct NamedPoint {
    point: Point,
    name: String,
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct ArrayAndDynamicCustom {
    points: [Point; 2],
    label: String,
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct TupleCustomDynamic {
    pair: (Point, String),
}

// ========================================================================
// encode_len tests — these catch the head_size=0 bug for Custom types
// ========================================================================

#[test]
fn simple_alias_encode_len() {
    let s = SimpleAlias { count: 42 };
    assert_eq!(s.encode_len(), 32, "one u64 alias field = 32 bytes");
}

#[test]
fn multi_alias_encode_len() {
    let s = MultiAlias {
        owner: pvm_contract_sdk::Address([0xAB; 20]),
        amount: U256::from(1000u64),
    };
    assert_eq!(s.encode_len(), 64, "address + uint256 = 64 bytes");
}

#[test]
fn nested_custom_type_encode_len() {
    let l = Line {
        a: Point { x: 1, y: 2 },
        b: Point { x: 3, y: 4 },
    };
    // Point = (uint64, uint64) = 64 bytes
    // Line = (Point, Point) = 128 bytes
    assert_eq!(l.encode_len(), 128, "two 64-byte Point fields = 128 bytes");
}

#[test]
fn aliased_point_encode_len() {
    let p = AliasedPoint { x: 10, y: 20 };
    assert_eq!(p.encode_len(), 64, "two Coord (= u64) fields = 64 bytes");
}

#[test]
fn triangle_encode_len() {
    let t = Triangle {
        a: AliasedPoint { x: 1, y: 2 },
        b: AliasedPoint { x: 3, y: 4 },
        c: AliasedPoint { x: 5, y: 6 },
    };
    // AliasedPoint = 64 bytes, Triangle = 3 * 64 = 192 bytes
    assert_eq!(
        t.encode_len(),
        192,
        "three 64-byte AliasedPoint fields = 192 bytes"
    );
}

#[test]
fn mixed_struct_encode_len() {
    let s = MixedStruct {
        id: 1,
        count: 42,
        origin: Point { x: 10, y: 20 },
    };
    // u32 = 32, Count(u64) = 32, Point(u64,u64) = 64 → total 128
    assert_eq!(s.encode_len(), 128, "u32 + u64 alias + Point = 128 bytes");
}

// ========================================================================
// Roundtrip encode/decode tests — catch offset bugs
// ========================================================================

#[test]
fn simple_alias_roundtrip() {
    let s = SimpleAlias { count: 42 };
    let len = s.encode_len();
    let mut buf = vec![0u8; len];
    s.encode_to(&mut buf);
    let decoded = SimpleAlias::decode(&buf);
    assert_eq!(decoded, s);
}

#[test]
fn multi_alias_roundtrip() {
    let s = MultiAlias {
        owner: pvm_contract_sdk::Address([0xAB; 20]),
        amount: U256::from(1000u64),
    };
    let len = s.encode_len();
    let mut buf = vec![0u8; len];
    s.encode_to(&mut buf);
    let decoded = MultiAlias::decode(&buf);
    assert_eq!(decoded, s);
}

#[test]
fn nested_custom_type_roundtrip() {
    let l = Line {
        a: Point { x: 1, y: 2 },
        b: Point { x: 3, y: 4 },
    };
    let len = l.encode_len();
    let mut buf = vec![0u8; len];
    l.encode_to(&mut buf);
    let decoded = Line::decode(&buf);
    assert_eq!(decoded, l, "fields a and b must decode to distinct values");
}

#[test]
fn aliased_point_roundtrip() {
    let p = AliasedPoint { x: 10, y: 20 };
    let len = p.encode_len();
    let mut buf = vec![0u8; len];
    p.encode_to(&mut buf);
    let decoded = AliasedPoint::decode(&buf);
    assert_eq!(decoded, p);
}

#[test]
fn triangle_roundtrip() {
    let t = Triangle {
        a: AliasedPoint { x: 1, y: 2 },
        b: AliasedPoint { x: 3, y: 4 },
        c: AliasedPoint { x: 5, y: 6 },
    };
    let len = t.encode_len();
    let mut buf = vec![0u8; len];
    t.encode_to(&mut buf);
    let decoded = Triangle::decode(&buf);
    assert_eq!(decoded, t, "all three vertices must decode distinctly");
}

#[test]
fn mixed_struct_roundtrip() {
    let s = MixedStruct {
        id: 1,
        count: 42,
        origin: Point { x: 10, y: 20 },
    };
    let len = s.encode_len();
    let mut buf = vec![0u8; len];
    s.encode_to(&mut buf);
    let decoded = MixedStruct::decode(&buf);
    assert_eq!(decoded, s);
}

#[test]
fn named_point_roundtrip() {
    let s = NamedPoint {
        point: Point { x: 11, y: 22 },
        name: "alice".to_string(),
    };
    let len = s.encode_len();
    let mut buf = vec![0u8; len];
    s.encode_to(&mut buf);
    let decoded = NamedPoint::decode(&buf);
    assert_eq!(decoded, s);
}

#[test]
fn dynamic_and_custom_array_roundtrip() {
    let s = ArrayAndDynamicCustom {
        points: [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }],
        label: "polyline".to_string(),
    };
    let len = s.encode_len();
    let mut buf = vec![0u8; len];
    s.encode_to(&mut buf);
    let decoded = ArrayAndDynamicCustom::decode(&buf);
    assert_eq!(decoded, s);
}

#[test]
fn dynamic_tuple_with_custom_roundtrip() {
    let s = TupleCustomDynamic {
        pair: (Point { x: 9, y: 10 }, "origin".to_string()),
    };
    let len = s.encode_len();
    let mut buf = vec![0u8; len];
    s.encode_to(&mut buf);
    let decoded = TupleCustomDynamic::decode(&buf);
    assert_eq!(decoded, s);
}

// ========================================================================
// Cross-encoding: custom type encoding must match equivalent tuple encoding
// ========================================================================

#[test]
fn point_encoding_matches_tuple_encoding() {
    let p = Point { x: 100, y: 200 };
    let mut point_buf = vec![0u8; p.encode_len()];
    p.encode_to(&mut point_buf);

    // Manually encode (u64, u64) the same way
    let mut tuple_buf = vec![0u8; 64];
    <u64 as SolEncode>::encode_to(&100u64, &mut tuple_buf[0..32]);
    <u64 as SolEncode>::encode_to(&200u64, &mut tuple_buf[32..64]);

    assert_eq!(
        point_buf, tuple_buf,
        "Point encoding must match (uint64,uint64) tuple encoding"
    );
}

#[test]
fn line_encoding_matches_flat_fields() {
    let l = Line {
        a: Point { x: 1, y: 2 },
        b: Point { x: 3, y: 4 },
    };
    let mut line_buf = vec![0u8; l.encode_len()];
    l.encode_to(&mut line_buf);

    // Line should encode as 4 consecutive u64 values
    let mut flat_buf = vec![0u8; 128];
    <u64 as SolEncode>::encode_to(&1u64, &mut flat_buf[0..32]);
    <u64 as SolEncode>::encode_to(&2u64, &mut flat_buf[32..64]);
    <u64 as SolEncode>::encode_to(&3u64, &mut flat_buf[64..96]);
    <u64 as SolEncode>::encode_to(&4u64, &mut flat_buf[96..128]);

    assert_eq!(
        line_buf, flat_buf,
        "Line encoding must match flat (uint64,uint64,uint64,uint64)"
    );
}

// ========================================================================
// Container-wrapped Custom types — Vec<Alias>, [Custom; N], (Custom, prim)
// ========================================================================

#[derive(Debug, PartialEq, Eq, SolType)]
struct FixedPointArray {
    points: [Point; 2],
}

#[test]
fn fixed_array_of_custom_encode_len() {
    let s = FixedPointArray {
        points: [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }],
    };
    // [Point; 2] = 2 * 64 = 128 bytes
    assert_eq!(s.encode_len(), 128, "[Point; 2] should be 128 bytes");
}

#[test]
fn fixed_array_of_custom_roundtrip() {
    let s = FixedPointArray {
        points: [Point { x: 10, y: 20 }, Point { x: 30, y: 40 }],
    };
    let len = s.encode_len();
    let mut buf = vec![0u8; len];
    s.encode_to(&mut buf);
    let decoded = FixedPointArray::decode(&buf);
    assert_eq!(decoded, s);
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct TupleWithAlias {
    pair: (Count, u32),
}

#[test]
fn tuple_with_alias_encode_len() {
    let s = TupleWithAlias { pair: (42, 7) };
    // (u64, u32) = 64 bytes
    assert_eq!(s.encode_len(), 64, "(Count, u32) should be 64 bytes");
}

#[test]
fn tuple_with_alias_roundtrip() {
    let s = TupleWithAlias { pair: (42, 7) };
    let len = s.encode_len();
    let mut buf = vec![0u8; len];
    s.encode_to(&mut buf);
    let decoded = TupleWithAlias::decode(&buf);
    assert_eq!(decoded, s);
}

// ========================================================================
// sol_name tests — catch the canonical_name string-matching bug
// ========================================================================

mod sol_name_tests {
    use super::*;

    #[test]
    fn simple_alias_sol_name() {
        assert_eq!(<SimpleAlias as SolEncode>::SOL_NAME, "(uint64)");
    }

    #[test]
    fn multi_alias_sol_name() {
        assert_eq!(<MultiAlias as SolEncode>::SOL_NAME, "(address,uint256)");
    }

    #[test]
    fn point_sol_name() {
        assert_eq!(<Point as SolEncode>::SOL_NAME, "(uint64,uint64)");
    }

    #[test]
    fn line_sol_name() {
        assert_eq!(
            <Line as SolEncode>::SOL_NAME,
            "((uint64,uint64),(uint64,uint64))"
        );
    }

    #[test]
    fn aliased_point_sol_name() {
        assert_eq!(<AliasedPoint as SolEncode>::SOL_NAME, "(uint64,uint64)");
    }

    #[test]
    fn triangle_sol_name() {
        assert_eq!(
            <Triangle as SolEncode>::SOL_NAME,
            "((uint64,uint64),(uint64,uint64),(uint64,uint64))"
        );
    }

    #[test]
    fn mixed_struct_sol_name() {
        assert_eq!(
            <MixedStruct as SolEncode>::SOL_NAME,
            "(uint32,uint64,(uint64,uint64))"
        );
    }

    #[test]
    fn named_point_sol_name() {
        assert_eq!(
            <NamedPoint as SolEncode>::SOL_NAME,
            "((uint64,uint64),string)"
        );
    }

    #[test]
    fn array_and_dynamic_custom_sol_name() {
        assert_eq!(
            <ArrayAndDynamicCustom as SolEncode>::SOL_NAME,
            "((uint64,uint64)[2],string)"
        );
    }

    #[test]
    fn tuple_custom_dynamic_sol_name() {
        assert_eq!(
            <TupleCustomDynamic as SolEncode>::SOL_NAME,
            "(((uint64,uint64),string))"
        );
    }
}

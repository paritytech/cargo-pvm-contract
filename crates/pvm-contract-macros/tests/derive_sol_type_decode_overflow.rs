//! Regression tests: `#[derive(SolType)]` decode codegen must guard
//! attacker-controlled dynamic-field offsets with checked arithmetic, matching
//! the hand-written decoders hardened in the `pvm-contract-types` crate.
//!
//! The codegen in `generate_dynamic_field_decode` previously composed
//! `base_offset + field_offset` (read from calldata) with a raw `+`. Under the
//! scaffold's `overflow-checks = false` release profile that wraps silently and
//! aliases reads back into the calldata buffer; under `overflow-checks = true`
//! it panics. Either way it fails open instead of returning `DecodeError`.

use pvm_contract_sdk::Bytes;
use pvm_contract_sdk::SolDecode;
use pvm_contract_sdk::SolType;
use pvm_contract_sdk::U256;

#[derive(Debug, PartialEq, Eq, SolType)]
struct DynStruct {
    data: Bytes,
}

#[derive(Debug, PartialEq, Eq, SolType)]
struct MixedStruct {
    head: U256,
    data: Bytes,
}

fn word_with_u64(v: u64) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[24..32].copy_from_slice(&v.to_be_bytes());
    w
}

#[test]
fn derive_decode_rejects_overflowing_offset_at_zero() {
    // Single head word whose dynamic-field offset pointer is usize::MAX.
    let input = word_with_u64(u64::MAX);
    assert!(DynStruct::decode_at(&input, 0).is_err());
}

#[test]
fn derive_decode_rejects_overflowing_dynamic_offset_in_mixed_struct() {
    // Mirrors `tuple_decode_rejects_overflowing_dynamic_offset` in
    // pvm-contract-types, but exercises the derive codegen.
    let mut input = Vec::new();
    input.extend_from_slice(&word_with_u64(0)); // static U256 head
    input.extend_from_slice(&word_with_u64(u64::MAX)); // dynamic Bytes offset
    assert!(MixedStruct::decode_at(&input, 0).is_err());
}

#[test]
fn derive_decode_rejects_offset_that_wraps_back_in_bounds() {
    // Decode at a NON-ZERO base offset with a field-offset word chosen so
    // `base_offset + field_offset` wraps to a small, in-bounds location holding
    // a valid (empty) Bytes header. The unguarded codegen wrapped to offset 32
    // and returned `Ok(DynStruct { data: Bytes([]) })`; the guarded codegen
    // must fail closed.
    //
    //   word 0 [0..32]  : padding
    //   word 1 [32..64] : aliased Bytes header — length 0 at [56..64]
    //   word 2 [64..96] : struct head at base offset 64; offset pointer at [88..96]
    let base_offset = 64usize;
    let field_offset = (base_offset as u64).wrapping_neg().wrapping_add(32); // 2^64 - 32

    let mut input = vec![0u8; 96];
    input[88..96].copy_from_slice(&field_offset.to_be_bytes());

    assert!(DynStruct::decode_at(&input, base_offset).is_err());
}

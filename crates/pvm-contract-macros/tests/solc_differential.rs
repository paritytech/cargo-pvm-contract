//! Differential storage-layout tests against the real Solidity compiler.
//!
//! Every other storage-layout test in this crate is a *snapshot* test: it
//! compares the macro's emitted `storageLayout` JSON to a hand-authored golden
//! file. Those pin stability, but they can't catch a layout bug that was baked
//! into both the walker and the golden at the same time — the test names say
//! "solc_compatible" but solc is never actually consulted.
//!
//! This file closes that gap. For each fixture it:
//!   1. takes a `#[contract]` module and the *equivalent* Solidity source,
//!   2. runs `solc --standard-json` with `storageLayout` output selection,
//!   3. resolves solc's `type` ids through its `types` table to the human label,
//!   4. normalizes both sides to `{label, slot, offset, type}` (the subset we
//!      emit — we don't emit solc's `astId` / `encoding` / `numberOfBytes`),
//!   5. asserts the two normalized layouts are identical.
//!
//! Gated behind the `solc-tests` feature (which implies `abi-gen`, since we
//! call the macro-emitted `__storage_layout_json()` accessor) because it needs
//! `solc` on PATH. Run with:
//!
//! ```text
//! cargo test -p pvm-contract-macros --features solc-tests
//! ```
//!
//! ## Known divergences (captured as `#[ignore]`d tests)
//!
//! Two shapes do NOT match solc today and are encoded as ignored, currently
//! failing tests at the bottom of this file (`soltype_struct_value_*`,
//! `substruct_*`) with TODOs to flip them on once fixed:
//!
//! - A `#[derive(SolType)]` struct used as a storage value: slots/offsets line
//!   up, but our `type` is the inline tuple name (`(uint64,uint64)`) rather than
//!   a solc `struct ...` type.
//! - A `#[storage]` sub-struct: we flatten its leaves into dotted top-level
//!   entries (`erc20.total_supply`), whereas solc emits a single struct-typed
//!   entry and describes members in its `types` table.
//!
//! Both stem from our `StorageLayoutEntry` format having no `types` table, so it
//! can't represent a Solidity `struct` the way solc does. Until that's
//! addressed, composed layouts also remain covered on the golden-snapshot path
//! in `storage_composition.rs` / `abi_output.rs`.
#![cfg(feature = "solc-tests")]

extern crate alloc;

use std::io::Write;
use std::process::{Command, Stdio};

use pvm_contract_sdk::SolType;
use pvm_contract_sdk::{Address, Bytes, I256, Lazy, Mapping, StorageVec, U256};

// ---------------------------------------------------------------------------
// Fixtures: a `#[contract(no_main)]` module + the equivalent Solidity source.
// Field order is kept identical on both sides so packing lines up slot-for-slot.
// ---------------------------------------------------------------------------

/// Sub-word primitives that solc packs into a shared slot, plus full-slot and
/// multi-slot-pair (`uint128` x2) cases. This is the highest-risk area for
/// encode/decode correctness and previously had zero ground-truth coverage.
#[pvm_contract_macros::contract(no_main)]
mod packed {
    use super::*;

    pub struct Packed {
        pub flag: Lazy<bool>,
        pub small: Lazy<u32>,
        pub who: Lazy<Address>,
        pub total: Lazy<U256>,
        pub lo: Lazy<u128>,
        pub hi: Lazy<u128>,
    }

    impl Packed {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const PACKED_SOL: &str = r#"
pragma solidity ^0.8.26;
contract Packed {
    bool flag;
    uint32 small;
    address who;
    uint256 total;
    uint128 lo;
    uint128 hi;
}
"#;

/// Mappings, including a nested `mapping(K => mapping(K => V))` and a
/// dynamic-value mapping.
#[pvm_contract_macros::contract(no_main)]
mod maps {
    use super::*;

    pub struct Maps {
        pub balances: Mapping<Address, U256>,
        pub allowances: Mapping<Address, Mapping<Address, U256>>,
        pub names: Mapping<U256, alloc::string::String>,
    }

    impl Maps {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const MAPS_SOL: &str = r#"
pragma solidity ^0.8.26;
contract Maps {
    mapping(address => uint256) balances;
    mapping(address => mapping(address => uint256)) allowances;
    mapping(uint256 => string) names;
}
"#;

/// Dynamic value types (`string`, `bytes`) alongside a full-slot static.
#[pvm_contract_macros::contract(no_main)]
mod dyns {
    use super::*;

    pub struct Dyns {
        pub name: Lazy<alloc::string::String>,
        pub blob: Lazy<Bytes>,
        pub total: Lazy<U256>,
    }

    impl Dyns {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const DYNS_SOL: &str = r#"
pragma solidity ^0.8.26;
contract Dyns {
    string name;
    bytes blob;
    uint256 total;
}
"#;

/// Dynamic arrays: flat `T[]`, nested `T[][]`, and a mapping-valued `T[]`.
#[pvm_contract_macros::contract(no_main)]
mod vecs {
    use super::*;

    pub struct Vecs {
        pub numbers: StorageVec<U256>,
        pub accounts: StorageVec<Address>,
        pub matrix: StorageVec<StorageVec<U256>>,
        pub buckets: Mapping<Address, StorageVec<U256>>,
    }

    impl Vecs {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const VECS_SOL: &str = r#"
pragma solidity ^0.8.26;
contract Vecs {
    uint256[] numbers;
    address[] accounts;
    uint256[][] matrix;
    mapping(address => uint256[]) buckets;
}
"#;

/// Signed integers, including packing (solc packs `intN` like `uintN` of the
/// same width) and full-slot `int256`.
#[pvm_contract_macros::contract(no_main)]
mod signed {
    use super::*;

    pub struct Signed {
        pub a: Lazy<i8>,
        pub b: Lazy<i32>,
        pub c: Lazy<i64>,
        pub big: Lazy<I256>,
        pub neg: Lazy<i128>,
    }

    impl Signed {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const SIGNED_SOL: &str = r#"
pragma solidity ^0.8.26;
contract Signed {
    int8 a;
    int32 b;
    int64 c;
    int256 big;
    int128 neg;
}
"#;

/// `bytesN` fixed bytes packed alongside an integer, plus a full-width
/// `bytes32`. (`[u8; N]` maps to `bytesN`.)
#[pvm_contract_macros::contract(no_main)]
mod fixedbytes {
    use super::*;

    pub struct FixedBytes {
        pub sel: Lazy<[u8; 4]>,
        pub tag: Lazy<[u8; 2]>,
        pub small: Lazy<u16>,
        pub hash: Lazy<[u8; 32]>,
    }

    impl FixedBytes {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const FIXEDBYTES_SOL: &str = r#"
pragma solidity ^0.8.26;
contract FixedBytes {
    bytes4 sel;
    bytes2 tag;
    uint16 small;
    bytes32 hash;
}
"#;

/// Fixed arrays as stored values: full-word (`uint256[3]` → 3 slots),
/// sub-word packed (`uint128[4]` → 2 slots), and `address[2]` → 2 slots, with a
/// trailing sentinel to verify the *next* field lands at the right slot
/// (i.e. that element packing matches solc, not one-slot-per-element).
#[pvm_contract_macros::contract(no_main)]
mod arrays {
    use super::*;

    pub struct Arrays {
        pub triple: Lazy<[U256; 3]>,
        pub quad: Lazy<[u128; 4]>,
        pub pair: Lazy<[Address; 2]>,
        pub sentinel: Lazy<U256>,
    }

    impl Arrays {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const ARRAYS_SOL: &str = r#"
pragma solidity ^0.8.26;
contract Arrays {
    uint256[3] triple;
    uint128[4] quad;
    address[2] pair;
    uint256 sentinel;
}
"#;

// ---------------------------------------------------------------------------
// Known divergences — currently FAILING, gated with `#[ignore]`.
//
// These two cases produce a layout that does NOT match solc today. They are
// captured here as ignored tests (rather than silently omitted) so the gap is
// visible and the test flips green the moment the generator is fixed. Run with
// `cargo test -p pvm-contract-macros --features solc-tests -- --ignored` to see
// the current diff.
//
// The shared root cause: our `StorageLayoutEntry` format inlines a flat type
// *string* and has no `types` table, so it cannot represent a Solidity
// `struct` the way solc does (a single entry whose `type` references a struct
// in the `types` table, with the struct's members described there). Fixing
// either case means teaching the generator to emit solc-shaped struct types.
// ---------------------------------------------------------------------------

/// A `#[derive(SolType)]` struct used as a `Lazy` value. The fields/slots line
/// up with solc, but our `type` is the inline tuple name `(uint64,uint64)`
/// whereas solc names it `struct WithStruct.Point`.
#[derive(Clone, Debug, PartialEq, Eq, SolType)]
pub struct Point {
    pub x: u64,
    pub y: u64,
}

#[pvm_contract_macros::contract(no_main)]
mod soltype_value {
    use super::*;

    pub struct WithStruct {
        pub p: Lazy<Point>,
    }

    impl WithStruct {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const SOLTYPE_VALUE_SOL: &str = r#"
pragma solidity ^0.8.26;
contract WithStruct {
    struct Point { uint64 x; uint64 y; }
    Point p;
}
"#;

/// A `#[storage]` sub-struct embedded in a contract. We flatten its leaves into
/// dotted top-level entries (`inner.a`@0, `inner.b`@1); solc emits a SINGLE
/// entry (`inner`@0, type `struct Outer.Inner`) and describes the members in
/// its `types` table. Different entry count and labels → mismatch.
#[pvm_contract_sdk::storage]
pub struct Inner {
    pub a: Lazy<U256>,
    pub b: Lazy<U256>,
}

#[pvm_contract_macros::contract(no_main)]
mod substruct {
    use super::*;

    pub struct Outer {
        pub inner: Inner,
    }

    impl Outer {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const SUBSTRUCT_SOL: &str = r#"
pragma solidity ^0.8.26;
contract Outer {
    struct Inner { uint256 a; uint256 b; }
    Inner inner;
}
"#;

// ---------------------------------------------------------------------------
// Normalized layout entry + helpers
// ---------------------------------------------------------------------------

/// The subset of a storage-layout entry our generator emits. Ordered so two
/// layouts can be compared regardless of source ordering.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NormEntry {
    slot: u64,
    offset: u64,
    label: String,
    ty: String,
}

/// Parse our macro-emitted `__storage_layout_json()` string into sorted
/// normalized entries. Shape: `{"storage":[{"label","slot"(str),"offset"(num),"type"}]}`.
fn ours(json: &str) -> Vec<NormEntry> {
    let v: serde_json::Value = serde_json::from_str(json).expect("our layout json parses");
    let storage = v["storage"].as_array().expect("storage is an array");
    let mut out: Vec<NormEntry> = storage
        .iter()
        .map(|e| NormEntry {
            slot: e["slot"]
                .as_str()
                .expect("slot is a string")
                .parse()
                .expect("slot parses as u64"),
            offset: e["offset"].as_u64().expect("offset is a number"),
            label: e["label"].as_str().expect("label is a string").to_owned(),
            ty: e["type"].as_str().expect("type is a string").to_owned(),
        })
        .collect();
    out.sort();
    out
}

/// Run `solc --standard-json` on `source` and return the named contract's
/// storage layout as sorted normalized entries, resolving each entry's `type`
/// id through the `types` table's `label`.
fn solc(source: &str, contract: &str) -> Vec<NormEntry> {
    let input = serde_json::json!({
        "language": "Solidity",
        "sources": { "C.sol": { "content": source } },
        "settings": { "outputSelection": { "*": { "*": ["storageLayout"] } } }
    });

    let mut child = Command::new("solc")
        .arg("--standard-json")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn solc — is it installed and on PATH?");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait for solc");
    assert!(
        out.status.success(),
        "solc exited non-zero:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("solc output parses as json");

    // Standard-json reports compile errors in an `errors` array with exit 0.
    if let Some(errors) = parsed["errors"].as_array() {
        let fatal: Vec<&str> = errors
            .iter()
            .filter(|e| e["severity"].as_str() == Some("error"))
            .filter_map(|e| e["formattedMessage"].as_str())
            .collect();
        assert!(
            fatal.is_empty(),
            "solc reported errors:\n{}",
            fatal.join("\n")
        );
    }

    let layout = &parsed["contracts"]["C.sol"][contract]["storageLayout"];
    let types = &layout["types"];
    let storage = layout["storage"]
        .as_array()
        .unwrap_or_else(|| panic!("no storageLayout.storage for contract {contract}"));

    let mut out: Vec<NormEntry> = storage
        .iter()
        .map(|e| {
            let type_id = e["type"].as_str().expect("solc entry has a type id");
            let ty = types[type_id]["label"]
                .as_str()
                .unwrap_or_else(|| panic!("solc types table missing label for {type_id}"))
                .to_owned();
            NormEntry {
                slot: e["slot"]
                    .as_str()
                    .expect("solc slot is a string")
                    .parse()
                    .expect("solc slot parses as u64"),
                offset: e["offset"].as_u64().expect("solc offset is a number"),
                label: e["label"].as_str().expect("solc label").to_owned(),
                ty,
            }
        })
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn packed_layout_matches_solc() {
    assert_eq!(
        ours(&packed::__storage_layout_json()),
        solc(PACKED_SOL, "Packed")
    );
}

#[test]
fn mapping_layout_matches_solc() {
    assert_eq!(ours(&maps::__storage_layout_json()), solc(MAPS_SOL, "Maps"));
}

#[test]
fn dynamic_layout_matches_solc() {
    assert_eq!(ours(&dyns::__storage_layout_json()), solc(DYNS_SOL, "Dyns"));
}

#[test]
fn vec_layout_matches_solc() {
    assert_eq!(ours(&vecs::__storage_layout_json()), solc(VECS_SOL, "Vecs"));
}

#[test]
fn signed_int_layout_matches_solc() {
    assert_eq!(
        ours(&signed::__storage_layout_json()),
        solc(SIGNED_SOL, "Signed")
    );
}

#[test]
fn fixed_bytes_layout_matches_solc() {
    assert_eq!(
        ours(&fixedbytes::__storage_layout_json()),
        solc(FIXEDBYTES_SOL, "FixedBytes")
    );
}

#[test]
fn fixed_array_layout_matches_solc() {
    assert_eq!(
        ours(&arrays::__storage_layout_json()),
        solc(ARRAYS_SOL, "Arrays")
    );
}

// --- Known-divergent cases (see the "Known divergences" section above) ------

// TODO(storage-layout solc parity): un-ignore once the layout generator emits
// solc-compatible Solidity `struct` types — i.e. a single entry whose `type`
// names the struct (`struct WithStruct.Point`) and a `types` table describing
// its members — instead of the inline tuple SOL_NAME (`(uint64,uint64)`).
// Currently FAILS only on the `type` field; label/slot/offset already match.
#[test]
#[ignore = "known divergence: SolType struct value emits inline tuple type name, \
            not a solc `struct ...` type — enable after generator fix"]
fn soltype_struct_value_layout_matches_solc() {
    assert_eq!(
        ours(&soltype_value::__storage_layout_json()),
        solc(SOLTYPE_VALUE_SOL, "WithStruct")
    );
}

// TODO(storage-layout solc parity): un-ignore once embedded `#[storage]`
// sub-structs emit a single struct-typed entry (matching solc) instead of
// flattening their leaves into dotted-label entries (`inner.a`, `inner.b`).
// Currently FAILS on entry count and labels (we emit N entries, solc emits 1).
#[test]
#[ignore = "known divergence: #[storage] sub-struct flattens to dotted-label \
            entries, solc emits one struct-typed entry — enable after generator fix"]
fn substruct_layout_matches_solc() {
    assert_eq!(
        ours(&substruct::__storage_layout_json()),
        solc(SUBSTRUCT_SOL, "Outer")
    );
}

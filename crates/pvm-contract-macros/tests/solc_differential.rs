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
//!   3. normalizes both sides into the same [`NormLayout`]: the `storage`
//!      entries as `{label, slot, offset, type}` and every struct entry of the
//!      `types` table as `{label, encoding, numberOfBytes, members}`. Each
//!      `type` id is resolved through the `types` table to its human label, so
//!      the key format (solc's astId-derived `t_struct(Point)6_storage` vs our
//!      counter-derived `t_struct(Point)0_storage`) never affects the
//!      comparison. solc's `astId` / `contract` fields have no equivalent on
//!      our side and are dropped,
//!   4. asserts the two normalized layouts are identical.
//!
//! Because `ours()` and `solc()` both return a [`NormLayout`], every fixture
//! compares the `types` table by construction — a new fixture can't silently
//! skip the struct members / `encoding` / `numberOfBytes` check.
//!
//! Only *struct* entries of `types` are compared. We don't route primitives
//! (`t_uint256`, …) or container types (`t_mapping(..)`, `t_array(..)`)
//! through the table — their `type` string is the human label directly — so
//! the solc entries without a `members` array are filtered out.
//!
//! Gated behind the `solc-tests` feature (which implies `abi-gen`, since we
//! call the macro-emitted `__storage_layout_json()` accessor) because it needs
//! `solc` on PATH. Run with:
//!
//! ```text
//! cargo test -p pvm-contract-macros --features solc-tests
//! ```
#![cfg(feature = "solc-tests")]

extern crate alloc;

use std::io::Write;
use std::process::{Command, Stdio};

use pvm_contract_sdk::{Address, Bytes, I256, Lazy, Mapping, StorageVec, U256};
use pvm_contract_sdk::{SolStorage, SolType};

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
// Struct-typed values. These exercise the `types` table: the storage entry's
// `type` is a key into `types`, whose entry carries the solc-style qualified
// label (`struct Contract.Name`), `encoding`, `numberOfBytes`, and the member
// layout with struct-relative slots.
// ---------------------------------------------------------------------------

/// A `#[derive(SolType, SolStorage)]` value struct. Used directly as a `Lazy`
/// value and as the element of every container shape below.
#[derive(Clone, Debug, PartialEq, Eq, SolType, SolStorage)]
pub struct Point {
    pub x: u64,
    pub y: u64,
}

/// A value struct as a `Lazy` value: one `struct WithStruct.Point` entry with
/// the two packed `uint64` members described in `types`.
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

/// A `#[storage]` sub-struct embedded in a contract. Like solc, we emit a
/// SINGLE entry (`inner`@0, type `struct Outer.Inner`) and describe the
/// members (`a`@0, `b`@1, struct-relative) in the `types` table.
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

/// A value struct inside every container shape: mapping value, dynamic array
/// element, and fixed array element. The containers' own `type` strings must
/// embed the qualified struct label (`mapping(address => struct C.Point)`,
/// `struct C.Point[]`, `struct C.Point[3]`) and the struct must be registered
/// in `types` exactly once. The trailing sentinel catches a wrong slot advance
/// for `Point[3]` (3 slots, one per element — not 1, not 6).
#[pvm_contract_macros::contract(no_main)]
mod struct_containers {
    use super::*;

    pub struct StructContainers {
        pub m: Mapping<Address, Point>,
        pub v: StorageVec<Point>,
        pub a: Lazy<[Point; 3]>,
        pub sentinel: Lazy<U256>,
    }

    impl StructContainers {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const STRUCT_CONTAINERS_SOL: &str = r#"
pragma solidity ^0.8.26;
contract StructContainers {
    struct Point { uint64 x; uint64 y; }
    mapping(address => Point) m;
    Point[] v;
    Point[3] a;
    uint256 sentinel;
}
"#;

/// A value struct with a dynamic (`string`) member followed by packable
/// members. Checks that `numberOfBytes` counts the string's slot, that the
/// members after it pack into the *next* slot with solc's offsets, and that a
/// struct holding a `string` is still `inplace` (only the member's own type
/// is `bytes`-encoded).
#[derive(Clone, Debug, PartialEq, Eq, SolType, SolStorage)]
pub struct Profile {
    pub name: alloc::string::String,
    pub age: u64,
    pub active: bool,
}

#[pvm_contract_macros::contract(no_main)]
mod dynstruct {
    use super::*;

    pub struct WithProfile {
        pub profile: Lazy<Profile>,
        pub sentinel: Lazy<U256>,
    }

    impl WithProfile {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

const DYNSTRUCT_SOL: &str = r#"
pragma solidity ^0.8.26;
contract WithProfile {
    struct Profile { string name; uint64 age; bool active; }
    Profile profile;
    uint256 sentinel;
}
"#;

// ---------------------------------------------------------------------------
// Normalized layout + helpers
// ---------------------------------------------------------------------------

/// The subset of a storage-layout entry our generator emits. Ordered so two
/// layouts can be compared regardless of source ordering. Used both for
/// top-level `storage` entries and for struct `members` inside `types`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NormEntry {
    slot: u64,
    offset: u64,
    label: String,
    ty: String,
}

/// A struct entry of the `types` table, independent of its key: solc keys by
/// astId (`t_struct(Point)6_storage`), we key by a per-contract counter
/// (`t_struct(Point)0_storage`), so entries are compared by `label`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NormType {
    label: String,
    encoding: String,
    number_of_bytes: String,
    members: Vec<NormEntry>,
}

/// Everything a fixture compares: the top-level entries plus every struct
/// described in `types`. Both `ours()` and `solc()` produce one of these, so
/// the `types` table is always part of the assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NormLayout {
    storage: Vec<NormEntry>,
    types: Vec<NormType>,
}

/// Resolve a `type` id through the `types` table to its human label. Falls
/// back to the raw string for types that aren't in the table — on our side
/// every non-struct type (`uint256`, `mapping(address => uint256)`, …) is
/// already the label; on solc's side every type is a key, so a missing entry
/// surfaces as a raw `t_…` id in the assertion diff.
fn resolve_type(types: &serde_json::Value, raw_ty: &str) -> String {
    types
        .get(raw_ty)
        .and_then(|t| t["label"].as_str())
        .map_or_else(|| raw_ty.to_owned(), String::from)
}

/// Normalize one `storage` entry or one struct `member` (same JSON shape).
fn norm_entry(e: &serde_json::Value, types: &serde_json::Value) -> NormEntry {
    NormEntry {
        slot: e["slot"]
            .as_str()
            .expect("slot is a string")
            .parse()
            .expect("slot parses as u64"),
        offset: e["offset"].as_u64().expect("offset is a number"),
        label: e["label"].as_str().expect("label is a string").to_owned(),
        ty: resolve_type(types, e["type"].as_str().expect("type is a string")),
    }
}

/// Normalize a raw `storageLayout` object (ours or solc's) into a
/// [`NormLayout`]. Only `types` entries with a `members` array (structs) are
/// kept; primitive and container entries are dropped since we don't emit
/// them (see the module docs).
fn normalize(layout: &serde_json::Value) -> NormLayout {
    let types = &layout["types"];

    let mut storage: Vec<NormEntry> = layout["storage"]
        .as_array()
        .expect("storageLayout.storage is an array")
        .iter()
        .map(|e| norm_entry(e, types))
        .collect();
    storage.sort();

    let mut struct_types: Vec<NormType> = types
        .as_object()
        .into_iter()
        .flat_map(|obj| obj.values())
        .filter_map(|entry| {
            let mut members: Vec<NormEntry> = entry
                .get("members")?
                .as_array()?
                .iter()
                .map(|m| norm_entry(m, types))
                .collect();
            members.sort();
            Some(NormType {
                label: entry["label"].as_str().expect("type label").to_owned(),
                encoding: entry["encoding"]
                    .as_str()
                    .expect("type encoding")
                    .to_owned(),
                number_of_bytes: entry["numberOfBytes"]
                    .as_str()
                    .expect("type numberOfBytes")
                    .to_owned(),
                members,
            })
        })
        .collect();
    struct_types.sort();

    NormLayout {
        storage,
        types: struct_types,
    }
}

/// Parse our macro-emitted `__storage_layout_json()` string. Shape:
/// `{"storage":[{"label","slot"(str),"offset"(num),"type"}],"types":{..}}`
/// (`types` is omitted entirely when no struct is registered).
fn ours(json: &str) -> NormLayout {
    let v: serde_json::Value = serde_json::from_str(json).expect("our layout json parses");
    normalize(&v)
}

/// Run `solc --standard-json` on `source` and return the raw `storageLayout`
/// object for `contract`.
fn solc_raw(source: &str, contract: &str) -> serde_json::Value {
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
    assert!(
        layout.is_object(),
        "no storageLayout for contract {contract} in solc output"
    );
    layout.clone()
}

/// Run solc on `source` and normalize the named contract's storage layout.
fn solc(source: &str, contract: &str) -> NormLayout {
    normalize(&solc_raw(source, contract))
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

#[test]
fn soltype_struct_value_layout_matches_solc() {
    assert_eq!(
        ours(&soltype_value::__storage_layout_json()),
        solc(SOLTYPE_VALUE_SOL, "WithStruct")
    );
}

#[test]
fn substruct_layout_matches_solc() {
    assert_eq!(
        ours(&substruct::__storage_layout_json()),
        solc(SUBSTRUCT_SOL, "Outer")
    );
}

#[test]
fn struct_containers_layout_matches_solc() {
    assert_eq!(
        ours(&struct_containers::__storage_layout_json()),
        solc(STRUCT_CONTAINERS_SOL, "StructContainers")
    );
}

#[test]
fn dynamic_member_struct_layout_matches_solc() {
    assert_eq!(
        ours(&dynstruct::__storage_layout_json()),
        solc(DYNSTRUCT_SOL, "WithProfile")
    );
}

/// The normalizer must actually see our `types` table: a struct fixture whose
/// `types` went missing would otherwise compare equal to solc on the storage
/// entries alone.
#[test]
fn normalizer_keeps_struct_types() {
    let layout = ours(&struct_containers::__storage_layout_json());
    assert_eq!(
        layout.types,
        vec![NormType {
            label: "struct StructContainers.Point".to_owned(),
            encoding: "inplace".to_owned(),
            number_of_bytes: "32".to_owned(),
            members: vec![
                NormEntry {
                    slot: 0,
                    offset: 0,
                    label: "x".to_owned(),
                    ty: "uint64".to_owned(),
                },
                NormEntry {
                    slot: 0,
                    offset: 8,
                    label: "y".to_owned(),
                    ty: "uint64".to_owned(),
                },
            ],
        }]
    );
}

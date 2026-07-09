//! Storage-representation fixtures + the solc/revm ground-truth harness.
//!
//! Each fixture is a real `#[contract]` whose fields mirror an equivalent
//! Solidity contract, built via the macro-generated `Contract::with_host(mock)`
//! and driven by a `populate()` method. The macro computes the storage layout
//! (auto-numbered, packing sub-word siblings solc-style) exactly as solc does
//! for the `.sol` — so nothing here hand-places slots/offsets. We dump the
//! `MockHost` and compare `{slot -> 32 bytes}` against solc-on-revm.

// Each fixture's `#[constructor] new` is required by the macro but never called
// (`with_host` wires storage without running it), so it reads as dead code.
#![allow(dead_code)]

use std::collections::BTreeMap;

use pvm_contract_types::{MockHost, MockHostBuilder};

use alloy_core::primitives::keccak256;
use revm::context::TxEnv;
use revm::database::{CacheDB, EmptyDB};
use revm::primitives::{Address as RAddr, Bytes as RBytes, TxKind, U256 as RU256};
use revm::state::{AccountInfo, Bytecode};
use revm::{Context, ExecuteCommitEvm, MainBuilder, MainContext};

/// A normalized storage map: 32-byte slot key -> 32-byte value, zero values
/// omitted (SSTORE-of-zero deletes on both sides).
type StorageMap = BTreeMap<[u8; 32], [u8; 32]>;

/// Address the contract code is installed at for revm execution.
const CONTRACT: RAddr = RAddr::new([0x11; 20]);
/// Address that sends the `populate()` transaction.
const CALLER: RAddr = RAddr::new([0x22; 20]);

// ---------------------------------------------------------------------------
// solc + revm ground truth
// ---------------------------------------------------------------------------

/// Compile `source` with solc and return the named contract's deployed
/// (runtime) EVM bytecode.
fn solc_deployed_bytecode(source: &str, contract: &str) -> Vec<u8> {
    let parsed = crate::common::run_solc(source, &["evm.deployedBytecode.object"]);
    let hex = parsed["contracts"]["C.sol"][contract]["evm"]["deployedBytecode"]["object"]
        .as_str()
        .unwrap_or_else(|| panic!("no deployedBytecode for {contract}"));
    hex_decode(hex)
}

fn hex_decode(s: &str) -> Vec<u8> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    assert!(s.len().is_multiple_of(2), "odd-length hex");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

/// Execute the Solidity contract's `populate()` on revm and return its
/// resulting account storage as a normalized map.
fn solc_storage(source: &str, contract: &str) -> StorageMap {
    let code = solc_deployed_bytecode(source, contract);
    let bytecode = Bytecode::new_legacy(RBytes::from(code));

    let mut db = CacheDB::new(EmptyDB::default());
    db.insert_account_info(CONTRACT, AccountInfo::from_bytecode(bytecode));
    db.insert_account_info(
        CALLER,
        AccountInfo {
            balance: RU256::from(1u64) << 100,
            ..Default::default()
        },
    );

    let selector = keccak256(b"populate()")[..4].to_vec();

    let mut evm = Context::mainnet().with_db(db).build_mainnet();
    let result = evm
        .transact_commit(TxEnv {
            caller: CALLER,
            kind: TxKind::Call(CONTRACT),
            data: RBytes::from(selector),
            // EIP-7825 caps tx gas at 2^24; populate() is tiny so this is ample.
            gas_limit: 16_777_216,
            gas_price: 0,
            ..Default::default()
        })
        .expect("revm transact_commit");
    assert!(
        result.is_success(),
        "populate() reverted on revm: {result:?}"
    );

    use revm::context_interface::ContextTr;
    let db = evm.ctx.db();
    let acct = db
        .cache
        .accounts
        .get(&CONTRACT)
        .expect("contract account present after commit");

    let mut map = StorageMap::new();
    for (slot, value) in acct.storage.iter() {
        if *value != RU256::ZERO {
            map.insert(slot.to_be_bytes(), value.to_be_bytes());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// the SDK side
// ---------------------------------------------------------------------------

/// Snapshot the `MockHost`'s storage as a normalized map: 32-byte key -> 32-byte
/// value, zero values omitted (`set_storage_or_clear` already deletes on zero).
fn normalize_mock(mock: &MockHost) -> StorageMap {
    let mut map = StorageMap::new();
    for (k, v) in mock.storage_dump() {
        let val = to_32(&v);
        if val != [0u8; 32] {
            map.insert(to_32(&k), val);
        }
    }
    map
}

/// Big-endian pad to 32 bytes. pvm-storage always writes full 32-byte words, so
/// this is a no-op in practice; the `<= 32` assert guards against a short write
/// rather than silently truncating.
fn to_32(bytes: &[u8]) -> [u8; 32] {
    assert!(bytes.len() <= 32, "storage word longer than 32 bytes");
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(bytes);
    out
}

// ---------------------------------------------------------------------------
// Fixtures — a `#[contract]` + the equivalent Solidity, field order identical.
// ---------------------------------------------------------------------------

use pvm_contract_sdk::{
    Address, Bytes, I256, Lazy, Mapping, SolStorage, SolType, StorageComponent, StorageVec, U256,
};

/// Two distinct 20-byte addresses used across fixtures.
const ADDR_A: [u8; 20] = [0xAA; 20];
const ADDR_B: [u8; 20] = [0xBB; 20];

/// A packed static struct: two `uint128` share one 32-byte slot.
#[derive(Clone, Debug, PartialEq, Eq, SolType, SolStorage)]
pub struct Pair {
    pub lo: u128,
    pub hi: u128,
}

/// A genuinely multi-slot static struct: two `uint256`, two consecutive slots.
#[derive(Clone, Debug, PartialEq, Eq, SolType, SolStorage)]
pub struct Wide {
    pub a: U256,
    pub b: U256,
}

/// Mixed sub-word packing inside one struct slot (`flag`@0, `count`@1, `who`@9).
#[derive(Clone, Debug, PartialEq, Eq, SolType, SolStorage)]
pub struct Mixed {
    pub flag: bool,
    pub count: u64,
    pub who: Address,
}

/// A struct with a trailing dynamic `string` field.
#[derive(Clone, Debug, PartialEq, Eq, SolType, SolStorage)]
pub struct DynS {
    pub head: U256,
    pub tail: String,
}

// --- single full slot ------------------------------------------------------

#[pvm_contract_sdk::contract]
mod single {
    use super::*;
    pub struct Single {
        pub x: Lazy<U256>,
    }
    impl Single {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.x.set(&U256::from(42u64));
        }
    }
}

#[test]
fn uint256_single_slot_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract S { uint256 x; function populate() external { x = 42; } }
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = single::Single::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "S"));
}

// --- field-level packing (bool/u32/address share a slot; u128 pair) --------

#[pvm_contract_sdk::contract]
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
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.flag.set(&true);
            self.small.set(&0x0102_0304u32);
            self.who.set(&Address::from(ADDR_A));
            self.total.set(&U256::from(0x1122_3344_5566_7788u64));
            self.lo.set(&0xAAAA_AAAA_AAAA_AAAAu128);
            self.hi.set(&0xBBBB_BBBB_BBBB_BBBBu128);
        }
    }
}

#[test]
fn packed_fields_match_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Packed {
    bool flag;       // slot 0, offset 0
    uint32 small;    // slot 0, offset 1
    address who;     // slot 0, offset 5
    uint256 total;   // slot 1
    uint128 lo;      // slot 2, offset 0
    uint128 hi;      // slot 2, offset 16
    function populate() external {
        flag  = true;
        small = 0x01020304;
        who   = address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA));
        total = 0x1122334455667788;
        lo    = 0xAAAAAAAAAAAAAAAA;
        hi    = 0xBBBBBBBBBBBBBBBB;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = packed::Packed::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Packed"));
}

// --- mappings (single + nested) --------------------------------------------

#[pvm_contract_sdk::contract]
mod maps {
    use super::*;
    pub struct Maps {
        pub balances: Mapping<Address, U256>,
        pub allowances: Mapping<Address, Mapping<Address, U256>>,
    }
    impl Maps {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.balances
                .insert(&Address::from(ADDR_A), &U256::from(1000u64));
            self.allowances
                .view_mut(&Address::from(ADDR_A))
                .insert(&Address::from(ADDR_B), &U256::from(777u64));
        }
    }
}

#[test]
fn mappings_match_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Maps {
    mapping(address => uint256) balances;                       // slot 0
    mapping(address => mapping(address => uint256)) allowances; // slot 1
    function populate() external {
        balances[address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))] = 1000;
        allowances
            [address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))]
            [address(uint160(0x00BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB))] = 777;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = maps::Maps::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Maps"));
}

// --- dynamic string/bytes (inline + spilled) -------------------------------

#[pvm_contract_sdk::contract]
mod dyns {
    use super::*;
    pub struct Dyns {
        pub short: Lazy<String>,
        pub long: Lazy<String>,
        pub blob: Lazy<Bytes>,
    }
    impl Dyns {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.short.set(&String::from("hello"));
            self.long
                .set(&String::from("abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"));
            self.blob.set(&Bytes(vec![1, 2, 3, 4, 5, 6, 7, 8]));
        }
    }
}

#[test]
fn dynamic_string_bytes_match_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Dyns {
    string shortStr;  // slot 0
    string longStr;   // slot 1  (>= 32 bytes -> spilled)
    bytes blob;       // slot 2
    function populate() external {
        shortStr = "hello";
        longStr  = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEF";
        blob     = hex"0102030405060708";
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = dyns::Dyns::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Dyns"));
}

// --- StorageVec ------------------------------------------------------------

#[pvm_contract_sdk::contract]
mod vecs {
    use super::*;
    pub struct Vecs {
        pub nums: StorageVec<U256>,
        pub addrs: StorageVec<Address>,
    }
    impl Vecs {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            for n in [11u64, 22, 33] {
                self.nums.push(&U256::from(n));
            }
            self.addrs.push(&Address::from(ADDR_A));
            self.addrs.push(&Address::from(ADDR_B));
        }
    }
}

#[test]
fn storage_vec_match_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Vecs {
    uint256[] nums;    // slot 0
    address[] addrs;   // slot 1
    function populate() external {
        nums.push(11); nums.push(22); nums.push(33);
        addrs.push(address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA)));
        addrs.push(address(uint160(0x00BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB)));
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = vecs::Vecs::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Vecs"));
}

// --- fixed arrays (full-word striped + sub-word packed) --------------------

#[pvm_contract_sdk::contract]
mod arrays {
    use super::*;
    pub struct Arrays {
        pub triple: Lazy<[U256; 3]>,
        pub quad: Lazy<[u128; 4]>,
    }
    impl Arrays {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.triple
                .set(&[U256::from(1u64), U256::from(2u64), U256::from(3u64)]);
            self.quad.set(&[0xAu128, 0xB, 0xC, 0xD]);
        }
    }
}

#[test]
fn fixed_arrays_match_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Arrays {
    uint256[3] triple;  // slots 0,1,2
    uint128[4] quad;    // slots 3,4 (packed)
    function populate() external {
        triple[0] = 1; triple[1] = 2; triple[2] = 3;
        quad[0] = 0xA; quad[1] = 0xB; quad[2] = 0xC; quad[3] = 0xD;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = arrays::Arrays::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Arrays"));
}

// --- mapping to a packed struct value --------------------------------------

#[pvm_contract_sdk::contract]
mod map_pair {
    use super::*;
    pub struct MapPair {
        pub m: Mapping<Address, Pair>,
    }
    impl MapPair {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.m.insert(
                &Address::from(ADDR_A),
                &Pair {
                    lo: 0xAAAA_AAAA_AAAA_AAAAu128,
                    hi: 0xBBBB_BBBB_BBBB_BBBBu128,
                },
            );
        }
    }
}

#[test]
fn mapping_to_packed_struct_value_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract MapStruct {
    struct Pair { uint128 lo; uint128 hi; }
    mapping(address => Pair) m;   // slot 0
    function populate() external {
        m[address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))] =
            Pair(0xAAAAAAAAAAAAAAAA, 0xBBBBBBBBBBBBBBBB);
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = map_pair::MapPair::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "MapStruct"));
}

// --- mapping to a multi-slot struct value ----------------------------------

#[pvm_contract_sdk::contract]
mod map_wide {
    use super::*;
    pub struct MapWide {
        pub m: Mapping<Address, Wide>,
    }
    impl MapWide {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.m.insert(
                &Address::from(ADDR_A),
                &Wide {
                    a: U256::from(0x1111_1111_1111_1111u64),
                    b: U256::from(0x2222_2222_2222_2222u64),
                },
            );
        }
    }
}

#[test]
fn mapping_to_multi_slot_struct_value_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract MapWide {
    struct Wide { uint256 a; uint256 b; }
    mapping(address => Wide) m;   // slot 0
    function populate() external {
        m[address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))] =
            Wide(0x1111111111111111, 0x2222222222222222);
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = map_wide::MapWide::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "MapWide"));
}

// --- mixed-packed struct value (top-level) + sentinel ----------------------

#[pvm_contract_sdk::contract]
mod mixed {
    use super::*;
    pub struct MixedC {
        pub m: Lazy<Mixed>,
        pub sentinel: Lazy<U256>,
    }
    impl MixedC {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.m.set(&Mixed {
                flag: true,
                count: 0x0102_0304_0506_0708u64,
                who: Address::from(ADDR_B),
            });
            self.sentinel.set(&U256::from(0xDEADu64));
        }
    }
}

#[test]
fn mixed_packed_struct_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract MixedStruct {
    struct M { bool flag; uint64 count; address who; }
    M m;               // slot 0 (flag@0, count@1, who@9 — 29 bytes)
    uint256 sentinel;  // slot 1
    function populate() external {
        m = M(true, 0x0102030405060708, address(uint160(0x00BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB)));
        sentinel = 0xDEAD;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = mixed::MixedC::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "MixedStruct"));
}

// --- StorageVec of a packed struct -----------------------------------------

#[pvm_contract_sdk::contract]
mod vec_pair {
    use super::*;
    pub struct VecPair {
        pub items: StorageVec<Pair>,
    }
    impl VecPair {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.items.push(&Pair { lo: 1, hi: 2 });
            self.items.push(&Pair { lo: 3, hi: 4 });
        }
    }
}

#[test]
fn storage_vec_of_struct_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract VecStruct {
    struct Pair { uint128 lo; uint128 hi; }
    Pair[] items;   // slot 0
    function populate() external {
        items.push(Pair(0x1, 0x2));
        items.push(Pair(0x3, 0x4));
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = vec_pair::VecPair::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "VecStruct"));
}

// --- struct with a dynamic field -------------------------------------------

#[pvm_contract_sdk::contract]
mod dyn_struct {
    use super::*;
    pub struct DynStruct {
        pub s: Lazy<DynS>,
    }
    impl DynStruct {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.s.set(&DynS {
                head: U256::from(0x99u64),
                tail: String::from("abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"),
            });
        }
    }
}

#[test]
fn struct_with_dynamic_field_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract DynStruct {
    struct S { uint256 head; string tail; }
    S s;   // head -> slot 0, tail -> slot 1 (+ keccak spill)
    function populate() external {
        s.head = 0x99;
        s.tail = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEF";
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = dyn_struct::DynStruct::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "DynStruct"));
}

// ---------------------------------------------------------------------------
// Mutation / clearing: delete / remove / pop / overwrite vs solc's
// SSTORE-of-zero deletion and read-modify-write semantics.
// ---------------------------------------------------------------------------

#[pvm_contract_sdk::contract]
mod mut_c {
    use super::*;
    pub struct MutC {
        pub a: Lazy<U256>,
        pub b: Lazy<U256>,
        pub m: Mapping<Address, U256>,
    }
    impl MutC {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.a.set(&U256::from(111u64));
            self.b.set(&U256::from(222u64));
            self.m.insert(&Address::from(ADDR_A), &U256::from(5u64));
            self.m.insert(&Address::from(ADDR_B), &U256::from(9u64));
            self.a.clear();
            self.m.remove(&Address::from(ADDR_A));
        }
    }
}

#[test]
fn clear_and_remove_match_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Mut {
    uint256 a;                     // slot 0
    uint256 b;                     // slot 1
    mapping(address => uint256) m; // slot 2
    function populate() external {
        a = 111; b = 222;
        delete a;
        m[address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))] = 5;
        m[address(uint160(0x00BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB))] = 9;
        delete m[address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))];
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = mut_c::MutC::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Mut"));
}

#[pvm_contract_sdk::contract]
mod vec_pop {
    use super::*;
    pub struct VecPop {
        pub v: StorageVec<U256>,
    }
    impl VecPop {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            for n in [11u64, 22, 33] {
                self.v.push(&U256::from(n));
            }
            self.v.pop();
        }
    }
}

#[test]
fn storage_vec_pop_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract VecPop {
    uint256[] v;   // slot 0
    function populate() external {
        v.push(11); v.push(22); v.push(33);
        v.pop();
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = vec_pop::VecPop::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "VecPop"));
}

#[pvm_contract_sdk::contract]
mod over {
    use super::*;
    pub struct Over {
        pub lo: Lazy<u128>,
        pub hi: Lazy<u128>,
    }
    impl Over {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.lo.set(&1u128);
            self.hi.set(&2u128);
            self.lo.set(&0xAAAA_AAAA_AAAA_AAAAu128);
        }
    }
}

#[test]
fn overwrite_packed_field_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Over {
    uint128 lo; uint128 hi;   // share slot 0
    function populate() external {
        lo = 1; hi = 2;
        lo = 0xAAAAAAAAAAAAAAAA;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = over::Over::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Over"));
}

// ---------------------------------------------------------------------------
// Edge cases: negative signed, non-address mapping keys, empty + multi-slot
// dynamics.
// ---------------------------------------------------------------------------

#[pvm_contract_sdk::contract]
mod signed {
    use super::*;
    pub struct Signed {
        pub a: Lazy<I256>,
        pub lo: Lazy<i64>,
        pub hi: Lazy<i64>,
    }
    impl Signed {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.a.set(&I256::MINUS_ONE);
            self.lo.set(&-5i64);
            self.hi.set(&7i64);
        }
    }
}

#[test]
fn signed_negative_match_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Signed {
    int256 a;   // slot 0
    int64 lo;   // slot 1, offset 0
    int64 hi;   // slot 1, offset 8
    function populate() external {
        a = -1;
        lo = -5;
        hi = 7;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = signed::Signed::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Signed"));
}

#[pvm_contract_sdk::contract]
mod uint_key {
    use super::*;
    pub struct UintKey {
        pub m: Mapping<U256, U256>,
    }
    impl UintKey {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.m.insert(&U256::from(7u64), &U256::from(100u64));
        }
    }
}

#[test]
fn mapping_uint_key_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract UintKey {
    mapping(uint256 => uint256) m;   // slot 0
    function populate() external { m[7] = 100; }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = uint_key::UintKey::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "UintKey"));
}

#[pvm_contract_sdk::contract]
mod str_key {
    use super::*;
    pub struct StrKey {
        pub m: Mapping<String, U256>,
    }
    impl StrKey {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.m.insert(&String::from("hello"), &U256::from(42u64));
        }
    }
}

#[test]
fn mapping_string_key_matches_solc() {
    // Dynamic key: slot is keccak256(key_bytes ++ pad32(slot)) over the *raw*
    // (unpadded) key bytes — a different derivation from fixed-size keys.
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract StrKey {
    mapping(string => uint256) m;   // slot 0
    function populate() external { m["hello"] = 42; }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = str_key::StrKey::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "StrKey"));
}

#[pvm_contract_sdk::contract]
mod b32_key {
    use super::*;
    pub struct B32Key {
        pub m: Mapping<[u8; 32], U256>,
    }
    impl B32Key {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            let mut key = [0u8; 32];
            key[30] = 0x12;
            key[31] = 0x34;
            self.m.insert(&key, &U256::from(9u64));
        }
    }
}

#[test]
fn mapping_bytes32_key_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract B32Key {
    mapping(bytes32 => uint256) m;   // slot 0
    function populate() external {
        m[bytes32(uint256(0x1234))] = 9;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = b32_key::B32Key::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "B32Key"));
}

#[pvm_contract_sdk::contract]
mod empty {
    use super::*;
    pub struct Empty {
        pub s: Lazy<String>,
        pub sentinel: Lazy<U256>,
    }
    impl Empty {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.s.set(&String::new());
            self.sentinel.set(&U256::from(5u64));
        }
    }
}

/// Empty `string`: **intentional divergence from solc.** solc stores nothing
/// for an empty dynamic value (the slot is 0 / deleted). pvm-storage writes an
/// `EMPTY_INLINE_SENTINEL` (`0x01` at byte 30 of the header slot) so `try_get`
/// can distinguish "explicitly set to empty" from "never set" (Option
/// semantics solc lacks). The differential therefore FAILS — captured as an
/// ignored, executable record of the deviation. Un-ignore if the SDK ever drops
/// the sentinel to match solc byte-for-byte.
#[test]
#[ignore = "intentional divergence: pvm-storage writes EMPTY_INLINE_SENTINEL for \
            empty dynamics (try_get Option semantics); solc deletes the slot"]
fn empty_string_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Empty {
    string s;          // slot 0
    uint256 sentinel;  // slot 1
    function populate() external {
        s = "";
        sentinel = 5;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = empty::Empty::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Empty"));
}

#[pvm_contract_sdk::contract]
mod long_str {
    use super::*;
    pub struct LongStr {
        pub s: Lazy<String>,
    }
    impl LongStr {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.s.set(&String::from(
                "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ABCDEFGH",
            ));
        }
    }
}

#[test]
fn multi_slot_string_matches_solc() {
    // 70 bytes -> spans 3 keccak body slots.
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract LongStr {
    string s;   // slot 0
    function populate() external {
        s = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ABCDEFGH";
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = long_str::LongStr::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "LongStr"));
}

// --- sub-word spill: a packed run overflows its slot -----------------------

#[pvm_contract_sdk::contract]
mod spill {
    use super::*;
    pub struct Spill {
        pub flag: Lazy<bool>,
        pub who: Lazy<Address>,
        pub big: Lazy<u128>,
        pub small2: Lazy<u128>,
        pub tail: Lazy<U256>,
    }
    impl Spill {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.flag.set(&true);
            self.who.set(&Address::from(ADDR_A));
            self.big.set(&0xCCCC_CCCC_CCCC_CCCCu128);
            self.small2.set(&0xDDDD_DDDD_DDDD_DDDDu128);
            self.tail.set(&U256::from(0xEEu64));
        }
    }
}

#[test]
fn subword_spill_match_solc() {
    // flag(1B)+who(20B) fill 21 bytes of slot 0, so big(16B) doesn't fit in the
    // remaining 11 bytes and starts slot 1, where small2 packs after it; tail
    // proves the field after the spilled run lands at the right slot.
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Spill {
    bool flag;       // slot 0, offset 0
    address who;     // slot 0, offset 1
    uint128 big;     // slot 1, offset 0  (spills)
    uint128 small2;  // slot 1, offset 16
    uint256 tail;    // slot 2
    function populate() external {
        flag   = true;
        who    = address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA));
        big    = 0xCCCCCCCCCCCCCCCC;
        small2 = 0xDDDDDDDDDDDDDDDD;
        tail   = 0xEE;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = spill::Spill::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Spill"));
}

// ---------------------------------------------------------------------------
// Nested collections: T[][] and mapping(K => T[]).
// ---------------------------------------------------------------------------

/// `uint256[][]` — a `StorageVec` of `StorageVec`. Each inner row's length
/// lives at `keccak256(outer_slot) + row`, its elements at
/// `keccak256(that inner slot) + i`.
#[pvm_contract_sdk::contract]
mod nested_vec {
    use super::*;
    pub struct NestedVec {
        pub rows: StorageVec<StorageVec<U256>>,
    }
    impl NestedVec {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            {
                let mut r0 = self.rows.grow();
                r0.push(&U256::from(1u64));
                r0.push(&U256::from(2u64));
            }
            {
                let mut r1 = self.rows.grow();
                r1.push(&U256::from(3u64));
            }
        }
    }
}

#[test]
fn nested_storage_vec_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Matrix {
    uint256[][] rows;   // slot 0
    function populate() external {
        rows.push(); rows[0].push(1); rows[0].push(2);
        rows.push(); rows[1].push(3);
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = nested_vec::NestedVec::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Matrix"));
}

/// `mapping(address => uint256[])` — each key derives a `StorageVec` root at
/// `keccak256(pad(key) ++ pad(slot))`; its length + elements follow from there.
#[pvm_contract_sdk::contract]
mod mapping_vec {
    use super::*;
    pub struct MappingVec {
        pub buckets: Mapping<Address, StorageVec<U256>>,
    }
    impl MappingVec {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            {
                let mut a = self.buckets.entry(&Address::from(ADDR_A));
                a.push(&U256::from(11u64));
                a.push(&U256::from(22u64));
            }
            self.buckets
                .entry(&Address::from(ADDR_B))
                .push(&U256::from(33u64));
        }
    }
}

#[test]
fn mapping_to_storage_vec_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Buckets {
    mapping(address => uint256[]) buckets;   // slot 0
    function populate() external {
        buckets[address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))].push(11);
        buckets[address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))].push(22);
        buckets[address(uint160(0x00BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB))].push(33);
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = mapping_vec::MappingVec::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "Buckets"));
}

// ---------------------------------------------------------------------------
// More StorageVec write ops: clear() the whole vector, set(i) an existing
// index, pop() a packed element.
// ---------------------------------------------------------------------------

/// `delete v` (whole-vector clear): length slot + every element slot are
/// deleted; a sentinel proves `clear()` doesn't over-delete.
#[pvm_contract_sdk::contract]
mod vec_clear {
    use super::*;
    pub struct VecClear {
        pub v: StorageVec<U256>,
        pub sentinel: Lazy<U256>,
    }
    impl VecClear {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            for n in [11u64, 22, 33] {
                self.v.push(&U256::from(n));
            }
            self.v.clear();
            self.sentinel.set(&U256::from(7u64));
        }
    }
}

#[test]
fn storage_vec_clear_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract VecClear {
    uint256[] v;       // slot 0
    uint256 sentinel;  // slot 1
    function populate() external {
        v.push(11); v.push(22); v.push(33);
        delete v;
        sentinel = 7;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = vec_clear::VecClear::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "VecClear"));
}

/// `v[i] = x` (overwrite an existing index) — read-modify-write of one element
/// slot, length and neighbour element unchanged.
#[pvm_contract_sdk::contract]
mod vec_set {
    use super::*;
    pub struct VecSet {
        pub v: StorageVec<U256>,
    }
    impl VecSet {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            self.v.push(&U256::from(11u64));
            self.v.push(&U256::from(22u64));
            self.v.set(0, &U256::from(99u64));
        }
    }
}

#[test]
fn storage_vec_set_index_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract VecSet {
    uint256[] v;   // slot 0
    function populate() external {
        v.push(11); v.push(22);
        v[0] = 99;
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = vec_set::VecSet::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "VecSet"));
}

/// `pop()` a PACKED element: `uint128[]` fits two per element slot. Popping the
/// third element clears its (sole-occupant) body slot; the first two stay
/// packed in the base element slot.
#[pvm_contract_sdk::contract]
mod vec_pop_packed {
    use super::*;
    pub struct VecPopPacked {
        pub v: StorageVec<u128>,
    }
    impl VecPopPacked {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
        #[pvm_contract_sdk::method]
        pub fn populate(&mut self) {
            for n in [1u128, 2, 3] {
                self.v.push(&n);
            }
            self.v.pop();
        }
    }
}

#[test]
fn storage_vec_pop_packed_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract VecPopPacked {
    uint128[] v;   // slot 0 (two elements per body slot)
    function populate() external {
        v.push(1); v.push(2); v.push(3);
        v.pop();
    }
}
"#;
    let mock = MockHostBuilder::new().build();
    let mut c = vec_pop_packed::VecPopPacked::with_host(mock.clone());
    c.populate();
    assert_eq!(normalize_mock(&mock), solc_storage(SOL, "VecPopPacked"));
}

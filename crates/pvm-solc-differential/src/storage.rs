//! Storage-representation fixtures + the solc/revm ground-truth harness.
//! See the crate-level docs in `lib.rs` for the overall approach.

use std::collections::BTreeMap;
use std::rc::Rc;

use pvm_contract_types::{Host, MockHost, MockHostBuilder};

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
// the SDK side (pvm-storage)
// ---------------------------------------------------------------------------

/// Build a `MockHost` + a `Host` handle sharing its state, run `writes`
/// (which drive `pvm-storage` directly), then return the normalized storage.
fn sdk_storage(writes: impl FnOnce(&Host)) -> StorageMap {
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(Rc::new(mock.clone()));
    writes(&host);
    normalize_mock(&mock)
}

fn normalize_mock(mock: &MockHost) -> StorageMap {
    let mut map = StorageMap::new();
    for (k, v) in mock.storage_dump() {
        let key = to_32(&k);
        let val = to_32(&v);
        if val != [0u8; 32] {
            map.insert(key, val);
        }
    }
    map
}

/// Left-pad (big-endian) a storage key/value to 32 bytes. pvm-storage always
/// writes full 32-byte words, but be defensive about any short value.
fn to_32(bytes: &[u8]) -> [u8; 32] {
    assert!(bytes.len() <= 32, "storage word longer than 32 bytes");
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(bytes);
    out
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

use pvm_contract_sdk::{
    Address, Bytes, I256, Lazy, Mapping, SolType, StorageComponent, StorageVec, U256,
};

/// Two distinct 20-byte addresses used across mapping/vec fixtures.
const ADDR_A: [u8; 20] = [0xAA; 20];
const ADDR_B: [u8; 20] = [0xBB; 20];

/// A packed static struct: two `uint128` share one 32-byte slot
/// (`lo` low-order @ offset 0, `hi` high-order @ offset 16, solc-style). Used
/// to verify packing *inside* a mapping value.
#[derive(Clone, Debug, PartialEq, Eq, SolType)]
pub struct Pair {
    pub lo: u128,
    pub hi: u128,
}

/// A genuinely multi-slot static struct: two `uint256` occupy two consecutive
/// slots (no packing). Used to verify a struct value spanning >1 derived slot.
#[derive(Clone, Debug, PartialEq, Eq, SolType)]
pub struct Wide {
    pub a: U256,
    pub b: U256,
}

/// Mixed sub-word packing inside one struct slot: solc places
/// `flag`@0 (1B), `count`@1 (8B), `who`@9 (20B) — 29 bytes, one slot.
#[derive(Clone, Debug, PartialEq, Eq, SolType)]
pub struct Mixed {
    pub flag: bool,
    pub count: u64,
    pub who: Address,
}

/// A struct with a trailing dynamic field: solc stores `head` at the struct's
/// first slot and lays out `tail` (a `string`) at the next slot using its
/// inline/spilled `bytes` layout.
#[derive(Clone, Debug, PartialEq, Eq, SolType)]
pub struct DynS {
    pub head: U256,
    pub tail: String,
}

#[test]
fn uint256_single_slot_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract S { uint256 x; function populate() external { x = 42; } }
"#;
    let actual = sdk_storage(|host| {
        let mut x = <Lazy<U256> as StorageComponent>::new_at(0, 0, host.clone());
        x.set(&U256::from(42u64));
    });
    assert_eq!(actual, solc_storage(SOL, "S"));
}

/// Sub-word fields packed into shared slots (read-modify-write must not clobber
/// neighbours) plus a full-slot uint256 and a packed uint128 pair. The big-endian
/// `new_at` offsets mirror what the macro's walker emits; solc's converted
/// offsets are `flag@0, small@1, who@5` in slot 0.
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
    let actual = sdk_storage(|host| {
        // Big-endian offsets: solc_offset = 32 - high - packed_bytes.
        let mut flag = <Lazy<bool> as StorageComponent>::new_at(0, 31, host.clone());
        let mut small = <Lazy<u32> as StorageComponent>::new_at(0, 27, host.clone());
        let mut who = <Lazy<Address> as StorageComponent>::new_at(0, 7, host.clone());
        let mut total = <Lazy<U256> as StorageComponent>::new_at(1, 0, host.clone());
        let mut lo = <Lazy<u128> as StorageComponent>::new_at(2, 16, host.clone());
        let mut hi = <Lazy<u128> as StorageComponent>::new_at(2, 0, host.clone());
        flag.set(&true);
        small.set(&0x0102_0304u32);
        who.set(&Address::from(ADDR_A));
        total.set(&U256::from(0x1122_3344_5566_7788u64));
        lo.set(&0xAAAA_AAAA_AAAA_AAAAu128);
        hi.set(&0xBBBB_BBBB_BBBB_BBBBu128);
    });
    assert_eq!(actual, solc_storage(SOL, "Packed"));
}

/// Mapping key derivation `keccak256(pad32(key) ++ pad32(slot))`, single and
/// nested.
#[test]
fn mappings_match_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Maps {
    mapping(address => uint256) balances;                              // slot 0
    mapping(address => mapping(address => uint256)) allowances;        // slot 1
    function populate() external {
        balances[address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))] = 1000;
        allowances
            [address(uint160(0x00AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA))]
            [address(uint160(0x00BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB))] = 777;
    }
}
"#;
    let actual = sdk_storage(|host| {
        let mut balances = <Mapping<Address, U256> as StorageComponent>::new_at(0, 0, host.clone());
        let mut allowances = <Mapping<Address, Mapping<Address, U256>> as StorageComponent>::new_at(
            1,
            0,
            host.clone(),
        );
        balances.insert(&Address::from(ADDR_A), &U256::from(1000u64));
        allowances
            .entry(&Address::from(ADDR_A))
            .insert(&Address::from(ADDR_B), &U256::from(777u64));
    });
    assert_eq!(actual, solc_storage(SOL, "Maps"));
}

/// Dynamic `string`/`bytes`: short (< 32B, inline in the slot with `2*len` in
/// the low byte) and long (>= 32B, length*2+1 in the slot, body at
/// `keccak256(slot) + i`).
#[test]
fn dynamic_string_bytes_match_solc() {
    const LONG: &str = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"; // 42 bytes -> spilled
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Dyns {
    string shortStr;  // slot 0
    string longStr;   // slot 1
    bytes blob;       // slot 2
    function populate() external {
        shortStr = "hello";
        longStr  = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEF";
        blob     = hex"0102030405060708";
    }
}
"#;
    let actual = sdk_storage(|host| {
        let mut short = <Lazy<String> as StorageComponent>::new_at(0, 0, host.clone());
        let mut long = <Lazy<String> as StorageComponent>::new_at(1, 0, host.clone());
        let mut blob = <Lazy<Bytes> as StorageComponent>::new_at(2, 0, host.clone());
        short.set(&String::from("hello"));
        long.set(&String::from(LONG));
        blob.set(&Bytes(vec![1, 2, 3, 4, 5, 6, 7, 8]));
    });
    assert_eq!(actual, solc_storage(SOL, "Dyns"));
}

/// `StorageVec`: length word at the base slot, elements at
/// `keccak256(slot) + i * stride`.
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
    let actual = sdk_storage(|host| {
        let mut nums = <StorageVec<U256> as StorageComponent>::new_at(0, 0, host.clone());
        let mut addrs = <StorageVec<Address> as StorageComponent>::new_at(1, 0, host.clone());
        for n in [11u64, 22, 33] {
            nums.push(&U256::from(n));
        }
        addrs.push(&Address::from(ADDR_A));
        addrs.push(&Address::from(ADDR_B));
    });
    assert_eq!(actual, solc_storage(SOL, "Vecs"));
}

/// Fixed arrays striped across slots: full-word `uint256[3]` (3 slots) and
/// sub-word packed `uint128[4]` (2 slots, 2 elements per slot).
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
    let actual = sdk_storage(|host| {
        let mut triple = <Lazy<[U256; 3]> as StorageComponent>::new_at(0, 0, host.clone());
        let mut quad = <Lazy<[u128; 4]> as StorageComponent>::new_at(3, 0, host.clone());
        triple.set(&[U256::from(1u64), U256::from(2u64), U256::from(3u64)]);
        quad.set(&[0xAu128, 0xB, 0xC, 0xD]);
    });
    assert_eq!(actual, solc_storage(SOL, "Arrays"));
}

/// Packing *inside* a mapping value: `mapping(address => Pair)` where the
/// struct's two `uint128` share the single derived slot
/// `keccak256(pad(key) ++ pad(slot))` — `lo` in the low 16 bytes, `hi` in the
/// high 16. Verifies key derivation AND intra-value field packing together.
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
    let actual = sdk_storage(|host| {
        let mut m = <Mapping<Address, Pair> as StorageComponent>::new_at(0, 0, host.clone());
        m.insert(
            &Address::from(ADDR_A),
            &Pair {
                lo: 0xAAAA_AAAA_AAAA_AAAAu128,
                hi: 0xBBBB_BBBB_BBBB_BBBBu128,
            },
        );
    });
    assert_eq!(actual, solc_storage(SOL, "MapStruct"));
}

/// A struct value spanning two derived slots: `mapping(address => Wide)` where
/// `Wide { uint256 a; uint256 b }` writes `a` at the derived slot and `b` at
/// derived slot + 1.
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
    let actual = sdk_storage(|host| {
        let mut m = <Mapping<Address, Wide> as StorageComponent>::new_at(0, 0, host.clone());
        m.insert(
            &Address::from(ADDR_A),
            &Wide {
                a: U256::from(0x1111_1111_1111_1111u64),
                b: U256::from(0x2222_2222_2222_2222u64),
            },
        );
    });
    assert_eq!(actual, solc_storage(SOL, "MapWide"));
}

/// Mixed sub-word packing inside a top-level struct slot, plus a sentinel in
/// the next slot to prove the struct stays within its own slot (doesn't bleed).
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
    let actual = sdk_storage(|host| {
        let mut m = <Lazy<Mixed> as StorageComponent>::new_at(0, 0, host.clone());
        let mut sentinel = <Lazy<U256> as StorageComponent>::new_at(1, 0, host.clone());
        m.set(&Mixed {
            flag: true,
            count: 0x0102_0304_0506_0708u64,
            who: Address::from(ADDR_B),
        });
        sentinel.set(&U256::from(0xDEADu64));
    });
    assert_eq!(actual, solc_storage(SOL, "MixedStruct"));
}

/// `StorageVec` of a packed struct: `Pair[]` — length word at the base slot,
/// each `Pair` element at `keccak256(slot) + i` (one slot per element, two
/// `uint128` packed within).
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
    let actual = sdk_storage(|host| {
        let mut items = <StorageVec<Pair> as StorageComponent>::new_at(0, 0, host.clone());
        items.push(&Pair { lo: 1, hi: 2 });
        items.push(&Pair { lo: 3, hi: 4 });
    });
    assert_eq!(actual, solc_storage(SOL, "VecStruct"));
}

/// A struct containing a dynamic `string` field: `head` at the struct's first
/// slot, `tail` laid out at the next slot with solc's spilled-`bytes` layout
/// (length*2+1 in the header slot, body at `keccak256(header_slot) + i`).
#[test]
fn struct_with_dynamic_field_matches_solc() {
    const LONG: &str = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"; // 42 bytes -> spilled
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
    let actual = sdk_storage(|host| {
        let mut s = <Lazy<DynS> as StorageComponent>::new_at(0, 0, host.clone());
        s.set(&DynS {
            head: U256::from(0x99u64),
            tail: String::from(LONG),
        });
    });
    assert_eq!(actual, solc_storage(SOL, "DynStruct"));
}

// ---------------------------------------------------------------------------
// Mutation / clearing (gap #2): delete / remove / pop / overwrite must match
// solc's SSTORE-of-zero deletion and read-modify-write semantics.
// ---------------------------------------------------------------------------

/// `delete` a `Lazy` and `delete m[k]` a mapping entry: the cleared slots must
/// vanish (SSTORE 0 = delete), leaving only the survivors (`b`, `m[B]`).
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
    let actual = sdk_storage(|host| {
        let mut a = <Lazy<U256> as StorageComponent>::new_at(0, 0, host.clone());
        let mut b = <Lazy<U256> as StorageComponent>::new_at(1, 0, host.clone());
        let mut m = <Mapping<Address, U256> as StorageComponent>::new_at(2, 0, host.clone());
        a.set(&U256::from(111u64));
        b.set(&U256::from(222u64));
        m.insert(&Address::from(ADDR_A), &U256::from(5u64));
        m.insert(&Address::from(ADDR_B), &U256::from(9u64));
        a.clear();
        m.remove(&Address::from(ADDR_A));
    });
    assert_eq!(actual, solc_storage(SOL, "Mut"));
}

/// `StorageVec::pop` must decrement the length AND clear the removed element's
/// slot (solc deletes it) — so the popped slot doesn't linger.
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
    let actual = sdk_storage(|host| {
        let mut v = <StorageVec<U256> as StorageComponent>::new_at(0, 0, host.clone());
        for n in [11u64, 22, 33] {
            v.push(&U256::from(n));
        }
        v.pop();
    });
    assert_eq!(actual, solc_storage(SOL, "VecPop"));
}

/// Overwriting one packed field must read-modify-write without clobbering its
/// slot-neighbour (`hi` must survive `lo`'s second write).
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
    let actual = sdk_storage(|host| {
        let mut lo = <Lazy<u128> as StorageComponent>::new_at(0, 16, host.clone());
        let mut hi = <Lazy<u128> as StorageComponent>::new_at(0, 0, host.clone());
        lo.set(&1u128);
        hi.set(&2u128);
        lo.set(&0xAAAA_AAAA_AAAA_AAAAu128);
    });
    assert_eq!(actual, solc_storage(SOL, "Over"));
}

// ---------------------------------------------------------------------------
// Edge cases (gap #3): negative signed (two's complement / sign-extension),
// non-address mapping keys, empty + multi-slot dynamics.
// ---------------------------------------------------------------------------

/// Negative signed values: full-slot `int256 = -1` (all 0xff) and packed
/// `int64` negatives (sign-extended within their 8-byte window, neighbour intact).
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
    let actual = sdk_storage(|host| {
        let mut a = <Lazy<I256> as StorageComponent>::new_at(0, 0, host.clone());
        let mut lo = <Lazy<i64> as StorageComponent>::new_at(1, 24, host.clone());
        let mut hi = <Lazy<i64> as StorageComponent>::new_at(1, 16, host.clone());
        a.set(&I256::MINUS_ONE);
        lo.set(&-5i64);
        hi.set(&7i64);
    });
    assert_eq!(actual, solc_storage(SOL, "Signed"));
}

/// `mapping(uint256 => uint256)` — integer key left-padded to 32 bytes.
#[test]
fn mapping_uint_key_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract UintKey {
    mapping(uint256 => uint256) m;   // slot 0
    function populate() external { m[7] = 100; }
}
"#;
    let actual = sdk_storage(|host| {
        let mut m = <Mapping<U256, U256> as StorageComponent>::new_at(0, 0, host.clone());
        m.insert(&U256::from(7u64), &U256::from(100u64));
    });
    assert_eq!(actual, solc_storage(SOL, "UintKey"));
}

/// `mapping(string => uint256)` — DYNAMIC key: the slot is
/// `keccak256(key_bytes ++ pad32(slot))` over the *raw* (unpadded) key bytes,
/// a different derivation from fixed-size keys.
#[test]
fn mapping_string_key_matches_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract StrKey {
    mapping(string => uint256) m;   // slot 0
    function populate() external { m["hello"] = 42; }
}
"#;
    let actual = sdk_storage(|host| {
        let mut m = <Mapping<String, U256> as StorageComponent>::new_at(0, 0, host.clone());
        m.insert(&String::from("hello"), &U256::from(42u64));
    });
    assert_eq!(actual, solc_storage(SOL, "StrKey"));
}

/// `mapping(bytes32 => uint256)` — fixed-bytes key used directly (32 bytes,
/// no hashing of the key itself).
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
    let actual = sdk_storage(|host| {
        let mut m = <Mapping<[u8; 32], U256> as StorageComponent>::new_at(0, 0, host.clone());
        let mut key = [0u8; 32];
        key[30] = 0x12;
        key[31] = 0x34;
        m.insert(&key, &U256::from(9u64));
    });
    assert_eq!(actual, solc_storage(SOL, "B32Key"));
}

/// Empty `string`: **intentional divergence from solc.** solc stores nothing
/// for an empty dynamic value (the slot is 0 / deleted). pvm-storage writes an
/// `EMPTY_INLINE_SENTINEL` (`0x01` at byte 30 of the header slot) so `try_get`
/// can distinguish "explicitly set to empty" from "never set" (Option
/// semantics solc lacks). The differential test therefore FAILS — captured as
/// ignored, executable documentation of the deviation.
///
/// Interop note: a Solidity reader of our slot sees low byte 0 → length 0 →
/// empty (same value), but the raw slot is non-zero where solc's is deleted
/// (gas/refund and byte-equality differ). If the SDK ever drops the sentinel
/// to match solc byte-for-byte, un-ignore this.
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
    let actual = sdk_storage(|host| {
        let mut s = <Lazy<String> as StorageComponent>::new_at(0, 0, host.clone());
        let mut sentinel = <Lazy<U256> as StorageComponent>::new_at(1, 0, host.clone());
        s.set(&String::new());
        sentinel.set(&U256::from(5u64));
    });
    assert_eq!(actual, solc_storage(SOL, "Empty"));
}

/// A `string` long enough to span multiple keccak body slots (70 bytes -> 3
/// chunks): header (`2*len+1`) at the base slot, body at `keccak256(slot) + i`.
#[test]
fn multi_slot_string_matches_solc() {
    const LONG: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ABCDEFGH";
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract LongStr {
    string s;   // slot 0
    function populate() external {
        s = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ABCDEFGH";
    }
}
"#;
    let actual = sdk_storage(|host| {
        let mut s = <Lazy<String> as StorageComponent>::new_at(0, 0, host.clone());
        s.set(&String::from(LONG));
    });
    assert_eq!(actual, solc_storage(SOL, "LongStr"));
}

/// Sub-word **spill**: `flag`(1B) + `who`(20B) fill 21 bytes of slot 0, so the
/// next `uint128`(16B) doesn't fit in the remaining 11 bytes and starts a fresh
/// slot 1 — where `small2`(16B) then packs alongside it. A `tail` proves the
/// field after the spilled run lands at the right slot.
///
/// solc layout: flag@s0/off0, who@s0/off1, big@s1/off0, small2@s1/off16, tail@s2.
#[test]
fn subword_spill_match_solc() {
    const SOL: &str = r#"
pragma solidity ^0.8.26;
contract Spill {
    bool flag;       // slot 0, offset 0
    address who;     // slot 0, offset 1  (fills bytes 1..21)
    uint128 big;     // slot 1, offset 0  (spills: 16 > 11 bytes left in slot 0)
    uint128 small2;  // slot 1, offset 16 (packs after big)
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
    let actual = sdk_storage(|host| {
        // Big-endian offsets: high = 32 - solc_offset - packed_bytes.
        let mut flag = <Lazy<bool> as StorageComponent>::new_at(0, 31, host.clone());
        let mut who = <Lazy<Address> as StorageComponent>::new_at(0, 11, host.clone());
        let mut big = <Lazy<u128> as StorageComponent>::new_at(1, 16, host.clone());
        let mut small2 = <Lazy<u128> as StorageComponent>::new_at(1, 0, host.clone());
        let mut tail = <Lazy<U256> as StorageComponent>::new_at(2, 0, host.clone());
        flag.set(&true);
        who.set(&Address::from(ADDR_A));
        big.set(&0xCCCC_CCCC_CCCC_CCCCu128);
        small2.set(&0xDDDD_DDDD_DDDD_DDDDu128);
        tail.set(&U256::from(0xEEu64));
    });
    assert_eq!(actual, solc_storage(SOL, "Spill"));
}

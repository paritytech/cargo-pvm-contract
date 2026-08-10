//! The fixed corpus of values the TypeScript suite is checked against.
//!
//! Every case pairs a Rust value with the JSON viem should accept for it. The
//! Rust value is encoded by the SDK; viem encodes the JSON; the suite asserts
//! the bytes match. Writing the JSON out by hand rather than deriving it is
//! deliberate — a derived representation could only ever agree with itself,
//! whereas a hand-written one makes a real claim about what viem is handed.
//!
//! Contract cases name a function by its ABI name and canonical signature. The
//! argument and return *types* are re-declared locally rather than imported
//! from `examples/test-contracts` (a separate, riscv-targeted project); the
//! suite's selector assertion is what ties a case back to the emitted ABI, so a
//! drifting local declaration fails loudly instead of silently testing nothing.

use pvm_contract_sdk::{
    Address, Bytes, I256, Panic, RevertString, SolError, SolEvent, SolType, U256,
};
use serde_json::json;

use crate::surface::abi_surface::{
    AnonymousPing, DetailedFailure, Indexed3, IndexedComposite, IndexedDynamic,
    InsufficientBalance, Pair, Profile, SurfaceError, Unauthorized,
};
use crate::{
    ContractFixture, ErrorCase, EventCase, Fixtures, FunctionCase, ParameterCase, bytes,
    error_case, event_case, function_case, function_case_noargs, num, parameter_case,
};

// ---------------------------------------------------------------------------
// Types mirroring contracts that live outside this workspace crate
// ---------------------------------------------------------------------------

/// Mirrors `PointAdder.sol`'s `struct Point { uint a; uint b; }`. The component
/// *names* matter: viem needs an object rather than an array for a named tuple.
#[derive(SolType, Debug, PartialEq)]
pub struct Point {
    pub a: U256,
    pub b: U256,
}

/// Mirrors `sol/SolTypeSurface.sol`'s `Nested`: a struct field that is itself a
/// struct, so the ABI carries nested `tuple` components.
#[derive(SolType, Debug, PartialEq)]
pub struct Nested {
    pub inner: Pair,
    pub owner: Address,
}

/// Mirrors `sol/SolTypeSurface.sol`'s `Bundle`: every array shape in one struct.
#[derive(SolType, Debug, PartialEq)]
pub struct Bundle {
    pub dynamic_pairs: Vec<Pair>,
    pub fixed_pairs: [Pair; 2],
    pub tags: [[u8; 4]; 3],
}

/// Mirrors `error-handling.rs`'s parameterless errors.
#[derive(Debug, SolError)]
pub struct AlwaysReverts;

#[derive(Debug, SolError)]
pub struct ZeroNotAllowed;

/// Mirrors `error-handling.rs`'s error enum. Its own selector is zeroed, so
/// encoding through it is what proves the wire selector comes from the held
/// variant.
#[derive(Debug, SolError)]
pub enum ContractError {
    AlwaysReverts(AlwaysReverts),
    ZeroNotAllowed(ZeroNotAllowed),
}

/// Mirrors `example-mytoken-macro-storage`. Nested in its own module because
/// `Transfer` and the field-less `InsufficientBalance` would otherwise collide
/// with names already in scope from the `abi-surface` contract.
pub mod mytoken {
    use super::*;

    #[derive(SolEvent)]
    pub struct Transfer {
        #[indexed]
        pub from: Address,
        #[indexed]
        pub to: Address,
        pub value: U256,
    }

    #[derive(Debug, SolError)]
    pub struct InsufficientBalance;
}

/// Mirrors the errors and events of `sol/SolTypeSurface.sol`. Nested because
/// several names deliberately match the `abi-surface` contract's — the point is
/// that the same shape reaches the same wire bytes through both ABI paths.
pub mod sol_surface {
    use super::*;

    #[derive(Debug, SolError)]
    pub struct Unauthorized;

    #[derive(Debug, SolError)]
    pub struct InsufficientBalance {
        pub account: Address,
        pub required: U256,
        pub available: U256,
    }

    #[derive(Debug, SolError)]
    pub struct DetailedFailure {
        pub reason: String,
        pub code: u32,
    }

    /// A struct field beside a dynamic array, so the payload has both nested
    /// components and a head/tail split.
    #[derive(Debug, SolError)]
    pub struct CompositeFailure {
        pub p: Pair,
        pub values: Vec<U256>,
    }

    #[derive(SolEvent)]
    pub struct Simple {
        #[indexed]
        pub who: Address,
        pub amount: U256,
    }

    #[derive(SolEvent)]
    #[alloc]
    pub struct IndexedDynamic {
        #[indexed]
        pub name: String,
        #[indexed]
        pub payload: Bytes,
        pub note: String,
    }

    #[derive(SolEvent)]
    #[alloc]
    pub struct IndexedComposite {
        #[indexed]
        pub p: Pair,
        pub values: Vec<U256>,
    }

    #[derive(SolEvent)]
    pub struct ThreeIndexed {
        #[indexed]
        pub a: Address,
        #[indexed]
        pub b: U256,
        #[indexed]
        pub c: [u8; 32],
        pub d: u64,
    }

    #[derive(SolEvent)]
    #[anonymous]
    pub struct Anonymous {
        #[indexed]
        pub a: U256,
        #[indexed]
        pub b: Address,
        pub c: bool,
    }
}

/// Mirrors `Events.sol`'s `ValueChanged`. Field names here are the Rust ones;
/// the ABI (and therefore `decodeEventLog`) uses the `.sol` spelling
/// `oldValue` / `newValue`, which the fixture's `decoded` map reflects.
#[derive(SolEvent)]
pub struct ValueChanged {
    #[indexed]
    pub who: Address,
    pub old_value: U256,
    pub new_value: U256,
}

// ---------------------------------------------------------------------------
// Reusable values
// ---------------------------------------------------------------------------

const ADDR_BYTES: [u8; 20] = [
    0xd8, 0xda, 0x6b, 0xf2, 0x69, 0x64, 0xaf, 0x9d, 0x7e, 0xed, 0x9e, 0x03, 0xe5, 0x34, 0x15, 0xd3,
    0x7a, 0xa9, 0x60, 0x45,
];

fn addr() -> Address {
    Address(ADDR_BYTES)
}

fn addr_json() -> serde_json::Value {
    bytes(&ADDR_BYTES)
}

/// A 32-byte tag with a non-zero low byte, so a left-aligned `bytes32` and a
/// right-aligned `uint256` of the same value cannot be confused.
const TAG: [u8; 32] = [
    0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0x01,
];

/// Build every fixture.
pub fn build() -> Fixtures {
    Fixtures {
        parameters: parameters(),
        contracts: vec![
            flipper(),
            multi_method(),
            return_values(),
            dynamic_types(),
            composite_types(),
            storage_types(),
            point_adder(),
            constructor_args(),
            events(),
            error_handling(),
            error_caller(),
            payable(),
            receive(),
            caller_check(),
            flipper_call(),
            flipper_delegate(),
            point_adder_call(),
            mytoken_storage(),
            abi_surface(),
            sol_type_surface(),
        ],
    }
}

// ---------------------------------------------------------------------------
// A. Raw parameter round-trips
//
// Scope is the *delta* over `pvm-contract-types::tests`, which already checks
// every primitive and the common container shapes byte-for-byte against
// alloy-core under proptest. Duplicating that here would buy a weaker version of
// the same claim; what earns its place is one value per `SOL_NAME` family (the
// only check on `abi_param`'s descriptor output) plus the composites and
// boundaries the alloy differential does not reach.
// ---------------------------------------------------------------------------

fn parameters() -> Vec<ParameterCase> {
    let mut cases = Vec::new();
    let mut p = |case: ParameterCase| cases.push(case);

    // --- one value per SOL_NAME family -------------------------------------
    //
    // These exist for the descriptor, not the bytes: a parameter case builds
    // its `types` from `SolEncode::abi_param()`, and this is the only place
    // that output is handed to viem. The contract suites read `types` from the
    // emitted ABI files instead, so they never touch `abi_param`.
    p(parameter_case("uint8/max", &u8::MAX, num(u8::MAX)));
    p(parameter_case("uint256/max", &U256::MAX, num(U256::MAX)));
    p(parameter_case("int8/min", &i8::MIN, num(i8::MIN)));
    p(parameter_case("int256/min", &I256::MIN, num(I256::MIN)));
    p(parameter_case("bool/true", &true, json!(true)));
    p(parameter_case("address/value", &addr(), addr_json()));
    p(parameter_case(
        "bytes4/selector",
        &[0xa9u8, 0x05, 0x9c, 0xbb],
        bytes(&[0xa9, 0x05, 0x9c, 0xbb]),
    ));
    p(parameter_case("bytes32/tag", &TAG, bytes(&TAG)));
    p(parameter_case(
        "string/33-bytes",
        &"abcdefghijklmnopqrstuvwxyz0123456".to_string(),
        json!("abcdefghijklmnopqrstuvwxyz0123456"),
    ));
    p(parameter_case(
        "uint256-array/three",
        &vec![U256::from(1u64), U256::from(2u64), U256::MAX],
        json!(["1", "2", U256::MAX.to_string()]),
    ));
    p(parameter_case(
        "string-array",
        &vec![String::new(), "x".to_string()],
        json!(["", "x"]),
    ));

    // `Bytes` and `Vec<u8>` are the same Rust shape with different Solidity
    // types, so the pair is kept adjacent — `parameters.test.ts` asserts the two
    // encodings differ.
    p(parameter_case(
        "bytes/1",
        &Bytes(vec![0xff]),
        bytes(&[0xff]),
    ));
    p(parameter_case(
        "uint8-array/one",
        &vec![0xffu8],
        json!(["255"]),
    ));

    // --- shapes the alloy differential does not reach ----------------------
    //
    // `pvm-contract-types::tests` already pins every primitive and the common
    // container shapes against alloy-core, several under proptest. Restating
    // those here would only add hand-written vectors to keep in sync, so what
    // follows is the delta: composites and boundaries that differential does
    // not cover.

    // `bytes` either side of a two-word body (the alloy tests stop at 33).
    for (id, len) in [("bytes/63", 63usize), ("bytes/64", 64), ("bytes/65", 65)] {
        let data = vec![0x5au8; len];
        p(parameter_case(id, &Bytes(data.clone()), bytes(&data)));
    }
    // Only 4-byte code points: the length word counts bytes, not characters.
    p(parameter_case(
        "string/astral-only",
        &"🎉🎉🎉".to_string(),
        json!("🎉🎉🎉"),
    ));

    // Fixed arrays: single element, a `bytesN` element, and a dynamic element.
    p(parameter_case("uint64-fixed1", &[7u64], json!(["7"])));
    p(parameter_case(
        "bytes4-fixed3",
        &[[1u8, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]],
        json!(["0x01020304", "0x05060708", "0x090a0b0c"]),
    ));
    p(parameter_case(
        "string-fixed2",
        &[
            "".to_string(),
            "abcdefghijklmnopqrstuvwxyz0123456".to_string(),
        ],
        json!(["", "abcdefghijklmnopqrstuvwxyz0123456"]),
    ));
    p(parameter_case(
        "uint256-array-fixed2",
        &[vec![U256::from(1u64)], Vec::<U256>::new()],
        json!([["1"], []]),
    ));

    p(parameter_case(
        "int256-array",
        &vec![I256::MIN, I256::MINUS_ONE, I256::ZERO, I256::MAX],
        json!([I256::MIN.to_string(), "-1", "0", I256::MAX.to_string()]),
    ));

    // Two levels of dynamic nesting over a dynamic leaf.
    p(parameter_case(
        "bytes-array-array",
        &vec![vec![Bytes(vec![0xaa]), Bytes(vec![])], Vec::<Bytes>::new()],
        json!([["0xaa", "0x"], []]),
    ));
    p(parameter_case(
        "string-array-array",
        &vec![
            vec!["a".to_string()],
            vec![],
            vec![String::new(), "c".to_string()],
        ],
        json!([["a"], [], ["", "c"]]),
    ));

    // Tuples whose members are themselves composites.
    p(parameter_case(
        "tuple/arity1",
        &(U256::from(7u64),),
        json!(["7"]),
    ));
    p(parameter_case(
        "tuple/with-array",
        &(vec![U256::from(1u64), U256::from(2u64)], u8::MAX),
        json!([["1", "2"], num(u8::MAX)]),
    ));
    p(parameter_case(
        "tuple/with-struct",
        &(Pair { lo: 1, hi: 2 }, "tail".to_string()),
        json!([{ "lo": "1", "hi": "2" }, "tail"]),
    ));
    p(parameter_case(
        "tuple/nested-tuple",
        &((U256::from(1u64), true), "tail".to_string()),
        json!([["1", true], "tail"]),
    ));

    // `#[derive(SolType)]` structs: the descriptor carries named components,
    // which is what makes viem expect an object rather than an array.
    p(parameter_case(
        "struct/static",
        &Pair {
            lo: 1,
            hi: u64::MAX,
        },
        json!({ "lo": "1", "hi": u64::MAX.to_string() }),
    ));
    p(parameter_case(
        "struct/dynamic",
        &Profile {
            id: U256::from(9u64),
            name: "profile".to_string(),
            tags: vec![1u32, 2, 3],
        },
        json!({ "id": "9", "name": "profile", "tags": ["1", "2", "3"] }),
    ));
    // A struct field that is itself a struct: nested `tuple` components.
    p(parameter_case(
        "struct/nested",
        &Nested {
            inner: Pair { lo: 1, hi: 2 },
            owner: addr(),
        },
        json!({ "inner": { "lo": "1", "hi": "2" }, "owner": addr_json() }),
    ));
    // A struct whose fields cover every array shape at once.
    p(parameter_case(
        "struct/bundle",
        &Bundle {
            dynamic_pairs: vec![Pair { lo: 1, hi: 2 }],
            fixed_pairs: [Pair { lo: 3, hi: 4 }, Pair { lo: 5, hi: 6 }],
            tags: [[1u8, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]],
        },
        // Keys are the *Rust* field names: a parameter case derives its types
        // from `abi_param`, not from the `.sol` file. The `.sol` spellings are
        // exercised by the `sol-type-surface` function cases.
        json!({
            "dynamic_pairs": [{ "lo": "1", "hi": "2" }],
            "fixed_pairs": [{ "lo": "3", "hi": "4" }, { "lo": "5", "hi": "6" }],
            "tags": ["0x01020304", "0x05060708", "0x090a0b0c"],
        }),
    ));
    p(parameter_case(
        "struct-array",
        &vec![Pair { lo: 1, hi: 2 }, Pair { lo: 3, hi: 4 }],
        json!([{ "lo": "1", "hi": "2" }, { "lo": "3", "hi": "4" }]),
    ));
    // Array of a *dynamic* struct: an offset table over head/tail bodies.
    p(parameter_case(
        "dynamic-struct-array",
        &vec![
            Profile {
                id: U256::ZERO,
                name: String::new(),
                tags: vec![],
            },
            Profile {
                id: U256::MAX,
                name: "b".repeat(40),
                tags: vec![7u32],
            },
        ],
        json!([
            { "id": "0", "name": "", "tags": [] },
            { "id": U256::MAX.to_string(), "name": "b".repeat(40), "tags": ["7"] },
        ]),
    ));
    p(parameter_case(
        "struct-fixed2",
        &[Pair { lo: 1, hi: 2 }, Pair { lo: 3, hi: 4 }],
        json!([{ "lo": "1", "hi": "2" }, { "lo": "3", "hi": "4" }]),
    ));

    // `&str` is the one encodable type with no decoder — it cannot own the
    // bytes it would decode into — so it uses the encode-only constructor.
    p(crate::parameter_case_encode_only(
        "str-slice",
        &"borrowed",
        json!("borrowed"),
    ));

    cases
}

// ---------------------------------------------------------------------------
// B/C/D. Per-contract cases
// ---------------------------------------------------------------------------

/// Shorthand for a contract whose ABI is in `abi/{name}.abi.json`.
fn contract(
    name: &str,
    functions: Vec<FunctionCase>,
    errors: Vec<ErrorCase>,
    events: Vec<EventCase>,
) -> ContractFixture {
    ContractFixture {
        name: name.to_string(),
        abi_file: format!("abi/{name}.abi.json"),
        wrapped: false,
        functions,
        errors,
        events,
    }
}

/// A function returning nothing. The turbofish keeps `R` inferrable.
fn void(id: &str, function_name: &str, signature: &str) -> FunctionCase {
    function_case_noargs::<bool>(id, function_name, signature, None)
}

fn flipper() -> ContractFixture {
    contract(
        "flipper",
        vec![
            void("flip", "flip", "flip()"),
            function_case_noargs("get", "get", "get()", Some((&true, json!(true)))),
        ],
        vec![],
        vec![],
    )
}

fn multi_method() -> ContractFixture {
    contract(
        "multi-method",
        vec![
            function_case(
                "add",
                "add",
                "add(uint256,uint256)",
                &(U256::from(2u64), U256::MAX - U256::from(1u64)),
                vec![num(2), num(U256::MAX - U256::from(1u64))],
                Some((&U256::MAX, num(U256::MAX))),
            ),
            function_case(
                "isZero",
                "isZero",
                "isZero(uint256)",
                &(U256::ZERO,),
                vec![num(0)],
                Some((&true, json!(true))),
            ),
            function_case_noargs(
                "getCounter",
                "getCounter",
                "getCounter()",
                Some((&U256::from(41u64), num(41))),
            ),
            function_case(
                "mul",
                "mul",
                "mul(uint256,uint256)",
                &(U256::from(3u64), U256::from(4u64)),
                vec![num(3), num(4)],
                Some((&U256::from(12u64), num(12))),
            ),
            void("increment", "increment", "increment()"),
            void("reset", "reset", "reset()"),
        ],
        vec![],
        vec![],
    )
}

fn return_values() -> ContractFixture {
    contract(
        "return-values",
        vec![
            // Two outputs: a flat body, decoded by viem as an array.
            function_case_noargs(
                "getPair",
                "getPair",
                "getPair()",
                Some((&(U256::from(42u64), true), json!(["42", true]))),
            ),
            function_case_noargs(
                "getTriple",
                "getTriple",
                "getTriple()",
                Some((
                    &(U256::MAX, addr(), false),
                    json!([U256::MAX.to_string(), addr_json(), false]),
                )),
            ),
            function_case(
                "identity",
                "identity",
                "identity(uint256)",
                &(U256::from(7u64),),
                vec![num(7)],
                Some((&U256::from(7u64), num(7))),
            ),
        ],
        vec![],
        vec![],
    )
}

fn dynamic_types() -> ContractFixture {
    contract(
        "dynamic-types",
        vec![
            function_case(
                "getStringLength/empty",
                "getStringLength",
                "getStringLength(string)",
                &(String::new(),),
                vec![json!("")],
                Some((&U256::ZERO, num(0))),
            ),
            function_case(
                "getStringLength/unicode",
                "getStringLength",
                "getStringLength(string)",
                &("héllo 🎉".to_string(),),
                vec![json!("héllo 🎉")],
                Some((&U256::from(13u64), num(13))),
            ),
            // Single dynamic output: a 0x20 offset word precedes the body.
            function_case_noargs(
                "echoString",
                "echoString",
                "echoString()",
                Some((&"hello world".to_string(), json!("hello world"))),
            ),
            function_case(
                "getBytesLength",
                "getBytesLength",
                "getBytesLength(bytes)",
                &(Bytes(vec![0xde, 0xad, 0xbe, 0xef]),),
                vec![bytes(&[0xde, 0xad, 0xbe, 0xef])],
                Some((&U256::from(4u64), num(4))),
            ),
            function_case_noargs(
                "echoBytes",
                "echoBytes",
                "echoBytes()",
                Some((&Bytes(vec![0x01, 0x02, 0x03]), bytes(&[1, 2, 3]))),
            ),
            function_case(
                "sumArray",
                "sumArray",
                "sumArray(uint256[])",
                &(vec![U256::from(1u64), U256::from(2u64), U256::from(3u64)],),
                vec![json!(["1", "2", "3"])],
                Some((&U256::from(6u64), num(6))),
            ),
            function_case(
                "sumArray/empty",
                "sumArray",
                "sumArray(uint256[])",
                &(Vec::<U256>::new(),),
                vec![json!([])],
                Some((&U256::ZERO, num(0))),
            ),
            function_case_noargs(
                "getArray",
                "getArray",
                "getArray()",
                Some((&vec![U256::from(1u64), U256::from(2u64)], json!(["1", "2"]))),
            ),
        ],
        vec![],
        vec![],
    )
}

fn composite_types() -> ContractFixture {
    contract(
        "composite-types",
        vec![
            function_case(
                "sumFixedArray",
                "sumFixedArray",
                "sumFixedArray(uint256[3])",
                &([U256::from(10u64), U256::from(20u64), U256::from(30u64)],),
                vec![json!(["10", "20", "30"])],
                Some((&U256::from(60u64), num(60))),
            ),
            function_case_noargs(
                "getFixedArray",
                "getFixedArray",
                "getFixedArray()",
                Some((
                    &[U256::from(10u64), U256::from(20u64), U256::from(30u64)],
                    json!(["10", "20", "30"]),
                )),
            ),
            // A single tuple parameter with *unnamed* components, so viem wants
            // an array for it rather than an object.
            function_case(
                "processTuple",
                "processTuple",
                "processTuple((uint256,bool))",
                &((U256::from(99u64), true),),
                vec![json!(["99", true])],
                Some((&U256::from(99u64), num(99))),
            ),
        ],
        vec![],
        vec![],
    )
}

fn storage_types() -> ContractFixture {
    contract(
        "storage-types",
        vec![
            function_case(
                "setU8/max",
                "setU8",
                "setU8(uint8)",
                &(u8::MAX,),
                vec![num(u8::MAX)],
                None::<(&bool, _)>,
            ),
            function_case_noargs("getU8", "getU8", "getU8()", Some((&u8::MAX, num(u8::MAX)))),
            function_case(
                "setU16/max",
                "setU16",
                "setU16(uint16)",
                &(u16::MAX,),
                vec![num(u16::MAX)],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getU16",
                "getU16",
                "getU16()",
                Some((&u16::MAX, num(u16::MAX))),
            ),
            function_case(
                "setU32/max",
                "setU32",
                "setU32(uint32)",
                &(u32::MAX,),
                vec![num(u32::MAX)],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getU32",
                "getU32",
                "getU32()",
                Some((&u32::MAX, num(u32::MAX))),
            ),
            function_case(
                "setU64/max",
                "setU64",
                "setU64(uint64)",
                &(u64::MAX,),
                vec![num(u64::MAX)],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getU64",
                "getU64",
                "getU64()",
                Some((&u64::MAX, num(u64::MAX))),
            ),
            function_case(
                "setU128/max",
                "setU128",
                "setU128(uint128)",
                &(u128::MAX,),
                vec![num(u128::MAX)],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getU128",
                "getU128",
                "getU128()",
                Some((&u128::MAX, num(u128::MAX))),
            ),
            function_case(
                "setU256/max",
                "setU256",
                "setU256(uint256)",
                &(U256::MAX,),
                vec![num(U256::MAX)],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getU256",
                "getU256",
                "getU256()",
                Some((&U256::MAX, num(U256::MAX))),
            ),
            function_case(
                "setBool",
                "setBool",
                "setBool(bool)",
                &(true,),
                vec![json!(true)],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getBool",
                "getBool",
                "getBool()",
                Some((&true, json!(true))),
            ),
            function_case(
                "setBytes32",
                "setBytes32",
                "setBytes32(bytes32)",
                &(TAG,),
                vec![bytes(&TAG)],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getBytes32",
                "getBytes32",
                "getBytes32()",
                Some((&TAG, bytes(&TAG))),
            ),
            function_case(
                "setAddress",
                "setAddress",
                "setAddress(address)",
                &(addr(),),
                vec![addr_json()],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getAddress",
                "getAddress",
                "getAddress()",
                Some((&addr(), addr_json())),
            ),
        ],
        vec![],
        vec![],
    )
}

fn point_adder() -> ContractFixture {
    contract(
        "point-adder",
        vec![function_case(
            "add",
            "add",
            "add((uint256,uint256),(uint256,uint256))",
            &(
                Point {
                    a: U256::from(1u64),
                    b: U256::from(2u64),
                },
                Point {
                    a: U256::from(3u64),
                    b: U256::from(4u64),
                },
            ),
            vec![json!({ "a": "1", "b": "2" }), json!({ "a": "3", "b": "4" })],
            Some((
                &Point {
                    a: U256::from(4u64),
                    b: U256::from(6u64),
                },
                json!({ "a": "4", "b": "6" }),
            )),
        )],
        vec![],
        vec![],
    )
}

fn constructor_args() -> ContractFixture {
    contract(
        "constructor-args",
        vec![
            function_case_noargs(
                "getOwner",
                "getOwner",
                "getOwner()",
                Some((&addr(), addr_json())),
            ),
            function_case_noargs(
                "getInitialSupply",
                "getInitialSupply",
                "getInitialSupply()",
                Some((&U256::from(1_000_000u64), num(1_000_000))),
            ),
        ],
        vec![],
        vec![],
    )
}

/// The remaining `.sol`-backed binaries. Their ABI surface is small — an
/// `address` argument and an accessor or two — but the coverage gate in
/// `abi-shape.test.ts` requires every emitted item to be exercised, so that a
/// newly added contract method cannot silently go unchecked.
fn caller_check() -> ContractFixture {
    contract(
        "caller-check",
        vec![
            function_case_noargs(
                "getCaller",
                "getCaller",
                "getCaller()",
                Some((&addr(), addr_json())),
            ),
            void("recordCaller", "recordCaller", "recordCaller()"),
            function_case_noargs(
                "getLastCaller",
                "getLastCaller",
                "getLastCaller()",
                Some((&addr(), addr_json())),
            ),
        ],
        vec![],
        vec![],
    )
}

fn flipper_call() -> ContractFixture {
    contract(
        "flipper-call",
        vec![function_case(
            "callFlipper",
            "callFlipper",
            "callFlipper(address)",
            &(addr(),),
            vec![addr_json()],
            None::<(&bool, _)>,
        )],
        vec![],
        vec![],
    )
}

fn flipper_delegate() -> ContractFixture {
    contract(
        "flipper-delegate",
        vec![
            function_case(
                "delegateFlipper",
                "delegateFlipper",
                "delegateFlipper(address)",
                &(addr(),),
                vec![addr_json()],
                None::<(&bool, _)>,
            ),
            function_case_noargs("get", "get", "get()", Some((&false, json!(false)))),
        ],
        vec![],
        vec![],
    )
}

fn point_adder_call() -> ContractFixture {
    contract(
        "point-adder-call",
        vec![function_case(
            "callPointAdder",
            "callPointAdder",
            "callPointAdder(address)",
            &(addr(),),
            vec![addr_json()],
            None::<(&bool, _)>,
        )],
        vec![],
        vec![],
    )
}

fn events() -> ContractFixture {
    let event = ValueChanged {
        who: addr(),
        old_value: U256::from(1u64),
        new_value: U256::from(2u64),
    };
    contract(
        "events",
        vec![
            function_case(
                "setValue",
                "setValue",
                "setValue(uint256)",
                &(U256::from(2u64),),
                vec![num(2)],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getValue",
                "getValue",
                "getValue()",
                Some((&U256::from(2u64), num(2))),
            ),
        ],
        vec![],
        // Argument keys are the `.sol` spellings, which is what the ABI carries
        // and therefore what `decodeEventLog` returns.
        vec![event_case(
            "ValueChanged",
            &event,
            vec![
                ("who", addr_json()),
                ("oldValue", num(1)),
                ("newValue", num(2)),
            ],
            vec![
                ("who", addr_json()),
                ("oldValue", num(1)),
                ("newValue", num(2)),
            ],
        )],
    )
}

fn error_handling() -> ContractFixture {
    contract(
        "error-handling",
        vec![
            void("willRevert", "willRevert", "willRevert()"),
            function_case_noargs(
                "willSucceed",
                "willSucceed",
                "willSucceed()",
                Some((&true, json!(true))),
            ),
            function_case(
                "setGuarded",
                "setGuarded",
                "setGuarded(uint256)",
                &(U256::from(5u64),),
                vec![num(5)],
                None::<(&bool, _)>,
            ),
            function_case_noargs(
                "getGuarded",
                "getGuarded",
                "getGuarded()",
                Some((&U256::from(5u64), num(5))),
            ),
        ],
        vec![
            // Encoded through the enum, exactly as dispatch does, to prove the
            // wire selector is the held variant's and not the enum's (zeroed).
            error_case(
                "AlwaysReverts",
                "AlwaysReverts",
                "AlwaysReverts()",
                &ContractError::AlwaysReverts(AlwaysReverts),
                vec![],
            ),
            error_case(
                "ZeroNotAllowed",
                "ZeroNotAllowed",
                "ZeroNotAllowed()",
                &ContractError::ZeroNotAllowed(ZeroNotAllowed),
                vec![],
            ),
        ],
        vec![],
    )
}

fn error_caller() -> ContractFixture {
    contract(
        "error-caller",
        vec![function_case(
            "callError",
            "callError",
            "callError(address)",
            &(addr(),),
            vec![addr_json()],
            None::<(&bool, _)>,
        )],
        vec![error_case(
            "RevertString",
            "Error",
            "Error(string)",
            &RevertString("call failed".to_string()),
            vec![json!("call failed")],
        )],
        vec![],
    )
}

fn payable() -> ContractFixture {
    contract(
        "payable",
        vec![
            void("deposit", "deposit", "deposit()"),
            function_case(
                "depositTo",
                "depositTo",
                "depositTo(address)",
                &(addr(),),
                vec![addr_json()],
                None::<(&bool, _)>,
            ),
            function_case(
                "transfer",
                "transfer",
                "transfer(address,uint256)",
                &(addr(), U256::from(1_000u64)),
                vec![addr_json(), num(1_000)],
                Some((&true, json!(true))),
            ),
            function_case(
                "balanceOf",
                "balanceOf",
                "balanceOf(address)",
                &(addr(),),
                vec![addr_json()],
                Some((&U256::from(1_000u64), num(1_000))),
            ),
        ],
        vec![],
        vec![],
    )
}

fn receive() -> ContractFixture {
    contract(
        "receive",
        vec![
            function_case_noargs(
                "totalReceived",
                "totalReceived",
                "totalReceived()",
                Some((&U256::from(5u64), num(5))),
            ),
            function_case_noargs(
                "receiveCount",
                "receiveCount",
                "receiveCount()",
                Some((&U256::from(1u64), num(1))),
            ),
        ],
        vec![],
        vec![],
    )
}

/// The one fixture whose ABI file is a `{"abi":…,"storageLayout":…}` object
/// rather than a bare array, and whose ABI comes from the Rust emitter run
/// through the builder rather than from a `.sol` interface.
fn mytoken_storage() -> ContractFixture {
    let recipient = Address([0x11; 20]);
    ContractFixture {
        name: "mytoken-storage".to_string(),
        abi_file: "abi/mytoken-storage.abi.json".to_string(),
        wrapped: true,
        functions: vec![
            function_case_noargs(
                "totalSupply",
                "totalSupply",
                "totalSupply()",
                Some((&U256::from(1_000_000u64), num(1_000_000))),
            ),
            function_case(
                "balanceOf",
                "balanceOf",
                "balanceOf(address)",
                &(addr(),),
                vec![addr_json()],
                Some((&U256::from(500u64), num(500))),
            ),
            function_case(
                "transfer",
                "transfer",
                "transfer(address,uint256)",
                &(recipient, U256::from(250u64)),
                vec![bytes(&[0x11; 20]), num(250)],
                None::<(&bool, _)>,
            ),
            function_case(
                "mint",
                "mint",
                "mint(address,uint256)",
                &(addr(), U256::MAX),
                vec![addr_json(), num(U256::MAX)],
                None::<(&bool, _)>,
            ),
        ],
        errors: vec![error_case(
            "InsufficientBalance",
            "InsufficientBalance",
            "InsufficientBalance()",
            &mytoken::InsufficientBalance,
            vec![],
        )],
        events: vec![event_case(
            "Transfer",
            &mytoken::Transfer {
                from: Address([0u8; 20]),
                to: addr(),
                value: U256::from(1_000u64),
            },
            vec![
                ("from", bytes(&[0u8; 20])),
                ("to", addr_json()),
                ("value", num(1_000)),
            ],
            vec![
                ("from", bytes(&[0u8; 20])),
                ("to", addr_json()),
                ("value", num(1_000)),
            ],
        )],
    }
}

fn abi_surface() -> ContractFixture {
    let profile = Profile {
        id: U256::from(9u64),
        name: "profile".to_string(),
        tags: vec![1u32, 2, 3],
    };
    let profile_json = json!({ "id": "9", "name": "profile", "tags": ["1", "2", "3"] });

    let functions = vec![
        function_case_noargs("version", "version", "version()", Some((&1u32, num(1)))),
        function_case(
            "echoInts",
            "echoInts",
            "echoInts(int8,int16,int32,int64,int128,int256)",
            &(i8::MIN, i16::MIN, -1i32, i64::MIN, i128::MAX, I256::MIN),
            vec![
                num(i8::MIN),
                num(i16::MIN),
                num(-1),
                num(i64::MIN),
                num(i128::MAX),
                num(I256::MIN),
            ],
            Some((&I256::MINUS_ONE, num(I256::MINUS_ONE))),
        ),
        function_case(
            "echoBytesN",
            "echoBytesN",
            "echoBytesN(bytes1,bytes4,bytes20,bytes32)",
            &([0xabu8; 1], [0x01u8, 0x02, 0x03, 0x04], ADDR_BYTES, TAG),
            vec![
                bytes(&[0xab]),
                bytes(&[1, 2, 3, 4]),
                bytes(&ADDR_BYTES),
                bytes(&TAG),
            ],
            Some((
                &([0x01u8, 0x02, 0x03, 0x04], TAG),
                json!([bytes(&[1, 2, 3, 4]), bytes(&TAG)]),
            )),
        ),
        // The same three bytes as `bytes` and as `uint8[]`: identical Rust
        // shapes, deliberately different wire layouts.
        function_case(
            "echoBytesVsUint8",
            "echoBytesVsUint8",
            "echoBytesVsUint8(bytes,uint8[])",
            &(Bytes(vec![1, 2, 3]), vec![1u8, 2, 3]),
            vec![bytes(&[1, 2, 3]), json!(["1", "2", "3"])],
            Some((
                &(Bytes(vec![1, 2, 3]), vec![1u8, 2, 3]),
                json!([bytes(&[1, 2, 3]), ["1", "2", "3"]]),
            )),
        ),
        function_case(
            "echoStrings",
            "echoStrings",
            "echoStrings(string[])",
            &(vec![
                String::new(),
                "abcdefghijklmnopqrstuvwxyz0123456".to_string(),
            ],),
            vec![json!(["", "abcdefghijklmnopqrstuvwxyz0123456"])],
            Some((
                &vec![
                    String::new(),
                    "abcdefghijklmnopqrstuvwxyz0123456".to_string(),
                ],
                json!(["", "abcdefghijklmnopqrstuvwxyz0123456"]),
            )),
        ),
        function_case(
            "echoPairs",
            "echoPairs",
            "echoPairs((uint64,uint64)[])",
            &(vec![
                Pair { lo: 1, hi: 2 },
                Pair {
                    lo: u64::MAX,
                    hi: 0,
                },
            ],),
            vec![json!([
                { "lo": "1", "hi": "2" },
                { "lo": u64::MAX.to_string(), "hi": "0" }
            ])],
            Some((
                &vec![Pair { lo: 1, hi: 2 }],
                json!([{ "lo": "1", "hi": "2" }]),
            )),
        ),
        function_case(
            "echoFixedStrings",
            "echoFixedStrings",
            "echoFixedStrings(string[2])",
            &(["".to_string(), "tail".to_string()],),
            vec![json!(["", "tail"])],
            Some((&["".to_string(), "tail".to_string()], json!(["", "tail"]))),
        ),
        function_case(
            "echoFixedUints",
            "echoFixedUints",
            "echoFixedUints(uint256[3])",
            &([U256::ZERO, U256::from(1u64), U256::MAX],),
            vec![json!(["0", "1", U256::MAX.to_string()])],
            Some((
                &[U256::ZERO, U256::from(1u64), U256::MAX],
                json!(["0", "1", U256::MAX.to_string()]),
            )),
        ),
        // Multi-return with a dynamic member: two outputs, head/tail split.
        function_case(
            "mixed",
            "mixed",
            "mixed(uint256,string)",
            &(U256::from(3u64), "three".to_string()),
            vec![num(3), json!("three")],
            Some((
                &(U256::from(3u64), "three".to_string()),
                json!(["3", "three"]),
            )),
        ),
        // Single tuple output with named components: viem returns an object.
        function_case(
            "echoPair",
            "echoPair",
            "echoPair((uint64,uint64))",
            &(Pair { lo: 7, hi: 8 },),
            vec![json!({ "lo": "7", "hi": "8" })],
            Some((&Pair { lo: 7, hi: 8 }, json!({ "lo": "7", "hi": "8" }))),
        ),
        function_case(
            "echoProfile",
            "echoProfile",
            "echoProfile((uint256,string,uint32[]))",
            &(Profile {
                id: U256::from(9u64),
                name: "profile".to_string(),
                tags: vec![1u32, 2, 3],
            },),
            vec![profile_json.clone()],
            Some((&profile, profile_json.clone())),
        ),
        void("touch", "touch", "touch()"),
        void("deposit", "deposit", "deposit()"),
        void("alwaysFails", "alwaysFails", "alwaysFails()"),
        void("guarded", "guarded", "guarded()"),
        // Overloads: same ABI name, different selectors.
        function_case(
            "overloaded/uint256",
            "overloaded",
            "overloaded(uint256)",
            &(U256::from(5u64),),
            vec![num(5)],
            Some((&U256::from(5u64), num(5))),
        ),
        function_case(
            "overloaded/string",
            "overloaded",
            "overloaded(string)",
            &("hello".to_string(),),
            vec![json!("hello")],
            Some((&U256::from(5u64), num(5))),
        ),
    ];

    let errors = vec![
        error_case(
            "Unauthorized",
            "Unauthorized",
            "Unauthorized()",
            &SurfaceError::Unauthorized(Unauthorized),
            vec![],
        ),
        error_case(
            "InsufficientBalance",
            "InsufficientBalance",
            "InsufficientBalance(address,uint256,uint256)",
            &SurfaceError::InsufficientBalance(InsufficientBalance {
                account: addr(),
                required: U256::from(100u64),
                available: U256::from(40u64),
            }),
            vec![addr_json(), num(100), num(40)],
        ),
        // Dynamic field: the revert payload has its own head/tail split.
        error_case(
            "DetailedFailure",
            "DetailedFailure",
            "DetailedFailure(string,uint32)",
            &SurfaceError::DetailedFailure(DetailedFailure {
                reason: "insufficient allowance for transfer".to_string(),
                code: 7,
            }),
            vec![json!("insufficient allowance for transfer"), num(7)],
        ),
        error_case(
            "Panic/overflow",
            "Panic",
            "Panic(uint256)",
            &SurfaceError::Panic(Panic::Overflow),
            vec![num(0x11)],
        ),
        error_case(
            "Panic/division-by-zero",
            "Panic",
            "Panic(uint256)",
            &SurfaceError::Panic(Panic::DivisionByZero),
            vec![num(0x12)],
        ),
        error_case(
            "RevertString",
            "Error",
            "Error(string)",
            &SurfaceError::Revert(RevertString("guard tripped".to_string())),
            vec![json!("guard tripped")],
        ),
        // The OZ-compatible guard error, registered because a method carries
        // `#[non_reentrant]`. Its selector has to match OZ v5 byte for byte or
        // Foundry and Etherscan will not name it.
        error_case(
            "ReentrancyGuardReentrantCall",
            "ReentrancyGuardReentrantCall",
            "ReentrancyGuardReentrantCall()",
            &pvm_contract_sdk::ReentrancyGuardReentrantCall,
            vec![],
        ),
        // Framework errors are appended to every ABI, so a dispatch-level
        // revert is decodable by viem too.
        error_case(
            "UnknownSelector",
            "UnknownSelector",
            "UnknownSelector()",
            &FrameworkUnknownSelector,
            vec![],
        ),
    ];

    let events = vec![
        event_case(
            "Indexed3",
            &Indexed3 {
                who: addr(),
                amount: U256::MAX,
                tag: TAG,
                note: 42,
            },
            vec![
                ("who", addr_json()),
                ("amount", num(U256::MAX)),
                ("tag", bytes(&TAG)),
                ("note", num(42)),
            ],
            vec![
                ("who", addr_json()),
                ("amount", num(U256::MAX)),
                ("tag", bytes(&TAG)),
                ("note", num(42)),
            ],
        ),
        // Indexed dynamic fields hash into their topic, so `decoded` carries
        // the hash while `args` carries the value `encodeEventTopics` takes.
        {
            let event = IndexedDynamic {
                name: "alice".to_string(),
                payload: Bytes(vec![0xde, 0xad]),
                note: "transfer approved".to_string(),
            };
            let topics = event.topics();
            let name_hash = crate::hex(&topics.as_slice()[1]);
            let payload_hash = crate::hex(&topics.as_slice()[2]);
            event_case(
                "IndexedDynamic",
                &event,
                vec![
                    ("name", json!("alice")),
                    ("payload", bytes(&[0xde, 0xad])),
                    ("note", json!("transfer approved")),
                ],
                vec![
                    ("name", json!(name_hash)),
                    ("payload", json!(payload_hash)),
                    ("note", json!("transfer approved")),
                ],
            )
        },
        // An indexed static composite hashes to keccak256(abi.encode(value)).
        {
            let event = IndexedComposite {
                pair: Pair { lo: 1, hi: 2 },
                values: vec![U256::from(3u64), U256::from(4u64)],
            };
            let topics = event.topics();
            let pair_hash = crate::hex(&topics.as_slice()[1]);
            event_case(
                "IndexedComposite",
                &event,
                vec![
                    ("pair", json!({ "lo": "1", "hi": "2" })),
                    ("values", json!(["3", "4"])),
                ],
                vec![("pair", json!(pair_hash)), ("values", json!(["3", "4"]))],
            )
        },
        // Anonymous: no signature topic at all.
        event_case(
            "AnonymousPing",
            &AnonymousPing {
                a: U256::from(1u64),
                b: addr(),
                c: true,
            },
            vec![("a", num(1)), ("b", addr_json()), ("c", json!(true))],
            vec![("a", num(1)), ("b", addr_json()), ("c", json!(true))],
        ),
    ];

    ContractFixture {
        name: "abi-surface".to_string(),
        abi_file: "abi/abi-surface.abi.json".to_string(),
        wrapped: false,
        functions,
        errors,
        events,
    }
}

/// The `.sol`-parser counterpart of `abi-surface`: the same rich shapes, but
/// with the ABI derived from a Solidity interface instead of from Rust types.
/// Agreement between the two paths is the point — a divergence in either
/// direction shows up as a selector or byte mismatch here.
fn sol_type_surface() -> ContractFixture {
    let pair = Pair { lo: 1, hi: 2 };
    let pair_json = json!({ "lo": "1", "hi": "2" });
    let profile = Profile {
        id: U256::from(9u64),
        name: "profile".to_string(),
        tags: vec![1u32, 2, 3],
    };
    let profile_json = json!({ "id": "9", "name": "profile", "tags": ["1", "2", "3"] });
    let nested = Nested {
        inner: Pair { lo: 3, hi: 4 },
        owner: addr(),
    };
    // `.sol` component names, unlike the parameter cases which use the Rust ones.
    let nested_json = json!({ "inner": { "lo": "3", "hi": "4" }, "owner": addr_json() });
    let bundle = Bundle {
        dynamic_pairs: vec![Pair { lo: 1, hi: 2 }],
        fixed_pairs: [Pair { lo: 3, hi: 4 }, Pair { lo: 5, hi: 6 }],
        tags: [[1u8, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]],
    };
    let bundle_json = json!({
        "dynamicPairs": [{ "lo": "1", "hi": "2" }],
        "fixedPairs": [{ "lo": "3", "hi": "4" }, { "lo": "5", "hi": "6" }],
        "tags": ["0x01020304", "0x05060708", "0x090a0b0c"],
    });

    let functions = vec![
        // `uint` / `int` without a width are the 256-bit forms.
        function_case(
            "bareAliases",
            "bareAliases",
            "bareAliases(uint256,int256)",
            &(U256::MAX, I256::MIN),
            vec![num(U256::MAX), num(I256::MIN)],
            Some((
                &(U256::ZERO, I256::MAX),
                json!(["0", I256::MAX.to_string()]),
            )),
        ),
        function_case(
            "elementary",
            "elementary",
            "elementary(address,bool,string,bytes,bytes1,bytes17,bytes32,uint8,uint128,int8,int256)",
            &(
                addr(),
                true,
                "hello".to_string(),
                Bytes(vec![0xde, 0xad]),
                [0xabu8; 1],
                [0x11u8; 17],
                TAG,
                u8::MAX,
                u128::MAX,
                i8::MIN,
                I256::MINUS_ONE,
            ),
            vec![
                addr_json(),
                json!(true),
                json!("hello"),
                bytes(&[0xde, 0xad]),
                bytes(&[0xab]),
                bytes(&[0x11u8; 17]),
                bytes(&TAG),
                num(u8::MAX),
                num(u128::MAX),
                num(i8::MIN),
                num(I256::MINUS_ONE),
            ],
            Some((&true, json!(true))),
        ),
        // An enum resolves to `uint8` and a user-defined value type to its
        // underlying elementary type, so both are ordinary integers on the wire.
        function_case(
            "userDefined",
            "userDefined",
            "userDefined(uint8,uint64)",
            &(2u8, 1_700_000_000u64),
            vec![num(2), num(1_700_000_000u64)],
            Some((&1u8, num(1))),
        ),
        function_case(
            "takesPair",
            "takesPair",
            "takesPair((uint64,uint64))",
            &(Pair { lo: 1, hi: 2 },),
            vec![pair_json.clone()],
            Some((&pair, pair_json.clone())),
        ),
        function_case(
            "takesNested",
            "takesNested",
            "takesNested(((uint64,uint64),address))",
            &(Nested {
                inner: Pair { lo: 3, hi: 4 },
                owner: addr(),
            },),
            vec![nested_json.clone()],
            Some((&nested, nested_json.clone())),
        ),
        function_case(
            "takesProfile",
            "takesProfile",
            "takesProfile((uint256,string,uint32[]))",
            &(Profile {
                id: U256::from(9u64),
                name: "profile".to_string(),
                tags: vec![1u32, 2, 3],
            },),
            vec![profile_json.clone()],
            Some((&profile, profile_json.clone())),
        ),
        function_case(
            "takesBundle",
            "takesBundle",
            "takesBundle(((uint64,uint64)[],(uint64,uint64)[2],bytes4[3]))",
            &(Bundle {
                dynamic_pairs: vec![Pair { lo: 1, hi: 2 }],
                fixed_pairs: [Pair { lo: 3, hi: 4 }, Pair { lo: 5, hi: 6 }],
                tags: [[1u8, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]],
            },),
            vec![bundle_json.clone()],
            Some((&bundle, bundle_json.clone())),
        ),
        // A single tuple parameter with unnamed components: positional in viem.
        function_case(
            "inlineTuple",
            "inlineTuple",
            "inlineTuple((uint256,bool))",
            &((U256::from(99u64), true),),
            vec![json!(["99", true])],
            Some((&(U256::from(99u64), true), json!(["99", true]))),
        ),
        // Array suffix order has to survive the parser's recursion:
        // `uint256[][2]` is two dynamic arrays, not an array of two.
        function_case(
            "nestedArrays",
            "nestedArrays",
            "nestedArrays(uint256[][2],string[][])",
            &(
                [vec![U256::from(1u64), U256::from(2u64)], Vec::<U256>::new()],
                vec![vec!["a".to_string()], vec![]],
            ),
            vec![json!([["1", "2"], []]), json!([["a"], []])],
            Some((
                &vec![[U256::ZERO, U256::from(1u64), U256::from(2u64)]],
                json!([["0", "1", "2"]]),
            )),
        ),
        function_case(
            "userDefinedArrays",
            "userDefinedArrays",
            "userDefinedArrays(uint8[],uint64[2])",
            &(vec![0u8, 1, 2], [10u64, 20]),
            vec![json!(["0", "1", "2"]), json!(["10", "20"])],
            Some((&vec![2u8, 1], json!(["2", "1"]))),
        ),
    ];

    let errors = vec![
        error_case(
            "Unauthorized",
            "Unauthorized",
            "Unauthorized()",
            &sol_surface::Unauthorized,
            vec![],
        ),
        error_case(
            "InsufficientBalance",
            "InsufficientBalance",
            "InsufficientBalance(address,uint256,uint256)",
            &sol_surface::InsufficientBalance {
                account: addr(),
                required: U256::from(100u64),
                available: U256::from(40u64),
            },
            vec![addr_json(), num(100), num(40)],
        ),
        error_case(
            "DetailedFailure",
            "DetailedFailure",
            "DetailedFailure(string,uint32)",
            &sol_surface::DetailedFailure {
                reason: "not allowed".to_string(),
                code: 9,
            },
            vec![json!("not allowed"), num(9)],
        ),
        error_case(
            "CompositeFailure",
            "CompositeFailure",
            "CompositeFailure((uint64,uint64),uint256[])",
            &sol_surface::CompositeFailure {
                p: Pair { lo: 1, hi: 2 },
                values: vec![U256::from(3u64), U256::from(4u64)],
            },
            vec![pair_json.clone(), json!(["3", "4"])],
        ),
    ];

    let indexed_dynamic = sol_surface::IndexedDynamic {
        name: "alice".to_string(),
        payload: Bytes(vec![0xde, 0xad]),
        note: "note".to_string(),
    };
    let indexed_dynamic_topics = indexed_dynamic.topics();
    let name_hash = crate::hex(&indexed_dynamic_topics.as_slice()[1]);
    let payload_hash = crate::hex(&indexed_dynamic_topics.as_slice()[2]);

    let indexed_composite = sol_surface::IndexedComposite {
        p: Pair { lo: 1, hi: 2 },
        values: vec![U256::from(3u64)],
    };
    let pair_hash = crate::hex(&indexed_composite.topics().as_slice()[1]);

    let events = vec![
        event_case(
            "Simple",
            &sol_surface::Simple {
                who: addr(),
                amount: U256::from(7u64),
            },
            vec![("who", addr_json()), ("amount", num(7))],
            vec![("who", addr_json()), ("amount", num(7))],
        ),
        event_case(
            "IndexedDynamic",
            &indexed_dynamic,
            vec![
                ("name", json!("alice")),
                ("payload", bytes(&[0xde, 0xad])),
                ("note", json!("note")),
            ],
            vec![
                ("name", json!(name_hash)),
                ("payload", json!(payload_hash)),
                ("note", json!("note")),
            ],
        ),
        event_case(
            "IndexedComposite",
            &indexed_composite,
            vec![("p", pair_json.clone()), ("values", json!(["3"]))],
            vec![("p", json!(pair_hash)), ("values", json!(["3"]))],
        ),
        event_case(
            "ThreeIndexed",
            &sol_surface::ThreeIndexed {
                a: addr(),
                b: U256::MAX,
                c: TAG,
                d: 42,
            },
            vec![
                ("a", addr_json()),
                ("b", num(U256::MAX)),
                ("c", bytes(&TAG)),
                ("d", num(42)),
            ],
            vec![
                ("a", addr_json()),
                ("b", num(U256::MAX)),
                ("c", bytes(&TAG)),
                ("d", num(42)),
            ],
        ),
        event_case(
            "Anonymous",
            &sol_surface::Anonymous {
                a: U256::from(1u64),
                b: addr(),
                c: true,
            },
            vec![("a", num(1)), ("b", addr_json()), ("c", json!(true))],
            vec![("a", num(1)), ("b", addr_json()), ("c", json!(true))],
        ),
    ];

    ContractFixture {
        name: "sol-type-surface".to_string(),
        abi_file: "abi/sol-type-surface.abi.json".to_string(),
        wrapped: false,
        functions,
        errors,
        events,
    }
}

/// The framework's `UnknownSelector()` revert as a `SolError`, so it can go
/// through the same fixture path as user-declared errors. The framework itself
/// writes the raw selector rather than going through a type.
struct FrameworkUnknownSelector;

impl SolError for FrameworkUnknownSelector {
    const SELECTOR: [u8; 4] = pvm_contract_sdk::framework_errors::UNKNOWN_SELECTOR;
    const SIGNATURE: &'static str = "UnknownSelector()";

    fn encoded_size(&self) -> usize {
        4
    }

    fn encode_to(&self, buf: &mut [u8]) -> usize {
        buf[..4].copy_from_slice(&Self::SELECTOR);
        4
    }

    fn decode_at(
        input: &[u8],
        offset: usize,
    ) -> Result<Option<Self>, pvm_contract_sdk::DecodeError> {
        match input.get(offset..offset + 4) {
            Some(s) if s == Self::SELECTOR => Ok(Some(Self)),
            Some(_) => Ok(None),
            None => Err(pvm_contract_sdk::DecodeError),
        }
    }
}

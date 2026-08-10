//! Golden-vector generator for the viem round-trip suite.
//!
//! The TypeScript suite under `ts/viem-roundtrip` proves three things about the
//! ABI JSON this SDK emits:
//!
//! 1. viem can parse it at all (it is a valid `Abi`),
//! 2. viem's encoders and decoders agree byte-for-byte with the SDK's own
//!    [`SolEncode`] / [`SolError`] / [`SolEvent`] implementations,
//! 3. abitype infers the right TypeScript types from it.
//!
//! Points 1 and 3 need only the emitted `.abi.json`. Point 2 needs vectors, and
//! those vectors must be produced by the SDK itself — comparing viem against a
//! third encoder (alloy) would test alloy, and hand-written hex would silently
//! rot the moment codegen changes. So this crate walks a fixed corpus of Rust
//! values, encodes each one with the very traits the dispatch codegen calls,
//! and writes the results as JSON for the TypeScript side to check.
//!
//! Everything here is host-side: no riscv target, no PolkaVM link, no node.

use pvm_contract_types::{AbiParam, SolDecode, SolEncode, SolError, SolEvent, const_selector};
use serde::Serialize;

pub mod corpus;
pub mod surface;

// ---------------------------------------------------------------------------
// Fixture schema
//
// Field names are camelCase because the TypeScript loader consumes them
// directly. Every 256-bit integer travels as a decimal *string*: a JSON number
// silently loses precision past 2^53, which would make oversized vectors pass
// for the wrong reason.
// ---------------------------------------------------------------------------

/// Everything the TypeScript suite loads, in one file.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixtures {
    /// Raw `encodeAbiParameters` / `decodeAbiParameters` round-trips.
    ///
    /// Deliberately not an exhaustive type matrix: `pvm-contract-types::tests`
    /// already pins every primitive and the common container shapes against
    /// alloy-core, much of it under proptest, so restating them here would add
    /// hand-written vectors to keep in sync without adding a claim. What is left
    /// is the delta — composites and boundaries that differential does not
    /// reach — plus one value per `SOL_NAME` family, because these cases build
    /// their `types` from [`SolEncode::abi_param`] and are the only place that
    /// descriptor is checked against viem.
    pub parameters: Vec<ParameterCase>,
    /// Per-contract function, error and event vectors, keyed to an emitted ABI.
    pub contracts: Vec<ContractFixture>,
}

/// One `encodeAbiParameters(types, values)` round-trip.
///
/// `types` is a list because a Rust tuple models Solidity's *multiple*
/// parameters rather than a single `tuple` parameter — `(A, B)` encodes flat,
/// exactly as `f(A, B)`'s argument list does. A `#[derive(SolType)]` struct, by
/// contrast, is one `tuple` parameter and yields a single entry.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterCase {
    /// Stable identifier, used as the test name.
    pub id: String,
    /// viem `AbiParameter` descriptors, produced by [`SolEncode::abi_param`].
    pub types: Vec<AbiParam>,
    /// One JSON value per entry in `types`, in viem's input shape.
    pub values: Vec<serde_json::Value>,
    /// `0x`-prefixed output of [`SolEncode::encode_to`].
    pub encoded: String,
}

/// The vectors belonging to one emitted ABI file.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractFixture {
    /// Fixture-local name; also the export name in the generated `abis.ts`.
    pub name: String,
    /// Path of the ABI JSON relative to the fixtures directory.
    pub abi_file: String,
    /// Whether the ABI file is a bare array or a `{"abi":…,"storageLayout":…}`
    /// object. The loader has to unwrap the latter before handing it to viem.
    pub wrapped: bool,
    pub functions: Vec<FunctionCase>,
    pub errors: Vec<ErrorCase>,
    pub events: Vec<EventCase>,
}

/// One function's calldata and/or return-data vector.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCase {
    pub id: String,
    /// Name as it appears in the ABI (camelCase, post-`rename`).
    pub function_name: String,
    /// Canonical signature the selector is computed from.
    pub signature: String,
    /// `0x`-prefixed 4-byte selector.
    pub selector: String,
    /// Arguments in viem's input shape, one per ABI input.
    pub args: Vec<serde_json::Value>,
    /// `0x`-prefixed selector ++ encoded arguments.
    pub calldata: String,
    /// What `decodeFunctionResult` must return: the bare value for a single
    /// output, an array for several, and absent when the function returns
    /// nothing. viem's two shapes are deliberately not normalised here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// `0x`-prefixed return data, absent when the function returns nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returndata: Option<String>,
}

/// One revert payload as the contract would produce it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorCase {
    pub id: String,
    pub error_name: String,
    pub signature: String,
    pub selector: String,
    /// Arguments in viem's input shape, one per ABI input.
    pub args: Vec<serde_json::Value>,
    /// `0x`-prefixed selector ++ encoded fields.
    pub data: String,
}

/// One emitted log.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCase {
    pub id: String,
    pub event_name: String,
    pub signature: String,
    /// `0x`-prefixed topics, `topics[0]` being the signature hash unless the
    /// event is anonymous.
    pub topics: Vec<String>,
    /// `0x`-prefixed ABI-encoded non-indexed fields.
    pub data: String,
    /// Field values keyed by name, in the shape `encodeEventTopics` accepts.
    pub args: serde_json::Map<String, serde_json::Value>,
    /// What `decodeEventLog` must return. Differs from `args` for indexed
    /// dynamic fields, where the topic is `keccak256(value)` and the original
    /// value is not recoverable from the log.
    pub decoded: serde_json::Map<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

/// Format bytes as a `0x`-prefixed lowercase hex string.
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Top-level ABI encoding of `value`, exactly as the dispatch codegen produces
/// it for return data and as the argument tail of calldata.
pub fn encode<T: SolEncode>(value: &T) -> Vec<u8> {
    let mut buf = vec![0u8; value.encode_len()];
    value.encode_to(&mut buf);
    buf
}

/// The viem `AbiParameter` list corresponding to `T`.
///
/// A Rust tuple is Solidity's parameter *list*, so its components are lifted to
/// the top level; anything else is a single parameter.
pub fn param_types<T: SolEncode>() -> Vec<AbiParam> {
    let param = T::abi_param("");
    if T::IS_TUPLE {
        param.components
    } else {
        vec![param]
    }
}

/// Build a [`ParameterCase`] from a Rust value and its viem-shaped JSON.
///
/// `json` must be an array when `T` is a tuple (one element per component) and
/// the bare value otherwise. Getting that wrong is caught by the suite: viem
/// would encode different bytes than [`SolEncode`] did.
///
/// `T` is also required to be [`SolDecode`] so that every case asserts the SDK's
/// own decoder is the inverse of its encoder. viem cannot see that asymmetry —
/// it would happily agree with an encoding our own dispatch then misreads — so
/// the check belongs here, where it covers the whole corpus for free.
pub fn parameter_case<T>(id: &str, value: &T, json: serde_json::Value) -> ParameterCase
where
    T: SolEncode + SolDecode + PartialEq + core::fmt::Debug,
{
    let encoded = encode(value);
    let decoded = T::decode(&encoded).unwrap_or_else(|e| {
        panic!("parameter case `{id}`: SolDecode rejected our own encoding: {e:?}")
    });
    assert_eq!(
        &decoded, value,
        "parameter case `{id}`: SolDecode is not the inverse of SolEncode",
    );
    parameter_case_encode_only(id, value, json)
}

/// As [`parameter_case`], for the rare type that has no [`SolDecode`] impl.
pub fn parameter_case_encode_only<T: SolEncode>(
    id: &str,
    value: &T,
    json: serde_json::Value,
) -> ParameterCase {
    let types = param_types::<T>();
    let values = if T::IS_TUPLE {
        match json {
            serde_json::Value::Array(items) => items,
            other => panic!("parameter case `{id}`: tuple type needs an array value, got {other}"),
        }
    } else {
        vec![json]
    };
    assert_eq!(
        types.len(),
        values.len(),
        "parameter case `{id}`: {} types but {} values",
        types.len(),
        values.len(),
    );
    ParameterCase {
        id: id.to_string(),
        types,
        values,
        encoded: hex(&encode(value)),
    }
}

/// Build a [`FunctionCase`] for a function that takes `args` and returns
/// `result`.
///
/// `signature` is the canonical Solidity signature; the selector is derived
/// from it with the same `const_selector` the macro uses, so a disagreement
/// between the fixture and the emitted ABI shows up as a failing selector
/// assertion rather than as a confusing byte diff.
pub fn function_case<A: SolEncode, R: SolEncode>(
    id: &str,
    function_name: &str,
    signature: &str,
    args: &A,
    args_json: Vec<serde_json::Value>,
    result: Option<(&R, serde_json::Value)>,
) -> FunctionCase {
    let selector = const_selector(signature);
    let mut calldata = selector.to_vec();
    calldata.extend_from_slice(&encode(args));

    let (result_json, returndata) = match result {
        Some((value, json)) => (Some(json), Some(hex(&encode(value)))),
        None => (None, None),
    };

    FunctionCase {
        id: id.to_string(),
        function_name: function_name.to_string(),
        signature: signature.to_string(),
        selector: hex(&selector),
        args: args_json,
        calldata: hex(&calldata),
        result: result_json,
        returndata,
    }
}

/// A function with no arguments. Distinct from `function_case` with an empty
/// tuple because there is no zero-arity `SolEncode` tuple to pass.
pub fn function_case_noargs<R: SolEncode>(
    id: &str,
    function_name: &str,
    signature: &str,
    result: Option<(&R, serde_json::Value)>,
) -> FunctionCase {
    let selector = const_selector(signature);
    let (result_json, returndata) = match result {
        Some((value, json)) => (Some(json), Some(hex(&encode(value)))),
        None => (None, None),
    };

    FunctionCase {
        id: id.to_string(),
        function_name: function_name.to_string(),
        signature: signature.to_string(),
        selector: hex(&selector),
        args: vec![],
        calldata: hex(&selector),
        result: result_json,
        returndata,
    }
}

/// Build an [`ErrorCase`] from a value implementing [`SolError`].
///
/// `signature` is passed in rather than read from `E::SIGNATURE` because error
/// *enums* zero their own selector and signature — the wire selector belongs to
/// whichever variant is held. Naming the expected variant here is what makes
/// the enum-dispatch case meaningful.
pub fn error_case<E: SolError>(
    id: &str,
    error_name: &str,
    signature: &str,
    value: &E,
    args_json: Vec<serde_json::Value>,
) -> ErrorCase {
    let mut buf = vec![0u8; value.encoded_size()];
    let written = value.encode_to(&mut buf);
    buf.truncate(written);

    ErrorCase {
        id: id.to_string(),
        error_name: error_name.to_string(),
        signature: signature.to_string(),
        selector: hex(&const_selector(signature)),
        args: args_json,
        data: hex(&buf),
    }
}

/// Build an [`EventCase`] from a value implementing [`SolEvent`].
pub fn event_case<E: SolEvent>(
    id: &str,
    value: &E,
    args: Vec<(&str, serde_json::Value)>,
    decoded: Vec<(&str, serde_json::Value)>,
) -> EventCase {
    let topics = value.topics();
    let mut data = vec![0u8; value.data_len()];
    value.data_to(&mut data);

    EventCase {
        id: id.to_string(),
        event_name: E::NAME.to_string(),
        signature: E::SIGNATURE.to_string(),
        topics: topics.as_slice().iter().map(|t| hex(t)).collect(),
        data: hex(&data),
        args: args.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        decoded: decoded
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

/// JSON for a 256-bit-capable integer: always a decimal string.
pub fn num(value: impl std::fmt::Display) -> serde_json::Value {
    serde_json::Value::String(value.to_string())
}

/// JSON for a byte string (`bytesN`, `bytes`, `address`): `0x`-prefixed hex.
pub fn bytes(value: &[u8]) -> serde_json::Value {
    serde_json::Value::String(hex(value))
}

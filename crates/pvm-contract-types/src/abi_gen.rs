extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// A parameter in a Solidity ABI function signature.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AbiParam {
    /// Parameter name (empty string for unnamed outputs).
    pub name: String,
    /// Solidity type name (e.g. "uint256", "address", "tuple").
    #[serde(rename = "type")]
    pub param_type: String,
    /// For tuple types, the list of sub-parameters. Empty for primitives.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<AbiParam>,
}

/// A top-level item in a Solidity ABI JSON array.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AbiItem {
    Constructor {
        inputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        state_mutability: Option<String>,
    },
    Function {
        name: String,
        inputs: Vec<AbiParam>,
        outputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        state_mutability: Option<String>,
    },
    Error {
        name: String,
        inputs: Vec<AbiParam>,
    },
    Event {
        name: String,
        inputs: Vec<AbiEventParam>,
        anonymous: bool,
    },
    Receive {
        #[serde(rename = "stateMutability")]
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        state_mutability: Option<String>,
    },
}

/// A parameter in a Solidity ABI event signature.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AbiEventParam {
    /// Parameter name.
    pub name: String,
    /// Solidity type name (e.g. "uint256", "address", "tuple").
    #[serde(rename = "type")]
    pub param_type: String,
    /// For tuple types, the list of sub-parameters. Empty for primitives.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<AbiParam>,
    /// Whether this parameter is indexed (becomes a log topic).
    pub indexed: bool,
}

/// Wrapper for a complete ABI JSON array.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AbiJson(pub Vec<AbiItem>);

/// A single entry in the `storageLayout.storage` array.
///
/// Field order and naming match solc's `--storage-layout` JSON output so
/// downstream tooling (Hardhat, Foundry, Tenderly) consumes both
/// interchangeably.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StorageLayoutEntry {
    pub label: String,
    pub slot: String,
    /// Byte offset within the slot where this field begins. `0` for full-slot
    /// fields (the previous default); non-zero for packed sub-slot fields
    /// sharing a 32-byte word with neighbours. Matches solc's behavior.
    ///
    /// `#[serde(default)]` lets the SDK deserialize older `storageLayout`
    /// JSON (emitted before packing existed) without error — missing field
    /// reads as `0`, which is correct for any layout without packing.
    #[serde(default)]
    pub offset: u8,
    #[serde(rename = "type")]
    pub ty: String,
}

/// The top-level `storageLayout` object.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StorageLayout {
    pub storage: Vec<StorageLayoutEntry>,
}

/// Type-name resolver used by the storage-layout JSON emitter.
///
/// The `#[contract]` macro builds each leaf entry's `"type"` string by
/// recursively walking field types (`Lazy<T>` unwraps, `Mapping<K, V>` builds
/// `"mapping(K_name => V_name)"`). At leaf positions, the macro needs a way to
/// name the type — for primitives this is [`SolEncode::SOL_NAME`] (already
/// in this crate), for `#[storage]` sub-structs the macro emits an impl
/// returning the Rust ident, and for storage handles (`Lazy<T>`,
/// `Mapping<K, V>`, …) the impls live in `pvm-storage` and build their
/// names recursively from inner type names.
///
/// `name()` returns `String` rather than `&'static str` because the
/// recursive `Mapping` case has to `format!` together its inner type names,
/// which generics-using `const` items can't do. The layout-emit path only
/// runs under `--features abi-gen` (off-chain), so the allocation is free
/// from the contract's perspective.
pub trait StorageTypeName {
    /// Solidity-compat type name used in the `"type"` field of a
    /// `storageLayout` entry. Primitives return their solc name
    /// (`"uint256"`, `"address"`); `#[storage]` sub-structs and
    /// `#[derive(SolStorage)]` value structs return their Rust ident;
    /// `Lazy<T>` returns `T`'s name; `Mapping<K, V>` returns
    /// `"mapping(K_name => V_name)"`.
    ///
    /// **No `SolEncode` blanket impl.** A naive
    /// `impl<T: SolEncode> StorageTypeName for T` would let *any* ABI type
    /// claim a storage-layout name — including ABI-only structs that have
    /// no business appearing in `storageLayout` — and would emit the ABI
    /// tuple notation (`"(uint64,uint64)"`) for `#[derive(SolStorage)]`
    /// value structs instead of the Rust ident (parity break with the
    /// `#[storage]` attribute path, which returns the ident). Every
    /// storage-eligible type therefore provides an explicit impl —
    /// primitives in `storage_codec.rs` / `alloc_types.rs`, derived
    /// structs via `#[derive(SolStorage)]` and `#[storage]`, container
    /// handles via `pvm-storage`.
    fn name() -> String;
}

/// Serialize a [`StorageLayout`] to a JSON string.
pub fn storage_layout_to_json(layout: &StorageLayout) -> String {
    serde_json::to_string(layout).expect("StorageLayout serialization failed")
}

/// Serialize a list of ABI items to a JSON string.
pub fn abi_to_json(items: &[AbiItem]) -> String {
    serde_json::to_string_pretty(items).expect("ABI serialization failed")
}

/// Parse a Solidity type string (e.g. from an error signature) into an [`AbiParam`],
/// expanding inline tuples into `type: "tuple"` with nested `components`.
///
/// Examples:
/// - `"uint256"` → `AbiParam { param_type: "uint256", components: [] }`
/// - `"(uint256,address)"` → `AbiParam { param_type: "tuple", components: [...] }`
/// - `"(uint256,address)[]"` → `AbiParam { param_type: "tuple[]", components: [...] }`
pub fn parse_type_str(name: &str, raw_type: &str) -> AbiParam {
    // Dynamic array: T[]
    if let Some(base) = raw_type.strip_suffix("[]") {
        let inner = parse_type_str("", base);
        return AbiParam {
            name: String::from(name),
            param_type: alloc::format!("{}[]", inner.param_type),
            components: inner.components,
        };
    }
    // Fixed array: T[N]
    if raw_type.ends_with(']')
        && let Some(bracket_start) = raw_type.rfind('[')
    {
        let base = &raw_type[..bracket_start];
        let suffix = &raw_type[bracket_start..];
        let inner = parse_type_str("", base);
        return AbiParam {
            name: String::from(name),
            param_type: alloc::format!("{}{}", inner.param_type, suffix),
            components: inner.components,
        };
    }
    // Inline tuple: (type1,type2,...)
    if raw_type.starts_with('(') && raw_type.ends_with(')') {
        let inner_str = &raw_type[1..raw_type.len() - 1];
        let components = split_top_level_params(inner_str)
            .into_iter()
            .map(|t| parse_type_str("", t.trim()))
            .collect();
        return AbiParam {
            name: String::from(name),
            param_type: String::from("tuple"),
            components,
        };
    }
    // Primitive
    AbiParam {
        name: String::from(name),
        param_type: String::from(raw_type),
        components: Vec::new(),
    }
}

/// Split a comma-separated parameter string, respecting nested parens.
fn split_top_level_params(s: &str) -> Vec<&str> {
    let mut params = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let p = s[start..i].trim();
                if !p.is_empty() {
                    params.push(p);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        params.push(last);
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse_type_str_primitive() {
        let p = parse_type_str("x", "uint256");
        assert_eq!(p.param_type, "uint256");
        assert_eq!(p.name, "x");
        assert!(p.components.is_empty());
    }

    #[test]
    fn parse_type_str_tuple() {
        let p = parse_type_str("val", "(uint256,address)");
        assert_eq!(p.param_type, "tuple");
        assert_eq!(p.components.len(), 2);
        assert_eq!(p.components[0].param_type, "uint256");
        assert_eq!(p.components[1].param_type, "address");
    }

    #[test]
    fn parse_type_str_tuple_array() {
        let p = parse_type_str("items", "(uint64,bool)[]");
        assert_eq!(p.param_type, "tuple[]");
        assert_eq!(p.components.len(), 2);
    }

    #[test]
    fn parse_type_str_tuple_fixed_array() {
        let p = parse_type_str("pts", "(uint64,uint64)[3]");
        assert_eq!(p.param_type, "tuple[3]");
        assert_eq!(p.components.len(), 2);
    }

    #[test]
    fn parse_type_str_nested_tuple() {
        let p = parse_type_str("line", "((uint64,uint64),(uint64,uint64))");
        assert_eq!(p.param_type, "tuple");
        assert_eq!(p.components.len(), 2);
        assert_eq!(p.components[0].param_type, "tuple");
        assert_eq!(p.components[0].components.len(), 2);
    }

    #[test]
    fn parse_type_str_plain_array() {
        let p = parse_type_str("ids", "uint256[]");
        assert_eq!(p.param_type, "uint256[]");
        assert!(p.components.is_empty());
    }

    #[test]
    fn abi_to_json_roundtrip() {
        let items = vec![AbiItem::Function {
            name: "foo".into(),
            inputs: vec![AbiParam {
                name: "x".into(),
                param_type: "tuple".into(),
                components: vec![
                    AbiParam {
                        name: "a".into(),
                        param_type: "uint256".into(),
                        components: vec![],
                    },
                    AbiParam {
                        name: "b".into(),
                        param_type: "address".into(),
                        components: vec![],
                    },
                ],
            }],
            outputs: vec![],
            state_mutability: Some("payable".into()),
        }];
        let json = abi_to_json(&items);
        let parsed: Vec<AbiItem> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, items);
    }

    #[test]
    fn storage_layout_to_json_matches_expected_shape() {
        let layout = StorageLayout {
            storage: vec![
                StorageLayoutEntry {
                    label: "total_supply".into(),
                    slot: "0".into(),
                    offset: 0,
                    ty: "uint256".into(),
                },
                StorageLayoutEntry {
                    label: "balances".into(),
                    slot: "1".into(),
                    offset: 0,
                    ty: "mapping(address => uint256)".into(),
                },
            ],
        };
        let json = storage_layout_to_json(&layout);

        assert_eq!(
            json,
            r#"{"storage":[{"label":"total_supply","slot":"0","offset":0,"type":"uint256"},{"label":"balances","slot":"1","offset":0,"type":"mapping(address => uint256)"}]}"#
        );

        // Roundtrip.
        let parsed: StorageLayout = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, layout);
    }

    /// Backward-compat: storageLayout JSON emitted before the `offset` field
    /// existed (i.e. without an `"offset"` key per entry) must deserialize
    /// without error, defaulting `offset = 0` on every entry. Pins the
    /// `#[serde(default)]` contract.
    #[test]
    fn storage_layout_from_json_without_offset_defaults_to_zero() {
        let legacy = r#"{
            "storage": [
                {"label": "total_supply", "slot": "0", "type": "uint256"},
                {"label": "balances", "slot": "1", "type": "mapping(address => uint256)"}
            ]
        }"#;
        let parsed: StorageLayout = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.storage.len(), 2);
        assert_eq!(parsed.storage[0].offset, 0);
        assert_eq!(parsed.storage[1].offset, 0);
    }

    /// Forward-emit: a layout containing a packed sub-slot field (offset != 0)
    /// serializes with the offset visible — the load-bearing JSON parity
    /// requirement for solc-compatible tooling.
    #[test]
    fn storage_layout_emits_non_zero_offsets_for_packed_fields() {
        let layout = StorageLayout {
            storage: vec![
                // Classic solc packing example: bool + uint32 + address fit in
                // slot 0. solc counts `offset` from the least-significant byte,
                // so the lower-order-aligned fields land at 0, 1, 5.
                StorageLayoutEntry {
                    label: "a".into(),
                    slot: "0".into(),
                    offset: 0,
                    ty: "bool".into(),
                },
                StorageLayoutEntry {
                    label: "b".into(),
                    slot: "0".into(),
                    offset: 1,
                    ty: "uint32".into(),
                },
                StorageLayoutEntry {
                    label: "c".into(),
                    slot: "0".into(),
                    offset: 5,
                    ty: "address".into(),
                },
                StorageLayoutEntry {
                    label: "d".into(),
                    slot: "1".into(),
                    offset: 0,
                    ty: "uint256".into(),
                },
            ],
        };
        let json = storage_layout_to_json(&layout);
        assert_eq!(
            json,
            r#"{"storage":[{"label":"a","slot":"0","offset":0,"type":"bool"},{"label":"b","slot":"0","offset":1,"type":"uint32"},{"label":"c","slot":"0","offset":5,"type":"address"},{"label":"d","slot":"1","offset":0,"type":"uint256"}]}"#
        );
        let parsed: StorageLayout = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, layout);
    }
}

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
}

/// Wrapper for a complete ABI JSON array.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AbiJson(pub Vec<AbiItem>);

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
}

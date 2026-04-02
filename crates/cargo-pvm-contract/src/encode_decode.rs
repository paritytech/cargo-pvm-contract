use alloy_core::dyn_abi::{DynSolType, DynSolValue};
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::path::Path;
use tiny_keccak::{Hasher, Keccak};

/// A decoded ABI parameter.
#[derive(Debug, PartialEq)]
pub struct DecodedParam {
    pub name: String,
    pub sol_type: String,
    pub value: String,
}

/// Load an ABI JSON file and return the parsed ABI array.
fn load_abi(abi_path: &Path) -> Result<Vec<Value>> {
    let content = std::fs::read_to_string(abi_path)
        .with_context(|| format!("Failed to read ABI file: {}", abi_path.display()))?;
    let abi: Vec<Value> = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse ABI JSON: {}", abi_path.display()))?;
    Ok(abi)
}

/// Compute the 4-byte Keccak-256 selector for a Solidity function signature.
fn selector_from_signature(sig: &str) -> [u8; 4] {
    let mut hasher = Keccak::v256();
    hasher.update(sig.as_bytes());
    let mut hash = [0u8; 32];
    hasher.finalize(&mut hash);
    let mut selector = [0u8; 4];
    selector.copy_from_slice(&hash[..4]);
    selector
}

/// Build the canonical Solidity function signature from ABI metadata.
/// e.g. "transfer(address,uint256)" or "processTuple((uint256,bool))"
fn build_function_signature(func: &Value) -> Result<String> {
    let name = func["name"]
        .as_str()
        .ok_or_else(|| anyhow!("ABI entry missing 'name'"))?;
    let inputs = func["inputs"]
        .as_array()
        .ok_or_else(|| anyhow!("ABI entry missing 'inputs'"))?;
    let param_types: Vec<String> = inputs
        .iter()
        .map(canonical_type)
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("{}({})", name, param_types.join(",")))
}

/// Build the canonical type string for an ABI input entry.
/// For tuple types, recursively expands components: `tuple` → `(uint256,bool)`.
fn canonical_type(input: &Value) -> Result<String> {
    let sol_type = input["type"]
        .as_str()
        .ok_or_else(|| anyhow!("Input missing 'type'"))?;
    if sol_type == "tuple" {
        let components = input["components"]
            .as_array()
            .ok_or_else(|| anyhow!("Tuple type missing 'components'"))?;
        let inner: Vec<String> = components
            .iter()
            .map(canonical_type)
            .collect::<Result<Vec<_>>>()?;
        Ok(format!("({})", inner.join(",")))
    } else if sol_type == "tuple[]" {
        let components = input["components"]
            .as_array()
            .ok_or_else(|| anyhow!("Tuple array type missing 'components'"))?;
        let inner: Vec<String> = components
            .iter()
            .map(canonical_type)
            .collect::<Result<Vec<_>>>()?;
        Ok(format!("({})[]", inner.join(",")))
    } else if let Some(suffix) = sol_type.strip_prefix("tuple[") {
        // Fixed-size tuple arrays: "tuple[N]" → "(T1,T2,...)[N]"
        let components = input["components"]
            .as_array()
            .ok_or_else(|| anyhow!("Tuple array type missing 'components'"))?;
        let inner: Vec<String> = components
            .iter()
            .map(canonical_type)
            .collect::<Result<Vec<_>>>()?;
        Ok(format!("({})[{}", inner.join(","), suffix))
    } else {
        Ok(sol_type.to_string())
    }
}

/// Find a function entry in the ABI by name or full signature.
///
/// Accepts either a bare name (`"transfer"`) or a full signature
/// (`"transfer(address,uint256)"`). When a bare name matches multiple
/// overloads, returns an error asking for the full signature.
fn find_function<'a>(abi: &'a [Value], function_id: &str) -> Result<&'a Value> {
    if function_id.contains('(') {
        // Full signature lookup
        abi.iter()
            .find(|entry| {
                entry["type"].as_str() == Some("function")
                    && build_function_signature(entry)
                        .map(|sig| sig == function_id)
                        .unwrap_or(false)
            })
            .ok_or_else(|| anyhow!("Function signature '{}' not found in ABI", function_id))
    } else {
        // Name-only lookup — error on ambiguous overloads
        let matches: Vec<&Value> = abi
            .iter()
            .filter(|entry| {
                entry["type"].as_str() == Some("function")
                    && entry["name"].as_str() == Some(function_id)
            })
            .collect();

        match matches.len() {
            0 => Err(anyhow!("Function '{}' not found in ABI", function_id)),
            1 => Ok(matches[0]),
            _ => Err(anyhow!(
                "Function '{}' is overloaded in the ABI. Specify the full signature, e.g. '{}(type1,type2,...)'.",
                function_id,
                function_id
            )),
        }
    }
}

/// Find a constructor entry in the ABI.
fn find_constructor(abi: &[Value]) -> Result<&Value> {
    abi.iter()
        .find(|entry| entry["type"].as_str() == Some("constructor"))
        .ok_or_else(|| anyhow!("Constructor not found in ABI"))
}

/// Convert an ABI JSON input entry to alloy's `DynSolType`.
fn abi_to_dyn_sol_type(input: &Value) -> Result<DynSolType> {
    let sol_type = input["type"]
        .as_str()
        .ok_or_else(|| anyhow!("Input missing 'type'"))?;

    // Fixed-size array types (e.g. "uint256[3]", "address[5]")
    if let Some(bracket_pos) = sol_type.rfind('[') {
        let inner_part = &sol_type[bracket_pos + 1..sol_type.len() - 1];
        let base = &sol_type[..bracket_pos];
        let mut base_input = serde_json::json!({"type": base});
        if let Some(comps) = input["components"].as_array() {
            base_input["components"] = Value::Array(comps.clone());
        }
        let inner = abi_to_dyn_sol_type(&base_input)?;
        if inner_part.is_empty() {
            // Dynamic array (e.g. "uint256[]")
            return Ok(DynSolType::Array(Box::new(inner)));
        } else {
            // Fixed-size array (e.g. "uint256[3]")
            let size: usize = inner_part
                .parse()
                .map_err(|_| anyhow!("Invalid array size in '{sol_type}'"))?;
            return Ok(DynSolType::FixedArray(Box::new(inner), size));
        }
    }

    match sol_type {
        "bool" => Ok(DynSolType::Bool),
        "address" => Ok(DynSolType::Address),
        "string" => Ok(DynSolType::String),
        "bytes" => Ok(DynSolType::Bytes),
        "tuple" => {
            let components = input["components"]
                .as_array()
                .ok_or_else(|| anyhow!("Tuple type missing 'components'"))?;
            let inner = components
                .iter()
                .map(abi_to_dyn_sol_type)
                .collect::<Result<_>>()?;
            Ok(DynSolType::Tuple(inner))
        }
        t if t.starts_with("uint") => {
            let bits: usize = t[4..].parse().unwrap_or(256);
            Ok(DynSolType::Uint(bits))
        }
        t if t.starts_with("int") => {
            let bits: usize = t[3..].parse().unwrap_or(256);
            Ok(DynSolType::Int(bits))
        }
        t if t.starts_with("bytes") => {
            let size: usize = t[5..]
                .parse()
                .map_err(|_| anyhow!("Invalid fixed bytes size in '{t}'"))?;
            Ok(DynSolType::FixedBytes(size))
        }
        _ => anyhow::bail!("Unsupported ABI type: {sol_type}"),
    }
}

/// Format a `DynSolValue` into its string representation.
fn format_value(value: &DynSolValue) -> String {
    match value {
        DynSolValue::Bool(b) => b.to_string(),
        DynSolValue::Uint(n, _) => n.to_string(),
        DynSolValue::Int(n, _) => n.to_string(),
        DynSolValue::Address(addr) => format!("0x{}", hex::encode(addr.as_slice())),
        DynSolValue::FixedBytes(word, size) => {
            format!("0x{}", hex::encode(&word.as_slice()[..*size]))
        }
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        DynSolValue::String(s) => s.clone(),
        DynSolValue::Array(vals) | DynSolValue::FixedArray(vals) => {
            let items: Vec<String> = vals.iter().map(format_value).collect();
            format!("[{}]", items.join(","))
        }
        DynSolValue::Tuple(vals) => {
            let items: Vec<String> = vals.iter().map(format_value).collect();
            format!("({})", items.join(","))
        }
        other => format!("{other:?}"),
    }
}

/// Encode a function call with arguments into ABI-encoded calldata.
pub fn encode_call(abi_path: &Path, function_name: &str, args: &[String]) -> Result<Vec<u8>> {
    let abi = load_abi(abi_path)?;
    let func = find_function(&abi, function_name)?;
    let sig = build_function_signature(func)?;
    let selector = selector_from_signature(&sig);

    let inputs = func["inputs"]
        .as_array()
        .ok_or_else(|| anyhow!("Missing inputs"))?;

    if inputs.len() != args.len() {
        anyhow::bail!(
            "Function '{}' expects {} arguments, got {}",
            function_name,
            inputs.len(),
            args.len()
        );
    }

    let encoded_params = encode_params(inputs, args)?;

    let mut calldata = selector.to_vec();
    calldata.extend(encoded_params);
    Ok(calldata)
}

/// Encode constructor arguments into ABI-encoded calldata (no selector).
pub fn encode_constructor(abi_path: &Path, args: &[String]) -> Result<Vec<u8>> {
    let abi = load_abi(abi_path)?;
    let constructor = find_constructor(&abi)?;

    let inputs = constructor["inputs"]
        .as_array()
        .ok_or_else(|| anyhow!("Missing constructor inputs"))?;

    if inputs.len() != args.len() {
        anyhow::bail!(
            "Constructor expects {} arguments, got {}",
            inputs.len(),
            args.len()
        );
    }

    encode_params(inputs, args)
}

fn encode_params(inputs: &[Value], args: &[String]) -> Result<Vec<u8>> {
    let types: Vec<DynSolType> = inputs
        .iter()
        .map(abi_to_dyn_sol_type)
        .collect::<Result<_>>()?;

    let values: Vec<DynSolValue> = types
        .iter()
        .zip(args.iter())
        .map(|(ty, arg)| {
            ty.coerce_str(arg)
                .map_err(|e| anyhow!("Failed to parse '{}' as {}: {}", arg, ty, e))
        })
        .collect::<Result<_>>()?;

    Ok(DynSolValue::Tuple(values).abi_encode_params())
}

/// Decode calldata using an ABI JSON file.
/// Returns (function_name, Vec<DecodedParam>).
pub fn decode_call(abi_path: &Path, calldata_hex: &str) -> Result<(String, Vec<DecodedParam>)> {
    let hex_str = calldata_hex.strip_prefix("0x").unwrap_or(calldata_hex);
    let calldata =
        hex::decode(hex_str).with_context(|| format!("Invalid hex calldata: {calldata_hex}"))?;

    if calldata.len() < 4 {
        anyhow::bail!("Calldata too short (less than 4 bytes for selector)");
    }

    let selector = &calldata[..4];
    let data = &calldata[4..];

    let abi = load_abi(abi_path)?;

    // Find the function that matches this selector
    let func = abi
        .iter()
        .filter(|entry| entry["type"].as_str() == Some("function"))
        .find(|entry| {
            build_function_signature(entry)
                .map(|sig| selector_from_signature(&sig) == selector)
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            anyhow!(
                "No function found matching selector 0x{}",
                hex::encode(selector)
            )
        })?;

    let function_name = func["name"]
        .as_str()
        .ok_or_else(|| anyhow!("Function missing name"))?
        .to_string();

    let inputs = func["inputs"]
        .as_array()
        .ok_or_else(|| anyhow!("Function missing inputs"))?;

    let decoded = decode_params(inputs, data)?;
    Ok((function_name, decoded))
}

/// Decode constructor calldata using an ABI JSON file.
pub fn decode_constructor(abi_path: &Path, calldata_hex: &str) -> Result<Vec<DecodedParam>> {
    let hex_str = calldata_hex.strip_prefix("0x").unwrap_or(calldata_hex);
    let data = hex::decode(hex_str).with_context(|| "Invalid hex for constructor data")?;

    let abi = load_abi(abi_path)?;
    let constructor = find_constructor(&abi)?;

    let inputs = constructor["inputs"]
        .as_array()
        .ok_or_else(|| anyhow!("Constructor missing inputs"))?;

    decode_params(inputs, &data)
}

fn decode_params(inputs: &[Value], data: &[u8]) -> Result<Vec<DecodedParam>> {
    let types: Vec<DynSolType> = inputs
        .iter()
        .map(abi_to_dyn_sol_type)
        .collect::<Result<_>>()?;

    let tuple_type = DynSolType::Tuple(types);
    let decoded = tuple_type
        .abi_decode_params(data)
        .map_err(|e| anyhow!("Failed to decode ABI params: {e}"))?;

    let values = match decoded {
        DynSolValue::Tuple(v) => v,
        _ => anyhow::bail!("Expected tuple from ABI decode"),
    };

    let mut results = Vec::new();
    for (i, (input, value)) in inputs.iter().zip(values.iter()).enumerate() {
        let name = input["name"]
            .as_str()
            .unwrap_or(&format!("param{i}"))
            .to_string();
        let sol_type = canonical_type(input)?;
        results.push(DecodedParam {
            name,
            sol_type,
            value: format_value(value),
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_abi_file(abi_json: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(abi_json.as_bytes()).unwrap();
        f
    }

    // -- selector & signature (our code, not alloy) --

    #[test]
    fn selector_transfer() {
        let sel = selector_from_signature("transfer(address,uint256)");
        assert_eq!(sel, [0xa9, 0x05, 0x9c, 0xbb]);
    }

    #[test]
    fn build_sig_from_abi_entry() {
        let entry: Value = serde_json::from_str(
            r#"{"name":"transfer","inputs":[{"type":"address"},{"type":"uint256"}]}"#,
        )
        .unwrap();
        assert_eq!(
            build_function_signature(&entry).unwrap(),
            "transfer(address,uint256)"
        );
    }

    #[test]
    fn build_sig_tuple() {
        let entry: Value = serde_json::from_str(
            r#"{"name":"processTuple","inputs":[{"type":"tuple","components":[{"type":"uint256"},{"type":"bool"}]}]}"#,
        )
        .unwrap();
        assert_eq!(
            build_function_signature(&entry).unwrap(),
            "processTuple((uint256,bool))"
        );
    }

    // -- error handling (our validation, not alloy) --

    const ERC20_ABI: &str = r#"[
        {
            "type": "function",
            "name": "transfer",
            "inputs": [
                {"name": "to", "type": "address"},
                {"name": "amount", "type": "uint256"}
            ],
            "outputs": [{"name": "", "type": "bool"}]
        },
        {
            "type": "constructor",
            "inputs": [
                {"name": "initialSupply", "type": "uint256"}
            ]
        }
    ]"#;

    #[test]
    fn encode_call_wrong_arg_count() {
        let f = write_abi_file(ERC20_ABI);
        let args = vec!["0x0000000000000000000000000000000000000001".to_string()];
        assert!(encode_call(f.path(), "transfer", &args).is_err());
    }

    #[test]
    fn encode_call_unknown_function() {
        let f = write_abi_file(ERC20_ABI);
        assert!(encode_call(f.path(), "nonexistent", &[]).is_err());
    }

    #[test]
    fn decode_call_too_short() {
        let f = write_abi_file(ERC20_ABI);
        assert!(decode_call(f.path(), "0xaa").is_err());
    }

    #[test]
    fn decode_call_unknown_selector() {
        let f = write_abi_file(ERC20_ABI);
        let data = format!("0x{}{}", "deadbeef", "00".repeat(32));
        assert!(decode_call(f.path(), &data).is_err());
    }

    // -- format_value roundtrip (tests our formatting, not alloy encoding) --

    #[test]
    fn roundtrip_mixed_types() {
        let abi = r#"[{
            "type": "function",
            "name": "doStuff",
            "inputs": [
                {"name": "flag", "type": "bool"},
                {"name": "addr", "type": "address"},
                {"name": "label", "type": "string"},
                {"name": "arr", "type": "uint256[]"},
                {"name": "data", "type": "tuple", "components": [
                    {"name": "x", "type": "uint256"},
                    {"name": "y", "type": "bool"}
                ]}
            ],
            "outputs": []
        }]"#;
        let f = write_abi_file(abi);
        let args = vec![
            "true".to_string(),
            "0x000000000000000000000000000000000000CAFE".to_string(),
            "hello".to_string(),
            "[1,2,3]".to_string(),
            "(42,true)".to_string(),
        ];
        let calldata = encode_call(f.path(), "doStuff", &args).unwrap();
        let hex_data = format!("0x{}", hex::encode(&calldata));

        assert_eq!(
            decode_call(f.path(), &hex_data).unwrap(),
            (
                "doStuff".to_string(),
                vec![
                    DecodedParam {
                        name: "flag".into(),
                        sol_type: "bool".into(),
                        value: "true".into()
                    },
                    DecodedParam {
                        name: "addr".into(),
                        sol_type: "address".into(),
                        value: "0x000000000000000000000000000000000000cafe".into()
                    },
                    DecodedParam {
                        name: "label".into(),
                        sol_type: "string".into(),
                        value: "hello".into()
                    },
                    DecodedParam {
                        name: "arr".into(),
                        sol_type: "uint256[]".into(),
                        value: "[1,2,3]".into()
                    },
                    DecodedParam {
                        name: "data".into(),
                        sol_type: "(uint256,bool)".into(),
                        value: "(42,true)".into()
                    },
                ]
            )
        );
    }
}

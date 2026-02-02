use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;
use tiny_keccak::{Hasher, Keccak};

#[derive(Debug, Clone)]
pub struct ContractInfo {
    pub sol_path: Option<String>,
    pub methods: Vec<MethodInfo>,
    pub has_constructor: bool,
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub name: String,
    pub rename: Option<String>,
    pub inputs: Vec<ParamInfo>,
    pub outputs: Vec<ParamInfo>,
}

#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub sol_type: String,
}

#[derive(Debug, Serialize)]
pub struct AbiJson(Vec<AbiItem>);

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AbiItem {
    Function {
        name: String,
        inputs: Vec<AbiParam>,
        outputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        state_mutability: String,
    },
    Constructor {
        inputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        state_mutability: String,
    },
}

#[derive(Debug, Serialize)]
pub struct AbiParam {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
}

pub fn generate_abi(manifest_dir: &Path) -> Result<Option<AbiJson>> {
    let src_dir = manifest_dir.join("src");
    if !src_dir.exists() {
        return Ok(None);
    }

    for entry in std::fs::read_dir(&src_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            if let Some(contract) = parse_contract_file(&path)? {
                if let Some(sol_path) = &contract.sol_path {
                    let sol_full_path = manifest_dir.join(sol_path);
                    return generate_abi_from_sol(&sol_full_path);
                } else {
                    return Ok(Some(generate_abi_from_methods(&contract)));
                }
            }
        }
    }

    Ok(None)
}

fn parse_contract_file(path: &Path) -> Result<Option<ContractInfo>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let file =
        syn::parse_file(&content).with_context(|| format!("Failed to parse {}", path.display()))?;

    for item in &file.items {
        if let syn::Item::Mod(item_mod) = item {
            if let Some(contract_info) = parse_contract_module(item_mod)? {
                return Ok(Some(contract_info));
            }
        }
    }

    Ok(None)
}

fn parse_contract_module(item_mod: &syn::ItemMod) -> Result<Option<ContractInfo>> {
    let sol_path = extract_contract_sol_path(&item_mod.attrs);

    let is_pvm_contract = item_mod.attrs.iter().any(|attr| {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        segments.len() == 2
            && (segments[0].ident == "pvm_contract" || segments[0].ident == "pvm")
            && segments[1].ident == "contract"
    });

    if !is_pvm_contract {
        return Ok(None);
    }

    let content = match &item_mod.content {
        Some((_, items)) => items,
        None => return Ok(None),
    };

    let mut methods = Vec::new();
    let mut has_constructor = false;

    for item in content {
        if let syn::Item::Fn(func) = item {
            if has_pvm_attr(&func.attrs, "constructor") {
                has_constructor = true;
            } else if has_pvm_attr(&func.attrs, "method") {
                let method = parse_method_fn(func)?;
                methods.push(method);
            }
        }
    }

    Ok(Some(ContractInfo {
        sol_path,
        methods,
        has_constructor,
    }))
}

fn extract_contract_sol_path(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        if segments.len() == 2
            && (segments[0].ident == "pvm_contract" || segments[0].ident == "pvm")
            && segments[1].ident == "contract"
        {
            if let syn::Meta::List(meta_list) = &attr.meta {
                let tokens_str = meta_list.tokens.to_string();
                if let Some(first_arg) = tokens_str.split(',').next() {
                    let trimmed = first_arg.trim().trim_matches('"');
                    if trimmed.ends_with(".sol") {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
    }
    None
}

fn has_pvm_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        segments.len() == 2
            && (segments[0].ident == "pvm_contract" || segments[0].ident == "pvm")
            && segments[1].ident == name
    })
}

fn extract_method_rename(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        if segments.len() == 2
            && (segments[0].ident == "pvm_contract" || segments[0].ident == "pvm")
            && segments[1].ident == "method"
        {
            if let syn::Meta::List(meta_list) = &attr.meta {
                let tokens_str = meta_list.tokens.to_string();
                if let Some(start) = tokens_str.find("rename") {
                    let after_rename = &tokens_str[start..];
                    if let Some(eq_pos) = after_rename.find('=') {
                        let after_eq = after_rename[eq_pos + 1..].trim();
                        let name = after_eq.trim_matches(|c| c == '"' || c == ' ');
                        if !name.is_empty() {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

fn parse_method_fn(func: &syn::ItemFn) -> Result<MethodInfo> {
    let name = func.sig.ident.to_string();
    let rename = extract_method_rename(&func.attrs);

    let inputs: Vec<ParamInfo> = func
        .sig
        .inputs
        .iter()
        .filter_map(|arg| {
            if let syn::FnArg::Typed(pat_type) = arg {
                let param_name = if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    pat_ident.ident.to_string()
                } else {
                    String::new()
                };
                let sol_type = rust_type_to_solidity(&pat_type.ty);
                Some(ParamInfo {
                    name: param_name,
                    sol_type,
                })
            } else {
                None
            }
        })
        .collect();

    let outputs = parse_return_type(&func.sig.output);

    Ok(MethodInfo {
        name,
        rename,
        inputs,
        outputs,
    })
}

fn rust_type_to_solidity(ty: &syn::Type) -> String {
    let type_str = quote::quote!(#ty).to_string().replace(' ', "");

    match type_str.as_str() {
        "Address" | "pvm_contract::Address" => "address".to_string(),
        "U256" | "pvm_contract::U256" => "uint256".to_string(),
        "u256" => "uint256".to_string(),
        "u128" => "uint128".to_string(),
        "u64" => "uint64".to_string(),
        "u32" => "uint32".to_string(),
        "u16" => "uint16".to_string(),
        "u8" => "uint8".to_string(),
        "i128" => "int128".to_string(),
        "i64" => "int64".to_string(),
        "i32" => "int32".to_string(),
        "i16" => "int16".to_string(),
        "i8" => "int8".to_string(),
        "bool" => "bool".to_string(),
        "[u8;32]" => "bytes32".to_string(),
        "[u8;20]" => "bytes20".to_string(),
        "String" | "alloc::string::String" => "string".to_string(),
        _ => "bytes".to_string(),
    }
}

fn parse_return_type(output: &syn::ReturnType) -> Vec<ParamInfo> {
    match output {
        syn::ReturnType::Default => vec![],
        syn::ReturnType::Type(_, ty) => {
            let type_str = quote::quote!(#ty).to_string().replace(' ', "");

            if type_str.starts_with("Result<") {
                if let Some(inner) = extract_result_ok_type(&type_str) {
                    if inner == "()" {
                        return vec![];
                    }
                    return vec![ParamInfo {
                        name: String::new(),
                        sol_type: rust_type_str_to_solidity(&inner),
                    }];
                }
            }

            if type_str == "()" {
                return vec![];
            }

            vec![ParamInfo {
                name: String::new(),
                sol_type: rust_type_to_solidity(ty),
            }]
        }
    }
}

fn extract_result_ok_type(type_str: &str) -> Option<String> {
    let inner = type_str.strip_prefix("Result<")?.strip_suffix('>')?;
    let comma_pos = inner.find(',')?;
    Some(inner[..comma_pos].trim().to_string())
}

fn rust_type_str_to_solidity(type_str: &str) -> String {
    match type_str {
        "Address" | "pvm_contract::Address" => "address".to_string(),
        "U256" | "pvm_contract::U256" => "uint256".to_string(),
        "u256" => "uint256".to_string(),
        "u128" => "uint128".to_string(),
        "u64" => "uint64".to_string(),
        "u32" => "uint32".to_string(),
        "u16" => "uint16".to_string(),
        "u8" => "uint8".to_string(),
        "bool" => "bool".to_string(),
        "()" => "".to_string(),
        "String" | "alloc::string::String" => "string".to_string(),
        _ => "bytes".to_string(),
    }
}

fn generate_abi_from_sol(sol_path: &Path) -> Result<Option<AbiJson>> {
    let content = std::fs::read_to_string(sol_path)
        .with_context(|| format!("Failed to read sol file: {}", sol_path.display()))?;

    let mut items = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("function ") {
            if let Some(func) = parse_sol_function_line(line) {
                items.push(func);
            }
        }
    }

    if items.is_empty() {
        return Ok(None);
    }

    Ok(Some(AbiJson(items)))
}

fn parse_sol_function_line(line: &str) -> Option<AbiItem> {
    let line = line.strip_prefix("function ")?.trim();

    let paren_start = line.find('(')?;
    let name = line[..paren_start].trim().to_string();

    let paren_end = line.find(')')?;
    let params_str = &line[paren_start + 1..paren_end];
    let inputs = parse_sol_params(params_str);

    let outputs = if let Some(returns_idx) = line.find("returns") {
        let after_returns = &line[returns_idx + 7..];
        if let Some(start) = after_returns.find('(') {
            if let Some(end) = after_returns.find(')') {
                parse_sol_params(&after_returns[start + 1..end])
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let state_mutability = if line.contains(" view ") || line.contains(" view)") {
        "view"
    } else if line.contains(" pure ") || line.contains(" pure)") {
        "pure"
    } else if line.contains(" payable ") || line.contains(" payable)") {
        "payable"
    } else {
        "nonpayable"
    }
    .to_string();

    Some(AbiItem::Function {
        name,
        inputs,
        outputs,
        state_mutability,
    })
}

fn parse_sol_params(params_str: &str) -> Vec<AbiParam> {
    if params_str.trim().is_empty() {
        return vec![];
    }

    params_str
        .split(',')
        .filter_map(|p| {
            let p = p.trim();
            let parts: Vec<&str> = p.split_whitespace().collect();
            if parts.is_empty() {
                return None;
            }
            let param_type = parts[0].to_string();
            let name = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
            Some(AbiParam { name, param_type })
        })
        .collect()
}

fn generate_abi_from_methods(contract: &ContractInfo) -> AbiJson {
    let mut items = Vec::new();

    if contract.has_constructor {
        items.push(AbiItem::Constructor {
            inputs: vec![],
            state_mutability: "nonpayable".to_string(),
        });
    }

    for method in &contract.methods {
        let fn_name = method
            .rename
            .clone()
            .unwrap_or_else(|| to_camel_case(&method.name));

        let inputs: Vec<AbiParam> = method
            .inputs
            .iter()
            .map(|p| AbiParam {
                name: p.name.clone(),
                param_type: p.sol_type.clone(),
            })
            .collect();

        let outputs: Vec<AbiParam> = method
            .outputs
            .iter()
            .filter(|p| !p.sol_type.is_empty())
            .map(|p| AbiParam {
                name: p.name.clone(),
                param_type: p.sol_type.clone(),
            })
            .collect();

        let state_mutability = if outputs.is_empty() {
            "nonpayable"
        } else {
            "view"
        }
        .to_string();

        items.push(AbiItem::Function {
            name: fn_name,
            inputs,
            outputs,
            state_mutability,
        });
    }

    AbiJson(items)
}

fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for (i, c) in s.chars().enumerate() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else if i == 0 {
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }

    result
}

#[allow(dead_code)]
pub fn compute_selector(canonical_signature: &str) -> [u8; 4] {
    let mut hasher = Keccak::v256();
    hasher.update(canonical_signature.as_bytes());
    let mut output = [0u8; 32];
    hasher.finalize(&mut output);
    [output[0], output[1], output[2], output[3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_camel_case() {
        assert_eq!(to_camel_case("total_supply"), "totalSupply");
        assert_eq!(to_camel_case("balance_of"), "balanceOf");
        assert_eq!(to_camel_case("transfer"), "transfer");
    }

    #[test]
    fn test_rust_type_to_solidity() {
        let ty: syn::Type = syn::parse_str("Address").unwrap();
        assert_eq!(rust_type_to_solidity(&ty), "address");

        let ty: syn::Type = syn::parse_str("U256").unwrap();
        assert_eq!(rust_type_to_solidity(&ty), "uint256");

        let ty: syn::Type = syn::parse_str("u64").unwrap();
        assert_eq!(rust_type_to_solidity(&ty), "uint64");
    }
}

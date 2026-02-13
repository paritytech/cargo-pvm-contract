use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use toml_edit::DocumentMut;

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
    pub rust_type: String,
    pub sol_type: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct AbiJson(Vec<AbiItem>);

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AbiItem {
    Function {
        name: String,
        inputs: Vec<AbiParam>,
        outputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        #[serde(skip_serializing_if = "Option::is_none")]
        state_mutability: Option<String>,
    },
    Constructor {
        inputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        #[serde(skip_serializing_if = "Option::is_none")]
        state_mutability: Option<String>,
    },
}

#[derive(Debug, Serialize, PartialEq)]
pub struct AbiParam {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
}

#[derive(Debug, Clone, PartialEq)]
enum SolType {
    Address,
    Bool,
    Uint(usize),
    Int(usize),
    Bytes(usize),
    String,
    Array(Box<SolType>),
    FixedArray(Box<SolType>, usize),
    Tuple(Vec<SolType>),
}

impl SolType {
    fn canonical_name(&self) -> String {
        match self {
            SolType::Address => "address".to_string(),
            SolType::Bool => "bool".to_string(),
            SolType::Uint(bits) => format!("uint{}", bits),
            SolType::Int(bits) => format!("int{}", bits),
            SolType::Bytes(size) => format!("bytes{}", size),
            SolType::String => "string".to_string(),
            SolType::Array(inner) => format!("{}[]", inner.canonical_name()),
            SolType::FixedArray(inner, size) => format!("{}[{}]", inner.canonical_name(), size),
            SolType::Tuple(types) => {
                let inner: Vec<_> = types.iter().map(|t| t.canonical_name()).collect();
                format!("({})", inner.join(","))
            }
        }
    }

    fn from_rust_type(ty: &syn::Type) -> Option<Self> {
        match ty {
            syn::Type::Path(type_path) => {
                let last_segment = type_path.path.segments.last()?;

                if last_segment.ident == "Vec"
                    && let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments
                    && let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first()
                {
                    return Self::from_rust_type(inner_ty)
                        .map(|inner| SolType::Array(Box::new(inner)));
                }

                if last_segment.ident == "Result"
                    && let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments
                    && let Some(syn::GenericArgument::Type(ok_ty)) = args.args.first()
                {
                    return Self::from_rust_type(ok_ty);
                }

                let type_str = quote::quote!(#ty).to_string().replace(' ', "");
                match type_str.as_str() {
                    "Address"
                    | "pvm_contract_types::Address"
                    | "::pvm_contract_types::Address"
                    | "pvm_contract::Address"
                    | "::pvm_contract::Address" => Some(SolType::Address),
                    "U256" | "pvm_contract::U256" | "u256" => Some(SolType::Uint(256)),
                    "u128" => Some(SolType::Uint(128)),
                    "u64" => Some(SolType::Uint(64)),
                    "u32" => Some(SolType::Uint(32)),
                    "u16" => Some(SolType::Uint(16)),
                    "u8" => Some(SolType::Uint(8)),
                    "i128" => Some(SolType::Int(128)),
                    "i64" => Some(SolType::Int(64)),
                    "i32" => Some(SolType::Int(32)),
                    "i16" => Some(SolType::Int(16)),
                    "i8" => Some(SolType::Int(8)),
                    "bool" => Some(SolType::Bool),
                    "[u8;32]" => Some(SolType::Bytes(32)),
                    "[u8;20]" => Some(SolType::Bytes(20)),
                    "String" | "alloc::string::String" => Some(SolType::String),
                    _ => None,
                }
            }
            syn::Type::Array(type_array) => {
                if let syn::Expr::Lit(expr_lit) = &type_array.len
                    && let syn::Lit::Int(len) = &expr_lit.lit
                    && let Ok(size) = len.base10_parse::<usize>()
                {
                    return Self::from_rust_type(&type_array.elem)
                        .map(|inner| SolType::FixedArray(Box::new(inner), size));
                }
                None
            }
            syn::Type::Tuple(type_tuple) => {
                let mut types = Vec::new();
                for elem in &type_tuple.elems {
                    types.push(Self::from_rust_type(elem)?);
                }
                Some(SolType::Tuple(types))
            }
            _ => None,
        }
    }
}

pub fn generate_abi(manifest_dir: &Path) -> Result<Option<AbiJson>> {
    let src_dir = manifest_dir.join("src");
    if !src_dir.exists() {
        return Ok(None);
    }

    for entry in std::fs::read_dir(&src_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "rs")
            && let Some(contract) = parse_contract_file(&path)?
        {
            if let Some(sol_path) = &contract.sol_path {
                let sol_full_path = manifest_dir.join(sol_path);
                return generate_abi_from_sol(&sol_full_path);
            } else {
                let probe_type_map = probe_type_map_or_fallback(manifest_dir, &contract);
                return Ok(Some(generate_abi_from_methods(&contract, &probe_type_map)));
            }
        }
    }

    Ok(None)
}

pub fn generate_abi_for_bin(manifest_dir: &Path, bin_name: &str) -> Result<Option<AbiJson>> {
    let contract_path = resolve_bin_source_path(manifest_dir, bin_name)?;
    if !contract_path.exists() {
        return Ok(None);
    }

    if let Some(contract) = parse_contract_file(&contract_path)? {
        if let Some(sol_path) = &contract.sol_path {
            let sol_full_path = manifest_dir.join(sol_path);
            generate_abi_from_sol(&sol_full_path)
        } else {
            let probe_type_map = probe_type_map_or_fallback(manifest_dir, &contract);
            Ok(Some(generate_abi_from_methods(&contract, &probe_type_map)))
        }
    } else {
        Ok(None)
    }
}

fn resolve_bin_source_path(manifest_dir: &Path, bin_name: &str) -> Result<std::path::PathBuf> {
    let cargo_toml_path = manifest_dir.join("Cargo.toml");
    let cargo_toml = std::fs::read_to_string(&cargo_toml_path)
        .with_context(|| format!("Failed to read {}", cargo_toml_path.display()))?;
    let doc = cargo_toml
        .parse::<DocumentMut>()
        .context("Failed to parse Cargo.toml")?;

    if let Some(bin_array) = doc.get("bin").and_then(|b| b.as_array_of_tables()) {
        for bin in bin_array {
            if bin.get("name").and_then(|n| n.as_str()) == Some(bin_name) {
                if let Some(path) = bin.get("path").and_then(|p| p.as_str()) {
                    return Ok(manifest_dir.join(path));
                }
                return Ok(manifest_dir.join("src/bin").join(format!("{bin_name}.rs")));
            }
        }
    }

    if doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        == Some(bin_name)
    {
        return Ok(manifest_dir.join("src/main.rs"));
    }

    Ok(manifest_dir.join("src/bin").join(format!("{bin_name}.rs")))
}

fn parse_contract_file(path: &Path) -> Result<Option<ContractInfo>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let file =
        syn::parse_file(&content).with_context(|| format!("Failed to parse {}", path.display()))?;

    for item in &file.items {
        if let syn::Item::Mod(item_mod) = item
            && let Some(contract_info) = parse_contract_module(item_mod)?
        {
            return Ok(Some(contract_info));
        }
    }

    Ok(None)
}

const VALID_PREFIXES: &[&str] = &["pvm", "pvm_contract", "pvm_contract_macros"];

fn is_pvm_path(segments: &[&syn::PathSegment], attr_name: &str) -> bool {
    segments.len() == 2
        && VALID_PREFIXES.contains(&segments[0].ident.to_string().as_str())
        && segments[1].ident == attr_name
}

fn parse_contract_module(item_mod: &syn::ItemMod) -> Result<Option<ContractInfo>> {
    let sol_path = extract_contract_sol_path(&item_mod.attrs);

    let is_pvm_contract = item_mod.attrs.iter().any(|attr| {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        is_pvm_path(&segments, "contract")
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
        if is_pvm_path(&segments, "contract")
            && let syn::Meta::List(meta_list) = &attr.meta
        {
            let tokens_str = meta_list.tokens.to_string();
            if let Some(first_arg) = tokens_str.split(',').next() {
                let trimmed = first_arg.trim().trim_matches('"');
                if trimmed.ends_with(".sol") {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

fn has_pvm_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        is_pvm_path(&segments, name)
    })
}

fn extract_method_rename(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        if is_pvm_path(&segments, "method")
            && let syn::Meta::List(meta_list) = &attr.meta
        {
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
                let rust_type = normalize_rust_type(&pat_type.ty);
                let sol_type = rust_type_to_solidity(&pat_type.ty);
                Some(ParamInfo {
                    name: param_name,
                    rust_type,
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
    SolType::from_rust_type(ty)
        .map(|sol_type| sol_type.canonical_name())
        .unwrap_or_else(|| "bytes".to_string())
}

fn parse_return_type(output: &syn::ReturnType) -> Vec<ParamInfo> {
    match output {
        syn::ReturnType::Default => vec![],
        syn::ReturnType::Type(_, ty) => {
            let return_ty = unwrap_result_ok_type(ty);
            if is_unit_type(return_ty) {
                return vec![];
            }

            vec![ParamInfo {
                name: String::new(),
                rust_type: normalize_rust_type(return_ty),
                sol_type: rust_type_to_solidity(return_ty),
            }]
        }
    }
}

fn normalize_rust_type(ty: &syn::Type) -> String {
    quote::quote!(#ty).to_string().replace(' ', "")
}

fn unwrap_result_ok_type(ty: &syn::Type) -> &syn::Type {
    if let syn::Type::Path(type_path) = ty
        && let Some(last_segment) = type_path.path.segments.last()
        && last_segment.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments
        && let Some(syn::GenericArgument::Type(ok_ty)) = args.args.first()
    {
        return ok_ty;
    }
    ty
}

fn is_unit_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Tuple(type_tuple) => type_tuple.elems.is_empty(),
        _ => {
            let type_str = quote::quote!(#ty).to_string().replace(' ', "");
            type_str == "()"
        }
    }
}

fn probe_type_map_or_fallback(
    manifest_dir: &Path,
    contract: &ContractInfo,
) -> HashMap<String, String> {
    match run_sol_name_probe(manifest_dir, contract) {
        Ok(type_map) => type_map,
        Err(err) => {
            eprintln!("Warning: probe failed, using fallback: {err}");
            HashMap::new()
        }
    }
}

fn run_sol_name_probe(
    manifest_dir: &Path,
    contract: &ContractInfo,
) -> Result<HashMap<String, String>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").context("OUT_DIR is set")?);
    let probe_target_dir = super::get_target_root().join("abi-probe-target");

    run_sol_name_probe_with_paths(manifest_dir, contract, &out_dir, &probe_target_dir)
}

fn run_sol_name_probe_with_paths(
    manifest_dir: &Path,
    contract: &ContractInfo,
    out_dir: &Path,
    probe_target_dir: &Path,
) -> Result<HashMap<String, String>> {
    let type_paths = collect_probe_type_paths(contract);
    if type_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let probe_dir = out_dir.join("abi-probe");
    let probe_src_dir = probe_dir.join("src");

    fs::create_dir_all(&probe_src_dir).with_context(|| {
        format!(
            "Failed to create probe src dir: {}",
            probe_src_dir.display()
        )
    })?;

    let pvm_contract_types_path = resolve_pvm_contract_types_path()?;
    let manifest_path = probe_dir.join("Cargo.toml");
    let main_path = probe_src_dir.join("main.rs");

    fs::write(
        &manifest_path,
        render_probe_manifest(&pvm_contract_types_path),
    )
    .with_context(|| {
        format!(
            "Failed to write probe manifest: {}",
            manifest_path.display()
        )
    })?;

    fs::write(&main_path, render_probe_main(&type_paths))
        .with_context(|| format!("Failed to write probe source: {}", main_path.display()))?;

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    let output = Command::new(&cargo)
        .current_dir(manifest_dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env("CARGO_TARGET_DIR", probe_target_dir)
        .arg("run")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .output()
        .context("Failed to execute ABI probe")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ABI probe cargo run failed:\n{stderr}");
    }

    parse_probe_stdout(&output.stdout)
}

fn collect_probe_type_paths(contract: &ContractInfo) -> Vec<String> {
    let mut unique = BTreeSet::new();

    for method in &contract.methods {
        for param in method.inputs.iter().chain(method.outputs.iter()) {
            if !param.rust_type.is_empty() {
                unique.insert(param.rust_type.clone());
            }
        }
    }

    unique.into_iter().collect()
}

fn resolve_pvm_contract_types_path() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../pvm-contract-types")
        .canonicalize()
        .context("Failed to resolve pvm-contract-types path for ABI probe")
}

fn render_probe_manifest(pvm_contract_types_path: &Path) -> String {
    let escaped_path = pvm_contract_types_path
        .display()
        .to_string()
        .replace('\\', "\\\\");

    format!(
        "[package]\nname = \"abi-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\npvm-contract-types = {{ path = \"{escaped_path}\", features = [\"abi-reflection\"] }}\nruint = {{ version = \"1.12\", default-features = false }}\n"
    )
}

fn render_probe_main(type_paths: &[String]) -> String {
    use std::fmt::Write;

    let mut source = String::from(
        "use pvm_contract_types::{Address, SolTypeName};\nuse ruint::aliases::U256;\n\nfn main() {\n",
    );

    for type_path in type_paths {
        let escaped = type_path.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(
            source,
            "    println!(\"{}={{}}\", <{} as SolTypeName>::sol_name());",
            escaped, type_path
        )
        .expect("writing probe source to String should never fail");
    }

    source.push_str("}\n");
    source
}

fn parse_probe_stdout(stdout: &[u8]) -> Result<HashMap<String, String>> {
    let stdout_str =
        String::from_utf8(stdout.to_vec()).context("Probe output is not valid UTF-8")?;
    let mut map = HashMap::new();

    for line in stdout_str.lines() {
        if let Some((type_path, sol_name)) = line.split_once('=') {
            map.insert(type_path.to_string(), sol_name.to_string());
        }
    }

    Ok(map)
}

fn generate_abi_from_sol(sol_path: &Path) -> Result<Option<AbiJson>> {
    let content = std::fs::read_to_string(sol_path)
        .with_context(|| format!("Failed to read sol file: {}", sol_path.display()))?;

    let mut items = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("function ")
            && let Some(func) = parse_sol_function_line(line)
        {
            items.push(func);
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
        state_mutability: Some(state_mutability),
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

fn generate_abi_from_methods(
    contract: &ContractInfo,
    probe_type_map: &HashMap<String, String>,
) -> AbiJson {
    let mut items = Vec::new();

    if contract.has_constructor {
        items.push(AbiItem::Constructor {
            inputs: vec![],
            state_mutability: None,
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
                param_type: probe_type_map
                    .get(&p.rust_type)
                    .cloned()
                    .unwrap_or_else(|| p.sol_type.clone()),
            })
            .collect();

        let outputs: Vec<AbiParam> = method
            .outputs
            .iter()
            .filter(|p| !p.sol_type.is_empty())
            .map(|p| AbiParam {
                name: p.name.clone(),
                param_type: probe_type_map
                    .get(&p.rust_type)
                    .cloned()
                    .unwrap_or_else(|| p.sol_type.clone()),
            })
            .collect();

        items.push(AbiItem::Function {
            name: fn_name,
            inputs,
            outputs,
            state_mutability: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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

        let ty: syn::Type = syn::parse_str("pvm_contract_types::Address").unwrap();
        assert_eq!(rust_type_to_solidity(&ty), "address");

        let ty: syn::Type = syn::parse_str("U256").unwrap();
        assert_eq!(rust_type_to_solidity(&ty), "uint256");

        let ty: syn::Type = syn::parse_str("u64").unwrap();
        assert_eq!(rust_type_to_solidity(&ty), "uint64");

        let ty: syn::Type = syn::parse_str("Vec<Address>").unwrap();
        assert_eq!(rust_type_to_solidity(&ty), "address[]");

        let ty: syn::Type = syn::parse_str("Vec<u64>").unwrap();
        assert_eq!(rust_type_to_solidity(&ty), "uint64[]");
    }

    #[test]
    fn test_collect_probe_type_paths() {
        let contract = ContractInfo {
            sol_path: None,
            methods: vec![MethodInfo {
                name: "foo".to_string(),
                rename: None,
                inputs: vec![
                    ParamInfo {
                        name: "a".to_string(),
                        rust_type: "Address".to_string(),
                        sol_type: "address".to_string(),
                    },
                    ParamInfo {
                        name: "b".to_string(),
                        rust_type: "Vec<Address>".to_string(),
                        sol_type: "address[]".to_string(),
                    },
                ],
                outputs: vec![ParamInfo {
                    name: String::new(),
                    rust_type: "Address".to_string(),
                    sol_type: "address".to_string(),
                }],
            }],
            has_constructor: false,
        };

        assert_eq!(
            collect_probe_type_paths(&contract),
            vec!["Address".to_string(), "Vec<Address>".to_string()]
        );
    }

    #[test]
    fn test_generate_abi_from_methods_prefers_probe_types() {
        let contract = ContractInfo {
            sol_path: None,
            methods: vec![MethodInfo {
                name: "balance_of".to_string(),
                rename: None,
                inputs: vec![ParamInfo {
                    name: "account".to_string(),
                    rust_type: "MyAddress".to_string(),
                    sol_type: "bytes".to_string(),
                }],
                outputs: vec![ParamInfo {
                    name: String::new(),
                    rust_type: "Vec<MyAddress>".to_string(),
                    sol_type: "bytes".to_string(),
                }],
            }],
            has_constructor: false,
        };

        let probe_map = HashMap::from([
            ("MyAddress".to_string(), "address".to_string()),
            ("Vec<MyAddress>".to_string(), "address[]".to_string()),
        ]);

        assert_eq!(
            generate_abi_from_methods(&contract, &probe_map),
            AbiJson(vec![AbiItem::Function {
                name: "balanceOf".to_string(),
                inputs: vec![AbiParam {
                    name: "account".to_string(),
                    param_type: "address".to_string(),
                }],
                outputs: vec![AbiParam {
                    name: String::new(),
                    param_type: "address[]".to_string(),
                }],
                state_mutability: None,
            }])
        );
    }

    #[test]
    fn test_parse_probe_stdout() {
        let output = b"Address=address\nVec<Address>=address[]\n";

        assert_eq!(
            parse_probe_stdout(output).unwrap(),
            HashMap::from([
                ("Address".to_string(), "address".to_string()),
                ("Vec<Address>".to_string(), "address[]".to_string()),
            ])
        );
    }

    #[test]
    fn test_run_sol_name_probe_with_paths_success() {
        let out_dir = unique_temp_dir("probe-success-out");
        let target_dir = unique_temp_dir("probe-success-target");

        let contract = ContractInfo {
            sol_path: None,
            methods: vec![MethodInfo {
                name: "foo".to_string(),
                rename: None,
                inputs: vec![ParamInfo {
                    name: "a".to_string(),
                    rust_type: "Address".to_string(),
                    sol_type: "bytes".to_string(),
                }],
                outputs: vec![],
            }],
            has_constructor: false,
        };

        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let probe_map =
            run_sol_name_probe_with_paths(manifest_dir, &contract, &out_dir, &target_dir).unwrap();

        assert_eq!(
            probe_map,
            HashMap::from([("Address".to_string(), "address".to_string())])
        );

        std::fs::remove_dir_all(out_dir).ok();
        std::fs::remove_dir_all(target_dir).ok();
    }

    #[test]
    fn test_run_sol_name_probe_with_paths_failure() {
        let out_dir = unique_temp_dir("probe-failure-out");
        let target_dir = unique_temp_dir("probe-failure-target");

        let contract = ContractInfo {
            sol_path: None,
            methods: vec![MethodInfo {
                name: "foo".to_string(),
                rename: None,
                inputs: vec![ParamInfo {
                    name: "a".to_string(),
                    rust_type: "UnknownType".to_string(),
                    sol_type: "bytes".to_string(),
                }],
                outputs: vec![],
            }],
            has_constructor: false,
        };

        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let probe_error =
            run_sol_name_probe_with_paths(manifest_dir, &contract, &out_dir, &target_dir)
                .expect_err("unknown probe type should fail");

        let error_str = probe_error.to_string();
        assert!(error_str.contains("ABI probe cargo run failed"));

        std::fs::remove_dir_all(out_dir).ok();
        std::fs::remove_dir_all(target_dir).ok();
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time should be after unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "cargo-pvm-contract-builder-{label}-{}-{nanos}",
            std::process::id()
        ));

        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|err| panic!("failed to create temp dir {}: {err}", dir.display()));
        dir
    }
}

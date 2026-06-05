use anyhow::{Context, Result};
use std::{env, fs, path::Path, process::Command};
use toml_edit::DocumentMut;

// Re-export ABI types from the canonical definitions in pvm-contract-types.
pub use pvm_contract_types::{AbiEventParam, AbiItem, AbiJson, AbiParam, parse_type_str};

pub fn generate_abi_for_bin(
    manifest_dir: &Path,
    bin_name: &str,
    target_root: Option<&Path>,
    features: Option<&str>,
) -> Result<Option<AbiJson>> {
    generate_abi_via_feature(manifest_dir, bin_name, target_root, features)
}

/// Generate the storage layout JSON for a binary, if the contract declares
/// `#[slot(N)]` fields. Returns the raw `serde_json::Value` of the
/// `storageLayout` object, or `None` when no storage is declared.
///
/// Detection is a simple source-level check for `#[slot(`. When slot fields
/// are present, the `#[contract]` macro generates an abi-gen `main()` that
/// outputs the layout. If the abi-gen binary fails with "main function not
/// found", we treat it as "no storage layout" (the slot attr may have been
/// outside a `#[contract]` module). All other failures propagate normally.
pub fn generate_storage_layout_for_bin(
    manifest_dir: &Path,
    bin_name: &str,
    target_root: Option<&Path>,
    features: Option<&str>,
) -> Result<Option<serde_json::Value>> {
    let source_path = resolve_bin_source_path(manifest_dir, bin_name)?;
    if !source_path.exists()
        || !has_slot_fields(
            &fs::read_to_string(&source_path)
                .with_context(|| format!("Failed to read {}", source_path.display()))?,
        )
    {
        return Ok(None);
    }

    let stdout = match run_abi_gen_binary(manifest_dir, bin_name, target_root, features) {
        Ok(Some(s)) => s,
        Ok(None) => return Ok(None),
        Err(e) => {
            let msg = format!("{e:?}");
            if msg.contains("main function not found")
                || msg.contains("main` function not found")
                || msg.contains("does not contain this feature")
            {
                return Ok(None);
            }
            return Err(e).context("Failed to generate storage layout via abi-gen");
        }
    };
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Ok(None);
    }

    let value: serde_json::Value =
        serde_json::from_str(stdout).context("Failed to parse storage layout JSON from abi-gen")?;

    // The output is either just the storage layout (sol path) or a combined
    // object with "storageLayout" field (non-sol path).
    Ok(value
        .get("storageLayout")
        .cloned()
        .or_else(|| value.get("storage").map(|_| value.clone())))
}

fn get_host_triple() -> Result<String> {
    if let Ok(host) = env::var("HOST") {
        return Ok(host);
    }
    // Fallback: parse `rustc -vV` output
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("Failed to execute rustc -vV")?;
    if !output.status.success() {
        anyhow::bail!("rustc -vV failed");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(triple) = line.strip_prefix("host: ") {
            return Ok(triple.trim().to_string());
        }
    }
    anyhow::bail!("Could not determine host triple from rustc -vV")
}

fn generate_abi_via_feature(
    manifest_dir: &Path,
    bin_name: &str,
    target_root: Option<&Path>,
    features: Option<&str>,
) -> Result<Option<AbiJson>> {
    let source_path = resolve_bin_source_path(manifest_dir, bin_name)?;
    if !source_path.exists() {
        return Ok(None);
    }

    let source_content = fs::read_to_string(&source_path)
        .with_context(|| format!("Failed to read {}", source_path.display()))?;

    if let Some(sol_path) = extract_sol_path_from_source(&source_content) {
        let sol_full_path = manifest_dir.join(sol_path);
        return generate_abi_from_sol(&sol_full_path);
    }

    // Non-.sol path: ABI generation requires the `#[contract]` macro.
    if !has_contract_macro(&source_content) {
        return Ok(None);
    }

    // The abi-gen binary outputs either a bare ABI array (no storage) or
    // {"abi": [...], "storageLayout": {...}} (with storage). Extract
    // just the ABI items either way.
    let stdout = run_abi_gen_binary(manifest_dir, bin_name, target_root, features)?
        .context("abi-gen binary produced no output")?;

    let value: serde_json::Value =
        serde_json::from_str(&stdout).context("Failed to parse abi-gen output as JSON")?;

    let abi_value = match value.get("abi") {
        Some(v) => v.clone(),
        None => value, // bare array
    };

    let abi: AbiJson =
        serde_json::from_value(abi_value).context("Failed to parse ABI from abi-gen output")?;

    Ok(Some(abi))
}

/// Run the abi-gen binary and return its raw stdout.
fn run_abi_gen_binary(
    manifest_dir: &Path,
    bin_name: &str,
    target_root: Option<&Path>,
    features: Option<&str>,
) -> Result<Option<String>> {
    let target_dir = match target_root {
        Some(root) => root.join("abi-gen-target"),
        None => super::get_target_root().join("abi-gen-target"),
    };
    let manifest_path = manifest_dir.join("Cargo.toml");
    let host = get_host_triple()?;

    let has_toolchain_file = manifest_dir.join("rust-toolchain.toml").exists()
        || manifest_dir.join("rust-toolchain").exists();

    let mut cmd = Command::new("cargo");
    cmd.current_dir(manifest_dir)
        .env_remove("CARGO")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env("CARGO_TARGET_DIR", &target_dir);

    if has_toolchain_file {
        cmd.env_remove("RUSTUP_TOOLCHAIN");
    }

    // Combine `abi-gen` with any user-supplied features into a single
    // `--features` argument (cargo accepts comma-separated lists).
    let combined_features = match features {
        Some(list) if !list.trim().is_empty() => format!("abi-gen,{list}"),
        _ => "abi-gen".to_string(),
    };

    let output = cmd
        .env(super::INTERNAL_BUILD_ENV, "1")
        .arg("run")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--target")
        .arg(&host)
        .arg("--config")
        .arg(r#"unstable.build-std=["std","core","alloc"]"#)
        .arg("--features")
        .arg(&combined_features)
        .arg("--bin")
        .arg(bin_name)
        .output()
        .context("Failed to execute abi-gen compile and run")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ABI generation via abi-gen feature failed:\n{stderr}");
    }

    let stdout_str =
        String::from_utf8(output.stdout).context("ABI generation output is not valid UTF-8")?;
    let trimmed = stdout_str.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

/// Locate the `#[contract]` attribute in the source, if present, and return
/// a reference for further inspection. Used to gate ABI generation, which
/// runs only when the macro is actually present (DSL contracts skip it).
/// Matches every form the user might write: `#[contract]`, `#[contract(...)]`,
/// `#[pvm_contract_sdk::contract]`, `#[pvm_contract_sdk::contract(...)]`,
/// `#[pvm_contract_macros::contract]`, `#[pvm_contract_macros::contract(...)]`.
fn find_contract_attr(items: &[syn::Item]) -> Option<&syn::Attribute> {
    for item in items {
        for attr in item_attrs(item) {
            if attr
                .path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "contract")
            {
                return Some(attr);
            }
        }
        if let syn::Item::Mod(m) = item
            && let Some((_, nested)) = &m.content
            && let Some(found) = find_contract_attr(nested)
        {
            return Some(found);
        }
    }
    None
}

/// Return the attributes attached to any top-level Rust item (`struct`, `fn`,
/// `mod`, `impl`, etc.). The `_ => &[]` arm handles `syn::Item`'s
/// `#[non_exhaustive]` declaration so future variants don't break the build.
fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Enum(i) => &i.attrs,
        syn::Item::ExternCrate(i) => &i.attrs,
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::ForeignMod(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Macro(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        syn::Item::Struct(i) => &i.attrs,
        syn::Item::Trait(i) => &i.attrs,
        syn::Item::TraitAlias(i) => &i.attrs,
        syn::Item::Type(i) => &i.attrs,
        syn::Item::Union(i) => &i.attrs,
        syn::Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

/// Detect whether the source uses the `#[contract]` attribute macro. Used to
/// skip ABI generation for DSL-based contracts that don't use the macro.
///
/// Parses the source via `syn` so that `#[contract]`-shaped text in comments
/// or string literals doesn't trip detection.
pub(crate) fn has_contract_macro(source: &str) -> bool {
    let Ok(file) = syn::parse_file(source) else {
        return false;
    };
    find_contract_attr(&file.items).is_some()
}

/// Walk for any struct field carrying a `#[slot(...)]` attribute, recursing
/// into `mod` contents.
fn any_struct_field_has_slot_attr(items: &[syn::Item]) -> bool {
    for item in items {
        match item {
            syn::Item::Struct(s) => {
                if let syn::Fields::Named(named) = &s.fields
                    && named
                        .named
                        .iter()
                        .any(|f| f.attrs.iter().any(|a| a.path().is_ident("slot")))
                {
                    return true;
                }
            }
            syn::Item::Mod(m) => {
                if let Some((_, nested)) = &m.content
                    && any_struct_field_has_slot_attr(nested)
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Detect whether the source contains a `#[slot(...)]` attribute on a struct
/// field, which indicates storage fields on the contract struct.
///
/// Parses the source via `syn` so that `#[slot(...)]`-shaped text in comments
/// or string literals doesn't trip detection.
fn has_slot_fields(source: &str) -> bool {
    let Ok(file) = syn::parse_file(source) else {
        return false;
    };
    any_struct_field_has_slot_attr(&file.items)
}

pub(crate) fn extract_sol_path_from_source(source: &str) -> Option<String> {
    let file = syn::parse_file(source).ok()?;
    let attr = find_contract_attr(&file.items)?;

    // `#[contract]` with no args is `Meta::Path` (no Sol arg).
    // `#[contract(...)]` is `Meta::List`; parse the comma-separated args
    // and return the first literal string ending in `.sol`.
    let args = attr
        .parse_args_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated)
        .ok()?;

    for expr in &args {
        if let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) = expr
        {
            let path = s.value();
            if path.ends_with(".sol") {
                return Some(path);
            }
        }
    }
    None
}

pub(crate) fn resolve_bin_source_path(
    manifest_dir: &Path,
    bin_name: &str,
) -> Result<std::path::PathBuf> {
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

pub(crate) fn generate_abi_from_sol(sol_path: &Path) -> Result<Option<AbiJson>> {
    let content = std::fs::read_to_string(sol_path)
        .with_context(|| format!("Failed to read sol file: {}", sol_path.display()))?;

    let file: syn_solidity::File = match syn::parse_str(&content) {
        Ok(file) => file,
        // syn-solidity rejects input with no top-level items (e.g. a file that
        // is only comments/whitespace) as a parse error. Distinguish that from
        // a genuinely malformed file by relexing: a comments-only file produces
        // an empty token stream, which we treat as "no ABI" rather than a hard
        // failure. Every other parse error propagates.
        Err(e) => {
            return match content.parse::<proc_macro2::TokenStream>() {
                Ok(tokens) if tokens.is_empty() => Ok(None),
                _ => Err(anyhow::anyhow!(
                    "Failed to parse Solidity file {}: {e}",
                    sol_path.display()
                )),
            };
        }
    };

    // Flatten items, descending into contract/interface/library bodies.
    let mut flat: Vec<&syn_solidity::Item> = Vec::new();
    collect_items(&file.items, &mut flat);

    // First pass: build a registry of struct definitions for named-struct
    // resolution, keyed by the struct's last path segment.
    let mut structs: StructMap = std::collections::HashMap::new();
    for item in &flat {
        if let syn_solidity::Item::Struct(s) = item {
            structs.insert(s.name.to_string(), s);
        }
    }

    // Second pass: map declarations to ABI items.
    let mut items: Vec<AbiItem> = Vec::new();
    for item in &flat {
        match item {
            syn_solidity::Item::Function(func) => {
                if let Some(abi) = function_to_abi(func, &structs) {
                    items.push(abi);
                }
            }
            syn_solidity::Item::Error(err) => items.push(error_to_abi(err, &structs)),
            syn_solidity::Item::Event(evt) => items.push(event_to_abi(evt, &structs)),
            _ => {}
        }
    }

    if items.is_empty() {
        return Ok(None);
    }

    // Append framework-level parameterless custom errors, unless the .sol
    // interface already defines an error with the same name.
    for name in pvm_contract_types::framework_errors::NAMES {
        let already_defined = items
            .iter()
            .any(|item| matches!(item, AbiItem::Error { name: n, .. } if n == name));
        if !already_defined {
            items.push(AbiItem::Error {
                name: name.to_string(),
                inputs: vec![],
            });
        }
    }

    Ok(Some(AbiJson(items)))
}

type StructMap<'a> = std::collections::HashMap<String, &'a syn_solidity::ItemStruct>;

/// Recursively collect items, descending into contract/interface/library
/// bodies so nested structs, functions, errors, and events are all visited.
fn collect_items<'a>(items: &'a [syn_solidity::Item], out: &mut Vec<&'a syn_solidity::Item>) {
    for item in items {
        out.push(item);
        if let syn_solidity::Item::Contract(c) = item {
            collect_items(&c.body, out);
        }
    }
}

/// Render a `syn_solidity::Type` to its canonical Solidity type string,
/// expanding named structs inline into tuple strings (e.g. `(uint256,uint256)`)
/// so `parse_type_str` produces proper ABI `tuple` components.
fn type_to_sol_string(ty: &syn_solidity::Type, structs: &StructMap) -> String {
    use syn_solidity::Type;
    match ty {
        Type::Address(_, _) => "address".to_string(),
        Type::Bool(_) => "bool".to_string(),
        Type::String(_) => "string".to_string(),
        Type::Bytes(_) => "bytes".to_string(),
        Type::FixedBytes(_, size) => format!("bytes{}", size.get()),
        Type::Int(_, size) => format!("int{}", size.map(|s| s.get()).unwrap_or(256)),
        Type::Uint(_, size) => format!("uint{}", size.map(|s| s.get()).unwrap_or(256)),
        Type::Array(arr) => {
            let inner = type_to_sol_string(&arr.ty, structs);
            match arr.size() {
                Some(n) => format!("{inner}[{n}]"),
                None => format!("{inner}[]"),
            }
        }
        Type::Tuple(tuple) => {
            let parts: Vec<String> = tuple
                .types
                .iter()
                .map(|t| type_to_sol_string(t, structs))
                .collect();
            format!("({})", parts.join(","))
        }
        Type::Custom(path) => {
            let name = path.last().to_string();
            if let Some(def) = structs.get(&name) {
                let parts: Vec<String> = def
                    .fields
                    .iter()
                    .map(|field| type_to_sol_string(&field.ty, structs))
                    .collect();
                format!("({})", parts.join(","))
            } else {
                // Unknown custom type (enum, UDT, or undefined): fall back to
                // its bare name, matching pre-syn behavior.
                name
            }
        }
        // Mappings and function types are not valid ABI parameter types.
        Type::Mapping(_) | Type::Function(_) => String::new(),
    }
}

fn param_to_abi(decl: &syn_solidity::VariableDeclaration, structs: &StructMap) -> AbiParam {
    let name = decl
        .name
        .as_ref()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let rendered = type_to_sol_string(&decl.ty, structs);
    parse_type_str(&name, &rendered)
}

fn function_to_abi(func: &syn_solidity::ItemFunction, structs: &StructMap) -> Option<AbiItem> {
    use syn_solidity::{FunctionKind, Mutability};

    match func.kind {
        FunctionKind::Function(_) => {
            let inputs = func
                .parameters
                .iter()
                .map(|p| param_to_abi(p, structs))
                .collect();
            let outputs = func
                .returns
                .as_ref()
                .map(|r| r.returns.iter().map(|p| param_to_abi(p, structs)).collect())
                .unwrap_or_default();
            let state_mutability = match func.attributes.mutability() {
                Some(Mutability::Pure(_)) => "pure",
                Some(Mutability::View(_)) => "view",
                Some(Mutability::Payable(_)) => "payable",
                _ => "nonpayable",
            }
            .to_string();
            Some(AbiItem::Function {
                name: func.name().to_string(),
                inputs,
                outputs,
                state_mutability: Some(state_mutability),
            })
        }
        FunctionKind::Constructor(_) => {
            let inputs = func
                .parameters
                .iter()
                .map(|p| param_to_abi(p, structs))
                .collect();
            let state_mutability = match func.attributes.mutability() {
                Some(Mutability::Payable(_)) => "payable",
                _ => "nonpayable",
            }
            .to_string();
            Some(AbiItem::Constructor {
                inputs,
                state_mutability: Some(state_mutability),
            })
        }
        FunctionKind::Receive(_) => Some(AbiItem::Receive {
            state_mutability: Some("payable".to_string()),
        }),
        // Fallback and modifier definitions are not emitted in the ABI
        // (preserving the prior parser, which never handled them).
        FunctionKind::Fallback(_) | FunctionKind::Modifier(_) => None,
    }
}

fn error_to_abi(err: &syn_solidity::ItemError, structs: &StructMap) -> AbiItem {
    AbiItem::Error {
        name: err.name.to_string(),
        inputs: err
            .parameters
            .iter()
            .map(|p| param_to_abi(p, structs))
            .collect(),
    }
}

fn event_to_abi(evt: &syn_solidity::ItemEvent, structs: &StructMap) -> AbiItem {
    let inputs = evt
        .parameters
        .iter()
        .map(|p| {
            let name = p.name.as_ref().map(|n| n.to_string()).unwrap_or_default();
            let rendered = type_to_sol_string(&p.ty, structs);
            let expanded = parse_type_str(&name, &rendered);
            AbiEventParam {
                name: expanded.name,
                param_type: expanded.param_type,
                components: expanded.components,
                indexed: p.indexed.is_some(),
            }
        })
        .collect();
    AbiItem::Event {
        name: evt.name.to_string(),
        inputs,
        anonymous: evt.is_anonymous(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_sol(name: &str, body: &str) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{body}").unwrap();
        (dir, path)
    }

    #[test]
    fn generate_abi_from_sol_handles_block_and_inline_comments() {
        let (_d, path) = write_sol(
            "Commented.sol",
            r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Commented {
    /* a block comment
       function notAFunction(uint256 ignored) external; */
    function transfer(
        address to, // inline comment mid-signature
        uint256 amount
    ) external returns (bool);
}
"#,
        );

        let abi = generate_abi_from_sol(&path).unwrap().unwrap();
        let funcs: Vec<_> = abi
            .0
            .iter()
            .filter(|i| matches!(i, AbiItem::Function { .. }))
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(
            funcs[0],
            &AbiItem::Function {
                name: "transfer".to_string(),
                inputs: vec![
                    AbiParam {
                        name: "to".to_string(),
                        param_type: "address".to_string(),
                        components: vec![],
                    },
                    AbiParam {
                        name: "amount".to_string(),
                        param_type: "uint256".to_string(),
                        components: vec![],
                    },
                ],
                outputs: vec![AbiParam {
                    name: "".to_string(),
                    param_type: "bool".to_string(),
                    components: vec![],
                }],
                state_mutability: Some("nonpayable".to_string()),
            }
        );
    }

    #[test]
    fn generate_abi_from_sol_resolves_named_struct_into_tuple() {
        let (_d, path) = write_sol(
            "Points.sol",
            r#"pragma solidity ^0.8.0;

struct Point {
    uint256 x;
    uint256 y;
}

interface Points {
    function add(Point a, Point b) external returns (Point);
}
"#,
        );

        let abi = generate_abi_from_sol(&path).unwrap().unwrap();
        let (inputs, outputs) = abi
            .0
            .iter()
            .find_map(|i| match i {
                AbiItem::Function {
                    name,
                    inputs,
                    outputs,
                    ..
                } if name == "add" => Some((inputs, outputs)),
                _ => None,
            })
            .unwrap();
        let expected_components = vec![
            AbiParam {
                name: "".to_string(),
                param_type: "uint256".to_string(),
                components: vec![],
            },
            AbiParam {
                name: "".to_string(),
                param_type: "uint256".to_string(),
                components: vec![],
            },
        ];
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].param_type, "tuple");
        assert_eq!(inputs[0].components, expected_components);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].param_type, "tuple");
        assert_eq!(outputs[0].components, expected_components);
    }

    #[test]
    fn generate_abi_from_sol_strips_storage_locations_and_inline_tuples() {
        let (_d, path) = write_sol(
            "Mixed.sol",
            r#"pragma solidity ^0.8.0;
interface Mixed {
    function f(string calldata s, uint256[] memory arr, (uint256,address) pair) external view;
}
"#,
        );

        let abi = generate_abi_from_sol(&path).unwrap().unwrap();
        let inputs = abi
            .0
            .iter()
            .find_map(|i| match i {
                AbiItem::Function { name, inputs, .. } if name == "f" => Some(inputs.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            inputs[0],
            AbiParam {
                name: "s".to_string(),
                param_type: "string".to_string(),
                components: vec![],
            }
        );
        assert_eq!(
            inputs[1],
            AbiParam {
                name: "arr".to_string(),
                param_type: "uint256[]".to_string(),
                components: vec![],
            }
        );
        assert_eq!(inputs[2].param_type, "tuple");
        assert_eq!(inputs[2].name, "pair");
        assert_eq!(
            inputs[2].components,
            vec![
                AbiParam {
                    name: "".to_string(),
                    param_type: "uint256".to_string(),
                    components: vec![],
                },
                AbiParam {
                    name: "".to_string(),
                    param_type: "address".to_string(),
                    components: vec![],
                },
            ]
        );
    }

    // --- extract_sol_path_from_source ---

    #[test]
    fn extract_sol_path_valid() {
        let source = r#"
            #[pvm_contract_macros::contract("MyToken.sol", buffer = 256)]
            mod c {}
        "#;
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("MyToken.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_with_directory() {
        let source = r#"
            #[contract("interfaces/IToken.sol")]
            mod c {}
        "#;
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("interfaces/IToken.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_no_contract_attr() {
        assert_eq!(extract_sol_path_from_source("fn main() {}"), None);
    }

    #[test]
    fn extract_sol_path_non_sol_extension() {
        let source = r#"
            #[contract("MyToken.json")]
            mod c {}
        "#;
        assert_eq!(extract_sol_path_from_source(source), None);
    }

    #[test]
    fn extract_sol_path_with_whitespace_after_paren() {
        let source = r#"
            #[contract( "MyToken.sol" )]
            mod c {}
        "#;
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("MyToken.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_with_newline_formatting() {
        let source = "
            #[contract(\n    \"MyToken.sol\"\n)]
            mod c {}
        ";
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("MyToken.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_missing_closing_quote() {
        // Syntactically broken source: `syn::parse_file` returns Err and the
        // function returns the conservative `None` rather than guessing.
        let source = r#"#[contract("MyToken.sol)]"#;
        assert_eq!(extract_sol_path_from_source(source), None);
    }

    #[test]
    fn extract_sol_path_ignores_comment_before_real_attribute() {
        // The commented-out attribute must not shadow the real one below it.
        let source = r#"
            // #[pvm_contract_sdk::contract("Wrong.sol")]
            #[pvm_contract_sdk::contract("Right.sol")]
            mod c {}
        "#;
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("Right.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_with_allocator_only() {
        // `#[contract(allocator = "pico")]` has no .sol arg, must return None.
        let source = r#"
            #[pvm_contract_sdk::contract(allocator = "pico")]
            mod c {}
        "#;
        assert_eq!(extract_sol_path_from_source(source), None);
    }

    // --- has_contract_macro ---

    #[test]
    fn has_contract_macro_with_args() {
        let source = r#"
            #[pvm_contract_macros::contract(allocator = "pico")]
            mod c {}
        "#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_with_sol_path() {
        let source = r#"
            #[pvm_contract_macros::contract("MyToken.sol")]
            mod c {}
        "#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_no_args() {
        let source = r#"
            #[pvm_contract_macros::contract]
            mod c {}
        "#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_dsl_binary() {
        let source = r#"use pvm_contract_builder_dsl::{ContractBuilder, solidity_selector};"#;
        assert!(!has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_ignores_comment() {
        // A `#[contract(...)]`-shape inside a comment is not a real attribute.
        let source = r#"
            // #[pvm_contract_sdk::contract("Foo.sol")]
            fn main() {}
        "#;
        assert!(!has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_handles_bare_path_form() {
        // `#[contract]` (no `::`-prefix) must still match.
        let source = r#"
            use pvm_contract_sdk::contract;
            #[contract]
            mod c {}
        "#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_returns_false_on_syntax_error() {
        // Source that doesn't parse: function returns the conservative default.
        let source = r#"#[contract(unclosed"#;
        assert!(!has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_finds_attribute_inside_nested_mod() {
        let source = r#"
            mod outer {
                #[pvm_contract_sdk::contract("Foo.sol")]
                pub mod inner {}
            }
        "#;
        assert!(has_contract_macro(source));
    }

    // --- has_slot_fields ---

    #[test]
    fn has_slot_fields_detects_real_attribute() {
        let source = r#"
            struct S {
                #[slot(0)]
                x: u32,
            }
        "#;
        assert!(has_slot_fields(source));
    }

    #[test]
    fn has_slot_fields_ignores_comment() {
        // a `#[slot(...)]` shape inside a comment must not trip detection.
        let source = r#"
            struct S {
                // #[slot(0)]
                x: u32,
            }
        "#;
        assert!(!has_slot_fields(source));
    }

    #[test]
    fn has_slot_fields_finds_attribute_inside_nested_mod() {
        let source = r#"
            mod contract {
                pub struct S {
                    #[slot(0)]
                    total: u32,
                }
            }
        "#;
        assert!(has_slot_fields(source));
    }

    #[test]
    fn has_slot_fields_returns_false_when_absent() {
        let source = r#"
            struct S {
                x: u32,
            }
        "#;
        assert!(!has_slot_fields(source));
    }

    // --- parse_sol_params ---

    // --- parse_sol_function_line ---

    // --- generate_abi_from_sol (uses temp files) ---

    #[test]
    fn generate_abi_from_sol_valid_interface() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("IToken.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"// SPDX-License-Identifier: MIT
interface IToken {{
    function totalSupply() external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let json = serde_json::to_value(&abi).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 7);
        assert_eq!(arr[0]["name"], "totalSupply");
        assert_eq!(arr[1]["name"], "transfer");
        assert_eq!(arr[2]["name"], "InvalidCalldata");
        assert_eq!(arr[2]["type"], "error");
        assert_eq!(arr[3]["name"], "CalldataTooLarge");
        assert_eq!(arr[3]["type"], "error");
        assert_eq!(arr[4]["name"], "NoSelector");
        assert_eq!(arr[4]["type"], "error");
        assert_eq!(arr[5]["name"], "UnknownSelector");
        assert_eq!(arr[5]["type"], "error");
        assert_eq!(arr[6]["name"], "NonPayableValueReceived");
        assert_eq!(arr[6]["type"], "error");
    }

    #[test]
    fn generate_abi_from_sol_empty_file() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Empty.sol");
        std::fs::write(&sol_path, "// empty").unwrap();

        assert!(generate_abi_from_sol(&sol_path).unwrap().is_none());
    }

    #[test]
    fn generate_abi_from_sol_no_functions() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("IEmpty.sol");
        std::fs::write(&sol_path, "interface IEmpty {}").unwrap();

        assert!(generate_abi_from_sol(&sol_path).unwrap().is_none());
    }

    // --- resolve_bin_source_path (uses temp dirs with Cargo.toml) ---

    #[test]
    fn resolve_bin_path_explicit_bin_entry() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "myproject"
version = "0.1.0"

[[bin]]
name = "mybin"
path = "src/custom.rs"
"#,
        )
        .unwrap();

        let path = resolve_bin_source_path(dir.path(), "mybin").unwrap();
        assert_eq!(path, dir.path().join("src/custom.rs"));
    }

    #[test]
    fn resolve_bin_path_bin_entry_without_path() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "myproject"
version = "0.1.0"

[[bin]]
name = "mybin"
"#,
        )
        .unwrap();

        let path = resolve_bin_source_path(dir.path(), "mybin").unwrap();
        assert_eq!(path, dir.path().join("src/bin/mybin.rs"));
    }

    #[test]
    fn resolve_bin_path_package_name_match() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "mybin"
version = "0.1.0"
"#,
        )
        .unwrap();

        let path = resolve_bin_source_path(dir.path(), "mybin").unwrap();
        assert_eq!(path, dir.path().join("src/main.rs"));
    }

    #[test]
    fn resolve_bin_path_fallback() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "other"
version = "0.1.0"
"#,
        )
        .unwrap();

        let path = resolve_bin_source_path(dir.path(), "mybin").unwrap();
        assert_eq!(path, dir.path().join("src/bin/mybin.rs"));
    }

    // --- Error parsing ---

    #[test]
    fn generate_abi_from_sol_includes_errors() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("MyToken.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"interface IToken {{
    function transfer(address to, uint256 amount) external;
    error InsufficientBalance(address account, uint256 required);
    error Unauthorized();
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let json = serde_json::to_value(&abi).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 8);
        assert_eq!(arr[0]["name"], "transfer");
        assert_eq!(arr[0]["type"], "function");
        assert_eq!(arr[1]["name"], "InsufficientBalance");
        assert_eq!(arr[1]["type"], "error");
        assert_eq!(arr[2]["name"], "Unauthorized");
        assert_eq!(arr[2]["type"], "error");
        assert_eq!(arr[3]["name"], "InvalidCalldata");
        assert_eq!(arr[3]["type"], "error");
        assert_eq!(arr[4]["name"], "CalldataTooLarge");
        assert_eq!(arr[4]["type"], "error");
        assert_eq!(arr[5]["name"], "NoSelector");
        assert_eq!(arr[5]["type"], "error");
        assert_eq!(arr[6]["name"], "UnknownSelector");
        assert_eq!(arr[6]["type"], "error");
        assert_eq!(arr[7]["name"], "NonPayableValueReceived");
        assert_eq!(arr[7]["type"], "error");
    }

    // --- Multiline declaration support ---

    #[test]
    fn generate_abi_from_sol_multiline_function() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Multi.sol");
        std::fs::write(
            &sol_path,
            "interface Multi {\n    function transfer(\n        address to,\n        uint256 amount\n    ) external;\n}",
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let func = abi
            .0
            .iter()
            .find(|item| matches!(item, AbiItem::Function { name, .. } if name == "transfer"))
            .expect("should parse multiline function");
        if let AbiItem::Function { inputs, .. } = func {
            assert_eq!(inputs.len(), 2);
            assert_eq!(inputs[0].param_type, "address");
            assert_eq!(inputs[0].name, "to");
            assert_eq!(inputs[1].param_type, "uint256");
            assert_eq!(inputs[1].name, "amount");
        }
    }

    // --- Constructor parsing ---

    // --- Tuple type expansion in parse_sol_params ---

    // --- Constructor in generate_abi_from_sol ---

    #[test]
    fn generate_abi_from_sol_includes_constructor() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Token.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"// SPDX-License-Identifier: MIT
interface Token {{
    constructor(address owner, uint256 supply);
    function totalSupply() external view returns (uint256);
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();

        // Find constructor entry
        let ctor = abi
            .0
            .iter()
            .find(|item| matches!(item, AbiItem::Constructor { .. }))
            .expect("ABI should include constructor");
        assert_eq!(
            *ctor,
            AbiItem::Constructor {
                inputs: vec![
                    AbiParam {
                        name: "owner".into(),
                        param_type: "address".into(),
                        components: vec![],
                    },
                    AbiParam {
                        name: "supply".into(),
                        param_type: "uint256".into(),
                        components: vec![],
                    },
                ],
                state_mutability: Some("nonpayable".into()),
            }
        );

        // Find function entry
        let func = abi
            .0
            .iter()
            .find(|item| matches!(item, AbiItem::Function { name, .. } if name == "totalSupply"))
            .expect("ABI should include totalSupply");
        assert_eq!(
            *func,
            AbiItem::Function {
                name: "totalSupply".into(),
                inputs: vec![],
                outputs: vec![AbiParam {
                    name: "".into(),
                    param_type: "uint256".into(),
                    components: vec![],
                }],
                state_mutability: Some("view".into()),
            }
        );
    }

    #[test]
    fn generate_abi_from_sol_multiline_constructor() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Token.sol");
        std::fs::write(
            &sol_path,
            "interface Token {\n    constructor(\n        address owner,\n        uint256 supply\n    ) payable;\n    function totalSupply() external view returns (uint256);\n}",
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let ctor = abi
            .0
            .iter()
            .find(|item| matches!(item, AbiItem::Constructor { .. }))
            .expect("ABI should include multiline constructor");
        assert_eq!(
            *ctor,
            AbiItem::Constructor {
                inputs: vec![
                    AbiParam {
                        name: "owner".into(),
                        param_type: "address".into(),
                        components: vec![],
                    },
                    AbiParam {
                        name: "supply".into(),
                        param_type: "uint256".into(),
                        components: vec![],
                    },
                ],
                state_mutability: Some("payable".into()),
            }
        );
    }

    // --- has_slot_fields ---

    #[test]
    fn detects_slot_attr() {
        assert!(has_slot_fields(
            "pub struct MyToken { #[slot(0)] total_supply: Lazy<U256> }"
        ));
    }

    #[test]
    fn no_slot_attr_returns_false() {
        assert!(!has_slot_fields("pub struct MyToken;"));
    }

    // --- Event parsing ---

    #[test]
    fn generate_abi_from_sol_includes_events() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Events.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"interface IEvents {{
    function setValue(uint256 val) external;
    event ValueChanged(address indexed who, uint256 oldValue, uint256 newValue);
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let json = serde_json::to_value(&abi).unwrap();
        let arr = json.as_array().unwrap();

        let event = arr.iter().find(|item| item["type"] == "event").unwrap();
        assert_eq!(event["name"], "ValueChanged");
        assert_eq!(event["anonymous"], false);

        let inputs = event["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs[0]["name"], "who");
        assert_eq!(inputs[0]["type"], "address");
        assert_eq!(inputs[0]["indexed"], true);
        assert_eq!(inputs[1]["name"], "oldValue");
        assert_eq!(inputs[1]["type"], "uint256");
        assert_eq!(inputs[1]["indexed"], false);
        assert_eq!(inputs[2]["name"], "newValue");
        assert_eq!(inputs[2]["type"], "uint256");
        assert_eq!(inputs[2]["indexed"], false);
    }

    #[test]
    fn generate_abi_from_sol_parses_multiline_events() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Events.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"interface IEvents {{
    event Transfer(
        address indexed from,
        address indexed to,
        uint256 value
    );
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let json = serde_json::to_value(&abi).unwrap();
        let arr = json.as_array().unwrap();

        let event = arr.iter().find(|item| item["type"] == "event").unwrap();
        assert_eq!(event["name"], "Transfer");

        let inputs = event["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs[0]["name"], "from");
        assert_eq!(inputs[0]["indexed"], true);
        assert_eq!(inputs[1]["name"], "to");
        assert_eq!(inputs[1]["indexed"], true);
        assert_eq!(inputs[2]["name"], "value");
        assert_eq!(inputs[2]["indexed"], false);
    }
}

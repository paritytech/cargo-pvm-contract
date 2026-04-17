use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::Path, process::Command};
use toml_edit::DocumentMut;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct AbiJson(Vec<AbiItem>);

#[derive(Debug, Serialize, Deserialize, PartialEq)]
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
    Error {
        name: String,
        inputs: Vec<AbiParam>,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct AbiParam {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
}

pub fn generate_abi_for_bin(
    manifest_dir: &Path,
    bin_name: &str,
    target_root: Option<&Path>,
) -> Result<Option<AbiJson>> {
    generate_abi_via_feature(manifest_dir, bin_name, target_root)
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

    // ABI generation requires either a `.sol` file or the `#[contract]` macro
    // (which generates `__abi_json()` under `--features abi-gen`). DSL-based
    // contracts don't use the macro and are expected to handle ABI themselves.
    if !has_contract_macro(&source_content) {
        return Ok(None);
    }

    let target_dir = match target_root {
        Some(root) => root.join("abi-gen-target"),
        None => super::get_target_root().join("abi-gen-target"),
    };
    let manifest_path = manifest_dir.join("Cargo.toml");

    // The project's .cargo/config.toml targets RISC-V with build-std=core,alloc.
    // The abi-gen binary needs std and must run on the host, so we override both:
    // --target forces the host triple, build-std adds std to the sysroot rebuild.
    let host = get_host_triple()?;

    // Remove RUSTUP_TOOLCHAIN only when rust-toolchain.toml exists, matching
    // build_elf's behavior. Without a toolchain file we keep the inherited
    // toolchain (e.g. nightly passed via `cargo +nightly`).
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
        .arg("abi-gen")
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

    let abi: AbiJson = serde_json::from_str(&stdout_str)
        .context("Failed to parse ABI JSON from abi-gen output")?;

    Ok(Some(abi))
}

/// Detect whether the source uses the `#[contract]` attribute macro. Matches
/// both `::contract]` (no args) and `::contract(` (with args). Used to skip
/// ABI generation for DSL-based contracts that don't use the macro.
pub(crate) fn has_contract_macro(source: &str) -> bool {
    source.contains("::contract]") || source.contains("::contract(")
}

pub(crate) fn extract_sol_path_from_source(source: &str) -> Option<String> {
    if let Some(start) = source.find("contract(\"") {
        let after_quote = &source[start + 10..];
        if let Some(end) = after_quote.find('"') {
            let path = &after_quote[..end];
            if path.ends_with(".sol") {
                return Some(path.to_string());
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

    let mut items = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("function ")
            && let Some(func) = parse_sol_function_line(line)
        {
            items.push(func);
        }
        if line.starts_with("error ")
            && let Some(err) = parse_sol_error_line(line)
        {
            items.push(err);
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

/// Find the index of the closing `)` that matches the `(` at `start`.
fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in s[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split parameters at top-level commas, respecting nested parens and brackets.
fn split_top_level(params_str: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut depth = 0;
    let mut current = String::new();

    for ch in params_str.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                if !current.trim().is_empty() {
                    params.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        params.push(current.trim().to_string());
    }

    params
}

pub(crate) fn parse_sol_function_line(line: &str) -> Option<AbiItem> {
    let line = line.strip_prefix("function ")?.trim();

    let paren_start = line.find('(')?;
    let name = line[..paren_start].trim().to_string();

    let paren_end = find_matching_paren(line, paren_start)?;
    let params_str = &line[paren_start + 1..paren_end];
    let inputs = parse_sol_params(params_str);

    let outputs = if let Some(returns_idx) = line.find("returns") {
        let after_returns = &line[returns_idx + 7..];
        if let Some(start) = after_returns.find('(') {
            let abs_start = returns_idx + 7 + start;
            if let Some(end) = find_matching_paren(line, abs_start) {
                parse_sol_params(&line[abs_start + 1..end])
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

fn parse_sol_error_line(line: &str) -> Option<AbiItem> {
    let line = line.strip_prefix("error ")?.trim();

    let paren_start = line.find('(')?;
    let name = line[..paren_start].trim().to_string();

    let paren_end = find_matching_paren(line, paren_start)?;
    let params_str = &line[paren_start + 1..paren_end];
    let inputs = parse_sol_params(params_str);

    Some(AbiItem::Error { name, inputs })
}

pub(crate) fn parse_sol_params(params_str: &str) -> Vec<AbiParam> {
    if params_str.trim().is_empty() {
        return vec![];
    }

    split_top_level(params_str)
        .into_iter()
        .filter_map(|p| {
            let p = p.trim().to_string();
            let parts: Vec<&str> = p.split_whitespace().collect();
            if parts.is_empty() {
                return None;
            }
            let param_type = parts[0].to_string();
            let name = parts[1..]
                .iter()
                .find(|s| !matches!(**s, "memory" | "calldata" | "storage"))
                .map(|s| s.to_string())
                .unwrap_or_default();
            Some(AbiParam { name, param_type })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    // --- extract_sol_path_from_source ---

    #[test]
    fn extract_sol_path_valid() {
        let source = r#"#[pvm_contract_macros::contract("MyToken.sol", buffer = 256)]"#;
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("MyToken.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_with_directory() {
        let source = r#"#[contract("interfaces/IToken.sol")]"#;
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
        let source = r#"#[contract("MyToken.json")]"#;
        assert_eq!(extract_sol_path_from_source(source), None);
    }

    #[test]
    fn extract_sol_path_missing_closing_quote() {
        let source = r#"#[contract("MyToken.sol)]"#;
        // No closing quote before ) so find('"') finds the one before MyToken
        // Actually "contract(\"" consumes up to the quote, then after_quote starts at MyToken.sol)
        // find('"') returns None since there's no second quote
        assert_eq!(extract_sol_path_from_source(source), None);
    }

    // --- has_contract_macro ---

    #[test]
    fn has_contract_macro_with_args() {
        let source = r#"#[pvm_contract_macros::contract(allocator = "pico")]"#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_with_sol_path() {
        let source = r#"#[pvm_contract_macros::contract("MyToken.sol")]"#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_no_args() {
        let source = r#"#[pvm_contract_macros::contract]"#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_dsl_binary() {
        let source = r#"use pvm_contract_builder_dsl::{ContractBuilder, solidity_selector};"#;
        assert!(!has_contract_macro(source));
    }

    // --- parse_sol_params ---

    #[test]
    fn parse_params_empty() {
        assert_eq!(parse_sol_params(""), Vec::<AbiParam>::new());
    }

    #[test]
    fn parse_params_whitespace_only() {
        assert_eq!(parse_sol_params("   "), Vec::<AbiParam>::new());
    }

    #[test]
    fn parse_params_single_with_name() {
        let params = parse_sol_params("uint256 amount");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "uint256");
        assert_eq!(params[0].name, "amount");
    }

    #[test]
    fn parse_params_single_type_only() {
        let params = parse_sol_params("uint256");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "uint256");
        assert_eq!(params[0].name, "");
    }

    #[test]
    fn parse_params_multiple() {
        let params = parse_sol_params("address to, uint256 amount");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].param_type, "address");
        assert_eq!(params[0].name, "to");
        assert_eq!(params[1].param_type, "uint256");
        assert_eq!(params[1].name, "amount");
    }

    // --- parse_sol_function_line ---

    #[test]
    fn parse_function_simple_transfer() {
        let item =
            parse_sol_function_line("function transfer(address to, uint256 amount) external")
                .unwrap();
        assert_eq!(
            item,
            AbiItem::Function {
                name: "transfer".to_string(),
                inputs: vec![
                    AbiParam {
                        name: "to".to_string(),
                        param_type: "address".to_string()
                    },
                    AbiParam {
                        name: "amount".to_string(),
                        param_type: "uint256".to_string()
                    },
                ],
                outputs: vec![],
                state_mutability: Some("nonpayable".to_string()),
            }
        );
    }

    #[test]
    fn parse_function_view_with_returns() {
        let item = parse_sol_function_line(
            "function balanceOf(address account) external view returns (uint256)",
        )
        .unwrap();
        assert_eq!(
            item,
            AbiItem::Function {
                name: "balanceOf".to_string(),
                inputs: vec![AbiParam {
                    name: "account".to_string(),
                    param_type: "address".to_string()
                }],
                outputs: vec![AbiParam {
                    name: "".to_string(),
                    param_type: "uint256".to_string()
                }],
                state_mutability: Some("view".to_string()),
            }
        );
    }

    #[test]
    fn parse_function_no_params() {
        let item =
            parse_sol_function_line("function totalSupply() external view returns (uint256)")
                .unwrap();
        assert_eq!(
            item,
            AbiItem::Function {
                name: "totalSupply".to_string(),
                inputs: vec![],
                outputs: vec![AbiParam {
                    name: "".to_string(),
                    param_type: "uint256".to_string()
                }],
                state_mutability: Some("view".to_string()),
            }
        );
    }

    #[test]
    fn parse_function_pure_mutability() {
        let item =
            parse_sol_function_line("function add(uint256 a, uint256 b) pure returns (uint256)")
                .unwrap();
        if let AbiItem::Function {
            state_mutability, ..
        } = &item
        {
            assert_eq!(state_mutability.as_deref(), Some("pure"));
        } else {
            panic!("expected Function");
        }
    }

    #[test]
    fn parse_function_payable_mutability() {
        let item =
            parse_sol_function_line("function deposit() external payable returns (bool)").unwrap();
        if let AbiItem::Function {
            state_mutability, ..
        } = &item
        {
            assert_eq!(state_mutability.as_deref(), Some("payable"));
        } else {
            panic!("expected Function");
        }
    }

    #[test]
    fn parse_function_no_returns() {
        let item = parse_sol_function_line("function setOwner(address newOwner) external").unwrap();
        if let AbiItem::Function { outputs, .. } = &item {
            assert!(outputs.is_empty());
        } else {
            panic!("expected Function");
        }
    }

    #[test]
    fn parse_function_not_a_function() {
        assert!(parse_sol_function_line("event Transfer(address,address,uint256)").is_none());
    }

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
        assert_eq!(arr.len(), 6);
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

    #[test]
    fn parse_params_strips_data_location_qualifiers() {
        let params = parse_sol_params("string calldata s, uint256[] memory arr");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].param_type, "string");
        assert_eq!(params[0].name, "s");
        assert_eq!(params[1].param_type, "uint256[]");
        assert_eq!(params[1].name, "arr");
    }

    #[test]
    fn parse_params_strips_qualifier_without_name() {
        let params = parse_sol_params("string memory");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "string");
        assert_eq!(params[0].name, "");
    }

    #[test]
    fn parse_function_with_tuple_param() {
        let item = parse_sol_function_line(
            "function foo((address,uint256) param) external returns (bool)",
        )
        .unwrap();
        if let AbiItem::Function {
            name,
            inputs,
            outputs,
            ..
        } = &item
        {
            assert_eq!(name, "foo");
            assert_eq!(inputs.len(), 1);
            assert_eq!(inputs[0].param_type, "(address,uint256)");
            assert_eq!(outputs.len(), 1);
            assert_eq!(outputs[0].param_type, "bool");
        } else {
            panic!("expected Function");
        }
    }

    // --- Error parsing ---

    #[test]
    fn parse_error_with_params() {
        assert_eq!(
            parse_sol_error_line(
                "error InsufficientBalance(address account, uint256 required, uint256 available);",
            )
            .unwrap(),
            AbiItem::Error {
                name: "InsufficientBalance".to_string(),
                inputs: vec![
                    AbiParam {
                        name: "account".to_string(),
                        param_type: "address".to_string()
                    },
                    AbiParam {
                        name: "required".to_string(),
                        param_type: "uint256".to_string()
                    },
                    AbiParam {
                        name: "available".to_string(),
                        param_type: "uint256".to_string()
                    },
                ],
            }
        );
    }

    #[test]
    fn parse_error_no_params() {
        assert_eq!(
            parse_sol_error_line("error Unauthorized();").unwrap(),
            AbiItem::Error {
                name: "Unauthorized".to_string(),
                inputs: vec![],
            }
        );
    }

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
        assert_eq!(arr.len(), 7);
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
    }
}

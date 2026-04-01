use anyhow::{Context, Result};
use askama::Template;
use convert_case::{Case, Casing};
use serde::Deserialize;
use std::io::Write;
use std::{fs, path::PathBuf, process::Command};

const BUILDER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Pinned nightly toolchain for scaffolded contract projects.
/// Must satisfy the `rust-version` MSRV in the Cargo.toml template (currently 1.92).
const NIGHTLY_TOOLCHAIN: &str = "nightly-2026-02-01";

#[derive(Template)]
#[template(path = "scaffold/cargo_toml.txt")]
struct CargoTomlTemplate<'a> {
    contract_name: &'a str,
    bin_source: &'a str,
    use_dsl: bool,
    use_alloc: bool,
    builder_version: &'a str,
    local_path: Option<String>,
}

#[derive(Template)]
#[template(path = "scaffold/contract_macro.rs.txt")]
struct ContractMacroTemplate<'a> {
    use_alloc: bool,
    sol_file_name: Option<&'a str>,
    functions: Vec<MacroFunctionInfo>,
}

#[derive(Template)]
#[template(path = "scaffold/contract_dsl.rs.txt")]
struct ContractDslTemplate {
    use_alloc: bool,
    functions: Vec<DslFunctionInfo>,
}

#[derive(Template)]
#[template(path = "scaffold/build.rs.txt")]
struct BuildRsTemplate {
    use_dsl: bool,
}

struct MacroFunctionInfo {
    name_snake: String,
    params: String,
    return_type: String,
}

struct DslFunctionInfo {
    selector_const: String,
    solidity_signature: String,
    name_snake: String,
    params: Vec<DslParam>,
    return_rust_type: String,
}

struct DslParam {
    name: String,
    decode_expr: String,
}

#[derive(Debug, Deserialize)]
struct SolcOutput {
    contracts: std::collections::HashMap<String, std::collections::HashMap<String, ContractInfo>>,
}

#[derive(Debug, Deserialize)]
struct ContractInfo {
    metadata: String,
}

#[derive(Debug, Deserialize)]
struct ContractMetadata {
    output: MetadataOutput,
}

#[derive(Debug, Deserialize)]
struct MetadataOutput {
    abi: Vec<AbiItem>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
enum AbiItem {
    #[serde(rename = "function")]
    Function {
        name: String,
        inputs: Vec<AbiInput>,
        outputs: Vec<AbiOutput>,
        #[serde(rename = "stateMutability")]
        _state_mutability: String,
    },
    #[serde(rename = "event")]
    Event {
        #[serde(rename = "name")]
        _name: String,
        #[serde(rename = "inputs")]
        _inputs: Vec<AbiInput>,
    },
    #[serde(rename = "error")]
    Error {
        #[serde(rename = "name")]
        _name: String,
        #[serde(rename = "inputs")]
        _inputs: Vec<AbiInput>,
    },
    #[serde(rename = "constructor")]
    Constructor {
        #[serde(rename = "inputs")]
        _inputs: Vec<AbiInput>,
    },
}

#[derive(Debug, Deserialize, Clone)]
struct AbiInput {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
    #[serde(rename = "indexed")]
    _indexed: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
struct AbiOutput {
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "type")]
    type_name: String,
}

pub fn init_new_contract(contract_name: &str, use_dsl: bool, use_alloc: bool) -> Result<()> {
    let contract_name = contract_name.to_case(Case::Kebab);
    let target_dir = std::env::current_dir()?.join(&contract_name);
    if target_dir.exists() {
        anyhow::bail!("Directory already exists: {target_dir:?}");
    }

    fs::create_dir(&target_dir)
        .with_context(|| format!("Failed to create directory: {target_dir:?}"))?;

    let (target_json_path, target_json_name) = resolve_target_json()?;
    let target_json_dest = target_dir.join(&target_json_name);
    fs::copy(&target_json_path, &target_json_dest).with_context(|| {
        format!(
            "Failed to copy target JSON from {} to {}",
            target_json_path.display(),
            target_json_dest.display()
        )
    })?;

    let cargo_config_dir = target_dir.join(".cargo");
    fs::create_dir(&cargo_config_dir)?;
    fs::write(
        cargo_config_dir.join("config.toml"),
        format!(
            "[build]\n target = \"{target_json_name}\"\n\n[unstable]\n build-std = [\"core\", \"alloc\"]\n json-target-spec = true\n\n[env]\n RUSTC_BOOTSTRAP = \"1\"\n"
        ),
    )?;

    fs::write(target_dir.join(".gitignore"), "/target\n*.polkavm\n")?;
    fs::write(
        target_dir.join("rust-toolchain.toml"),
        format!("[toolchain]\nchannel = \"{NIGHTLY_TOOLCHAIN}\"\ncomponents = [\"rust-src\"]\n"),
    )?;

    fs::create_dir(target_dir.join("src"))?;
    let lib_rs_content = if use_dsl {
        generate_dsl_contract(use_alloc, vec![])?
    } else {
        generate_macro_contract(use_alloc, None, vec![])?
    };
    fs::write(
        target_dir.join(format!("src/{contract_name}.rs")),
        lib_rs_content,
    )?;

    let build_rs_content = generate_build_rs(use_dsl)?;
    fs::write(target_dir.join("build.rs"), build_rs_content)?;

    let cargo_toml_content =
        generate_cargo_toml(&contract_name, &contract_name, use_dsl, use_alloc)?;
    fs::write(target_dir.join("Cargo.toml"), cargo_toml_content)?;

    println!("Successfully initialized contract project: {target_dir:?}");
    println!("\nNext steps:");
    println!("  cd {contract_name}");
    println!("  cargo build");
    Ok(())
}

/// Create a new contract project from a Solidity file.
pub fn init_from_solidity_file(
    sol_file: &str,
    contract_name: &str,
    use_dsl: bool,
    use_alloc: bool,
) -> Result<()> {
    let sol_path = PathBuf::from(sol_file);
    if !sol_path.exists() {
        anyhow::bail!("Solidity file not found: {sol_file}");
    }

    let sol_abs_path = sol_path
        .canonicalize()
        .with_context(|| format!("Failed to get absolute path for {sol_file}"))?;

    let sol_file_name = sol_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid file name"))?
        .to_string();

    let sol_content = fs::read(&sol_abs_path)
        .with_context(|| format!("Failed to read Solidity file: {sol_abs_path:?}"))?;

    init_from_example_files_inner(
        &sol_content,
        &sol_file_name,
        None,
        contract_name,
        use_dsl,
        use_alloc,
    )
}

pub fn init_from_example_files(
    sol_contents: &[u8],
    sol_file_name: &str,
    rust_contents: &[u8],
    contract_name: &str,
    use_dsl: bool,
) -> Result<()> {
    init_from_example_files_inner(
        sol_contents,
        sol_file_name,
        Some(rust_contents),
        contract_name,
        use_dsl,
        false,
    )
}

fn init_from_example_files_inner(
    sol_contents: &[u8],
    sol_file_name: &str,
    rust_contents: Option<&[u8]>,
    contract_name: &str,
    use_dsl: bool,
    use_alloc: bool,
) -> Result<()> {
    let contract_name = contract_name.to_case(Case::Kebab);
    let sol_file_name = sol_file_name.to_string();

    log::debug!("Extracting metadata from {sol_file_name}");
    let (metadata, actual_contract_name) =
        extract_solc_metadata_from_bytes(sol_contents, &sol_file_name)?;
    let actual_contract_kebab = actual_contract_name.to_case(Case::Kebab);

    // Create project directory
    let target_dir = std::env::current_dir()?.join(&contract_name);
    if target_dir.exists() {
        anyhow::bail!("Directory already exists: {target_dir:?}");
    }
    fs::create_dir(&target_dir)
        .with_context(|| format!("Failed to create directory: {target_dir:?}"))?;

    let (target_json_path, target_json_name) = resolve_target_json()?;
    let target_json_dest = target_dir.join(target_json_name);
    // Read into memory first to avoid race conditions when multiple processes
    // concurrently call polkavm_linker::target_json_path (which writes a shared file).
    let target_json_content = fs::read(&target_json_path).with_context(|| {
        format!(
            "Failed to read target JSON from {}",
            target_json_path.display(),
        )
    })?;
    fs::write(&target_json_dest, &target_json_content).with_context(|| {
        format!(
            "Failed to write target JSON to {}",
            target_json_dest.display()
        )
    })?;

    let target_json_name = target_json_dest
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Target JSON path is missing a file name"))?;

    // Copy .sol file to project
    let target_sol_path = target_dir.join(&sol_file_name);
    fs::write(&target_sol_path, sol_contents)
        .with_context(|| format!("Failed to write {sol_file_name} to {target_sol_path:?}"))?;

    // Create .cargo directory and config
    let cargo_config_dir = target_dir.join(".cargo");
    fs::create_dir(&cargo_config_dir)?;
    fs::write(
        cargo_config_dir.join("config.toml"),
        format!(
            "[build]\n target = \"{target_json_name}\"\n\n[unstable]\n build-std = [\"core\", \"alloc\"]\n json-target-spec = true\n\n[env]\n RUSTC_BOOTSTRAP = \"1\"\n"
        ),
    )?;

    // Create .gitignore
    fs::write(target_dir.join(".gitignore"), "/target\n*.polkavm\n")?;
    fs::write(
        target_dir.join("rust-toolchain.toml"),
        format!("[toolchain]\nchannel = \"{NIGHTLY_TOOLCHAIN}\"\ncomponents = [\"rust-src\"]\n"),
    )?;
    // Generate src/{contract}.rs
    fs::create_dir(target_dir.join("src"))?;

    let lib_rs_content = if let Some(contents) = rust_contents {
        String::from_utf8(contents.to_vec()).context("Example Rust file is not valid UTF-8")?
    } else if use_dsl {
        let functions = extract_dsl_function_info(&metadata);
        generate_dsl_contract(use_alloc, functions)?
    } else {
        let functions = extract_function_info(&metadata);
        generate_macro_contract(use_alloc, Some(&sol_file_name), functions)?
    };
    fs::write(
        target_dir.join(format!("src/{actual_contract_kebab}.rs")),
        lib_rs_content,
    )?;

    let build_rs_content = generate_build_rs(use_dsl)?;
    fs::write(target_dir.join("build.rs"), build_rs_content)?;

    let cargo_toml_content =
        generate_cargo_toml(&contract_name, &actual_contract_kebab, use_dsl, use_alloc)?;
    fs::write(target_dir.join("Cargo.toml"), cargo_toml_content)?;

    println!("Successfully initialized contract project from {sol_file_name}: {target_dir:?}");
    println!("\nNext steps:");
    println!("  cd {contract_name}");
    println!("  cargo build");
    Ok(())
}

/// Internal helpers for template generation.
fn extract_solc_metadata_from_bytes(
    sol_contents: &[u8],
    sol_file_name: &str,
) -> Result<(ContractMetadata, String)> {
    let sol_content =
        String::from_utf8(sol_contents.to_vec()).context("Solidity file is not valid UTF-8")?;

    let solc_input = serde_json::json!({
        "language": "Solidity",
        "sources": {
            sol_file_name: {
                "content": sol_content
            }
        },
        "settings": {
            "outputSelection": {
                "*": {
                    "*": ["metadata"]
                }
            }
        }
    });

    let solc_input_str = serde_json::to_string(&solc_input)?;

    let mut child = Command::new("solc")
        .arg("--standard-json")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn solc. Make sure solc is installed and in PATH.")?;

    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("Failed to open stdin"))?
        .write_all(solc_input_str.as_bytes())?;

    let output_result = child
        .wait_with_output()
        .context("Failed to wait for solc")?;

    if !output_result.status.success() {
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        anyhow::bail!("solc failed: {stderr}");
    }

    log::debug!(
        "solc stdout: {}",
        String::from_utf8_lossy(&output_result.stdout)
    );

    let solc_output: SolcOutput =
        serde_json::from_slice(&output_result.stdout).with_context(|| {
            format!(
                "Failed to parse solc output. Output was: {}",
                String::from_utf8_lossy(&output_result.stdout)
            )
        })?;

    // Extract metadata from the first contract
    let contracts_for_file = solc_output
        .contracts
        .get(sol_file_name)
        .ok_or_else(|| anyhow::anyhow!("No contract found in solc output"))?;

    let (contract_name, contract_info) = contracts_for_file
        .iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No contract found in solc output"))?;

    let metadata: ContractMetadata = serde_json::from_str(&contract_info.metadata)
        .context("Failed to parse contract metadata")?;

    Ok((metadata, contract_name.clone()))
}

fn generate_macro_contract(
    use_alloc: bool,
    sol_file_name: Option<&str>,
    functions: Vec<MacroFunctionInfo>,
) -> Result<String> {
    ContractMacroTemplate {
        use_alloc,
        sol_file_name,
        functions,
    }
    .render()
    .context("Failed to render macro contract template")
}

fn generate_dsl_contract(use_alloc: bool, functions: Vec<DslFunctionInfo>) -> Result<String> {
    ContractDslTemplate {
        use_alloc,
        functions,
    }
    .render()
    .context("Failed to render dsl contract template")
}

fn extract_function_info(metadata: &ContractMetadata) -> Vec<MacroFunctionInfo> {
    metadata
        .output
        .abi
        .iter()
        .filter_map(|item| match item {
            AbiItem::Function {
                name,
                inputs,
                outputs,
                ..
            } => {
                let name_snake = name.to_case(Case::Snake);
                let params = inputs
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let param_name = if p.name.is_empty() {
                            format!("arg{i}")
                        } else {
                            p.name.to_case(Case::Snake)
                        };
                        format!("{param_name}: {}", solidity_to_rust_type(&p.type_name))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let return_type = if outputs.is_empty() {
                    "Result<(), Error>".to_string()
                } else if outputs.len() == 1 {
                    format!(
                        "Result<{}, Error>",
                        solidity_to_rust_type(&outputs[0].type_name)
                    )
                } else {
                    let types = outputs
                        .iter()
                        .map(|o| solidity_to_rust_type(&o.type_name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("Result<({types}), Error>")
                };
                Some(MacroFunctionInfo {
                    name_snake,
                    params,
                    return_type,
                })
            }
            _ => None,
        })
        .collect()
}

fn extract_dsl_function_info(metadata: &ContractMetadata) -> Vec<DslFunctionInfo> {
    metadata
        .output
        .abi
        .iter()
        .filter_map(|item| match item {
            AbiItem::Function {
                name,
                inputs,
                outputs,
                ..
            } => {
                let name_snake = name.to_case(Case::Snake);
                let screaming = name_snake.to_case(Case::ScreamingSnake);
                let selector_const = format!("{screaming}_SELECTOR");

                // Build Solidity signature like "transfer(address,uint256)"
                let sol_param_types: Vec<&str> =
                    inputs.iter().map(|p| p.type_name.as_str()).collect();
                let sol_params = sol_param_types.join(",");
                let solidity_signature = format!("{name}({sol_params})");

                // Build decode expressions for each parameter
                let mut offset_expr = String::new();
                let params: Vec<DslParam> = inputs
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let param_name = if p.name.is_empty() {
                            format!("arg{i}")
                        } else {
                            p.name.to_case(Case::Snake)
                        };
                        let rust_type = solidity_to_dsl_decode_type(&p.type_name);
                        let decode_expr = if i == 0 {
                            format!("{rust_type}::decode_at(input, 0)")
                        } else {
                            format!("{rust_type}::decode_at(input, {offset_expr})")
                        };

                        // Accumulate offset for next parameter
                        let size_expr = format!("<{rust_type} as StaticEncodedLen>::ENCODED_SIZE");
                        if i == 0 {
                            offset_expr = size_expr;
                        } else {
                            offset_expr = format!("{offset_expr} + {size_expr}");
                        }

                        DslParam {
                            name: param_name,
                            decode_expr,
                        }
                    })
                    .collect();

                let return_rust_type = if outputs.is_empty() {
                    "()".to_string()
                } else if outputs.len() == 1 {
                    solidity_to_rust_type(&outputs[0].type_name)
                } else {
                    let types = outputs
                        .iter()
                        .map(|o| solidity_to_rust_type(&o.type_name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({types})")
                };

                Some(DslFunctionInfo {
                    selector_const,
                    solidity_signature,
                    name_snake,
                    params,
                    return_rust_type,
                })
            }
            _ => None,
        })
        .collect()
}

fn solidity_to_rust_type(sol_type: &str) -> String {
    if let Some(inner) = sol_type.strip_suffix("[]") {
        let inner_type = solidity_to_rust_type(inner);
        return format!("Vec<{inner_type}>");
    }

    match sol_type {
        "address" => "Address".to_string(),
        "bool" => "bool".to_string(),
        "string" => "String".to_string(),
        "bytes" => "Vec<u8>".to_string(),
        s if s.starts_with("uint") => {
            let bits: u32 = s[4..].parse().unwrap_or(256);
            match bits {
                8 => "u8".to_string(),
                16 => "u16".to_string(),
                32 => "u32".to_string(),
                64 => "u64".to_string(),
                128 => "u128".to_string(),
                _ => "U256".to_string(),
            }
        }
        s if s.starts_with("int") => {
            let bits: u32 = s[3..].parse().unwrap_or(256);
            match bits {
                8 => "i8".to_string(),
                16 => "i16".to_string(),
                32 => "i32".to_string(),
                64 => "i64".to_string(),
                128 => "i128".to_string(),
                _ => "I256".to_string(),
            }
        }
        "bytes32" => "[u8; 32]".to_string(),
        s if s.starts_with("bytes") && s.len() > 5 => {
            let size: usize = s[5..].parse().unwrap_or(32);
            format!("[u8; {size}]")
        }
        _ => "U256".to_string(),
    }
}

/// Map Solidity types to the Rust type used for `SolDecode::decode_at` in DSL contracts.
fn solidity_to_dsl_decode_type(sol_type: &str) -> String {
    solidity_to_rust_type(sol_type)
}

fn generate_build_rs(use_dsl: bool) -> Result<String> {
    BuildRsTemplate { use_dsl }
        .render()
        .context("Failed to render build.rs template")
}

fn resolve_target_json() -> Result<(PathBuf, String)> {
    let mut args = polkavm_linker::TargetJsonArgs::default();
    args.is_64_bit = true;
    // Scaffolded projects use a pinned nightly >= 1.91, so always emit the new
    // target-spec format (integer `target-pointer-width`) regardless of which
    // rustc is active when the CLI runs.
    args.rustc_version = polkavm_linker::RustcVersion::Rustc_1_91;
    let target_json = polkavm_linker::target_json_path(args)
        .map_err(|e| anyhow::anyhow!("Failed to get target JSON: {e}"))?;

    let target_name = target_json
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Target JSON path is missing a file name"))?
        .to_string();

    Ok((target_json, target_name))
}

fn generate_cargo_toml(
    contract_name: &str,
    bin_source: &str,
    use_dsl: bool,
    use_alloc: bool,
) -> Result<String> {
    let local_path = std::env::var("CARGO_PVM_CONTRACT_PATH")
        .ok()
        .filter(|value| !value.trim().is_empty());

    if let Some(ref path) = local_path {
        let path = std::path::Path::new(path);
        if !path.exists() {
            anyhow::bail!("CARGO_PVM_CONTRACT_PATH does not exist: {}", path.display());
        }
    }

    let template = CargoTomlTemplate {
        contract_name,
        bin_source,
        use_dsl,
        use_alloc,
        builder_version: BUILDER_VERSION,
        local_path,
    };
    template
        .render()
        .context("Failed to render Cargo.toml template")
}

use anyhow::{Context, Result};
use askama::Template;
use convert_case::{Case, Casing};
use serde::Deserialize;
use std::io::Write;
use std::{fs, path::PathBuf, process::Command};
use tiny_keccak::{Hasher, Keccak};

#[derive(Template)]
#[template(path = "scaffold/contract_alloc.rs.txt")]
struct ContractAllocTemplate<'a> {
    sol_file_name: &'a str,
    functions: Vec<AllocFunctionInfo>,
}

#[derive(Template)]
#[template(path = "scaffold/contract_no_alloc.rs.txt")]
struct ContractNoAllocTemplate<'a> {
    contract_name_upper: &'a str,
    selectors: Vec<SelectorConst>,
    events: Vec<EventConst>,
    errors: Vec<ErrorConst>,
    functions: Vec<NoAllocFunctionInfo>,
}

#[derive(Template)]
#[template(path = "scaffold/cargo_toml.txt")]
struct CargoTomlTemplate<'a> {
    contract_name: &'a str,
    bin_source: &'a str,
    use_alloc: bool,
}

#[derive(Template)]
#[template(path = "scaffold/contract_blank.rs.txt")]
struct ContractBlankTemplate;

struct AllocFunctionInfo {
    name: String,
    name_snake: String,
    call_type: String,
}

struct SelectorConst {
    const_name: String,
    bytes_hex: String,
    signature: String,
}

struct EventConst {
    const_name: String,
    bytes_hex: String,
    signature: String,
}

struct ErrorConst {
    const_name: String,
    bytes_hex: String,
    signature: String,
}

struct NoAllocFunctionInfo {
    name: String,
    selector_const: String,
    min_call_data_len: usize,
    params: Vec<ParamDecode>,
}

struct ParamDecode {
    decode_line: String,
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
        #[allow(dead_code)]
        outputs: Vec<AbiOutput>,
        #[serde(rename = "stateMutability")]
        #[allow(dead_code)]
        state_mutability: String,
    },
    #[serde(rename = "event")]
    Event { name: String, inputs: Vec<AbiInput> },
    #[serde(rename = "error")]
    Error { name: String, inputs: Vec<AbiInput> },
    #[serde(rename = "constructor")]
    Constructor {
        #[allow(dead_code)]
        inputs: Vec<AbiInput>,
    },
}

#[derive(Debug, Deserialize, Clone)]
struct AbiInput {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
    #[allow(dead_code)]
    indexed: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
struct AbiOutput {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
}

/// Compute the keccak256 hash of a string
fn keccak256(input: &str) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(input.as_bytes());
    hasher.finalize(&mut output);
    output
}

/// Compute the 4-byte function selector from a function signature
fn compute_selector(signature: &str) -> [u8; 4] {
    let hash = keccak256(signature);
    [hash[0], hash[1], hash[2], hash[3]]
}

/// Build a function signature from name and input types
fn build_function_signature(name: &str, inputs: &[AbiInput]) -> String {
    let types: Vec<&str> = inputs.iter().map(|i| i.type_name.as_str()).collect();
    format!("{}({})", name, types.join(","))
}

/// Format a byte array as Rust hex literal
fn format_bytes_as_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("0x{:02x}", b))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format a 32-byte array with line breaks for readability
fn format_bytes32_multiline(bytes: &[u8; 32]) -> String {
    bytes
        .chunks(8)
        .map(|chunk| {
            chunk
                .iter()
                .map(|b| format!("0x{:02x}", b))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect::<Vec<_>>()
        .join(",\n    ")
}

/// Create a new blank contract project.
pub fn init_blank_contract(contract_name: &str) -> Result<()> {
    let contract_name = contract_name.to_case(Case::Kebab);
    let target_dir = std::env::current_dir()?.join(&contract_name);
    if target_dir.exists() {
        anyhow::bail!("Directory already exists: {target_dir:?}");
    }

    fs::create_dir(&target_dir)
        .with_context(|| format!("Failed to create directory: {target_dir:?}"))?;

    let (target_json_path, target_json_name) = resolve_target_json()?;
    let target_json_dest = target_dir.join(target_json_name);
    fs::copy(&target_json_path, &target_json_dest).with_context(|| {
        format!(
            "Failed to copy target JSON from {} to {}",
            target_json_path.display(),
            target_json_dest.display()
        )
    })?;

    let target_json_name = target_json_dest
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Target JSON path is missing a file name"))?;

    let cargo_config_dir = target_dir.join(".cargo");
    fs::create_dir(&cargo_config_dir)?;
    fs::write(
        cargo_config_dir.join("config.toml"),
        format!(
            "[build]\n target = \"{}\"\n\n[unstable]\n build-std = [\"core\", \"alloc\"]\n\n[env]\n RUSTC_BOOTSTRAP = \"1\"\n",
            target_json_name
        ),
    )?;

    fs::write(target_dir.join(".gitignore"), "/target\n*.polkavm\n")?;
    fs::write(
        target_dir.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"nightly\"\n",
    )?;
    fs::create_dir(target_dir.join("src"))?;
    let lib_rs_content = generate_blank_contract()?;
    fs::write(
        target_dir.join(format!("src/{}.rs", contract_name)),
        lib_rs_content,
    )?;

    let cargo_toml_content = generate_cargo_toml(&contract_name, &contract_name, false)?;
    fs::write(target_dir.join("Cargo.toml"), cargo_toml_content)?;

    println!("Successfully initialized blank contract project: {target_dir:?}");
    println!("\nNext steps:");
    println!("  cd {contract_name}");
    println!("  cargo pvm-contract build");
    Ok(())
}

/// Create a new contract project from a Solidity file.
pub fn init_from_solidity_file(sol_file: &str, contract_name: &str, use_alloc: bool) -> Result<()> {
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

    init_from_example_files_inner(&sol_content, &sol_file_name, None, contract_name, use_alloc)
}

pub fn init_from_example_files(
    sol_contents: &[u8],
    sol_file_name: &str,
    rust_contents: &[u8],
    contract_name: &str,
    use_alloc: bool,
) -> Result<()> {
    init_from_example_files_inner(
        sol_contents,
        sol_file_name,
        Some(rust_contents),
        contract_name,
        use_alloc,
    )
}

fn init_from_example_files_inner(
    sol_contents: &[u8],
    sol_file_name: &str,
    rust_contents: Option<&[u8]>,
    contract_name: &str,
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
    fs::copy(&target_json_path, &target_json_dest).with_context(|| {
        format!(
            "Failed to copy target JSON from {} to {}",
            target_json_path.display(),
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
            "[build]\n target = \"{}\"\n\n[unstable]\n build-std = [\"core\", \"alloc\"]\n\n[env]\n RUSTC_BOOTSTRAP = \"1\"\n",
            target_json_name
        ),
    )?;

    // Create .gitignore
    fs::write(target_dir.join(".gitignore"), "/target\n*.polkavm\n")?;
    fs::write(
        target_dir.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"nightly\"\n",
    )?;
    // Generate src/{contract}.rs
    fs::create_dir(target_dir.join("src"))?;

    let lib_rs_content = if let Some(contents) = rust_contents {
        String::from_utf8(contents.to_vec()).context("Example Rust file is not valid UTF-8")?
    } else if use_alloc {
        generate_rust_code_alloc(&sol_file_name, &metadata, &actual_contract_name)?
    } else {
        generate_rust_code_no_alloc(&metadata, &actual_contract_name)?
    };
    fs::write(
        target_dir.join(format!("src/{}.rs", actual_contract_kebab)),
        lib_rs_content,
    )?;

    // Create Cargo.toml
    let cargo_toml_content =
        generate_cargo_toml(&contract_name, &actual_contract_kebab, use_alloc)?;
    fs::write(target_dir.join("Cargo.toml"), cargo_toml_content)?;

    println!("Successfully initialized contract project from {sol_file_name}: {target_dir:?}");
    println!("\nNext steps:");
    println!("  cd {contract_name}");
    println!("  cargo pvm-contract build");
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

fn generate_blank_contract() -> Result<String> {
    ContractBlankTemplate
        .render()
        .context("Failed to render blank contract template")
}

fn generate_rust_code_alloc(
    sol_file_name: &str,
    metadata: &ContractMetadata,
    contract_name: &str,
) -> Result<String> {
    let contract_name_pascal = contract_name.to_case(Case::Pascal);

    let functions: Vec<AllocFunctionInfo> = metadata
        .output
        .abi
        .iter()
        .filter_map(|item| match item {
            AbiItem::Function { name, .. } => Some(AllocFunctionInfo {
                name: name.clone(),
                name_snake: name.to_case(Case::Snake),
                call_type: format!("{contract_name_pascal}::{name}Call"),
            }),
            _ => None,
        })
        .collect();

    let template = ContractAllocTemplate {
        sol_file_name,
        functions,
    };

    template.render().context("Failed to render alloc template")
}

fn generate_rust_code_no_alloc(metadata: &ContractMetadata, contract_name: &str) -> Result<String> {
    let contract_name_upper = contract_name.to_uppercase();

    // Collect function selectors
    let mut selectors = Vec::new();
    let mut functions = Vec::new();

    for item in &metadata.output.abi {
        if let AbiItem::Function { name, inputs, .. } = item {
            let signature = build_function_signature(name, inputs);
            let selector = compute_selector(&signature);
            let const_name = format!("{}_SELECTOR", name.to_case(Case::UpperSnake));

            selectors.push(SelectorConst {
                const_name: const_name.clone(),
                bytes_hex: format_bytes_as_hex(&selector),
                signature: signature.clone(),
            });

            // Generate decode params
            let mut params = Vec::new();

            for (idx, input) in inputs.iter().enumerate() {
                let param_name = if input.name.is_empty() {
                    format!("param_{}", idx)
                } else {
                    input.name.to_case(Case::Snake)
                };

                let decode_line =
                    format!("// TODO: decode {param_name} of type {}", input.type_name);

                params.push(ParamDecode { decode_line });
            }

            functions.push(NoAllocFunctionInfo {
                name: name.clone(),
                selector_const: const_name,
                min_call_data_len: 4 + inputs.len() * 32,
                params,
            });
        }
    }

    // Collect events
    let events: Vec<EventConst> = metadata
        .output
        .abi
        .iter()
        .filter_map(|item| {
            if let AbiItem::Event { name, inputs } = item {
                let signature = build_function_signature(name, inputs);
                let hash = keccak256(&signature);
                Some(EventConst {
                    const_name: format!("{}_EVENT_SIGNATURE", name.to_case(Case::UpperSnake)),
                    bytes_hex: format_bytes32_multiline(&hash),
                    signature,
                })
            } else {
                None
            }
        })
        .collect();

    // Collect errors
    let errors: Vec<ErrorConst> = metadata
        .output
        .abi
        .iter()
        .filter_map(|item| {
            if let AbiItem::Error { name, inputs } = item {
                let signature = build_function_signature(name, inputs);
                let selector = compute_selector(&signature);
                Some(ErrorConst {
                    const_name: format!("{}_ERROR", name.to_case(Case::UpperSnake)),
                    bytes_hex: format_bytes_as_hex(&selector),
                    signature,
                })
            } else {
                None
            }
        })
        .collect();

    let template = ContractNoAllocTemplate {
        contract_name_upper: &contract_name_upper,
        selectors,
        events,
        errors,
        functions,
    };

    template
        .render()
        .context("Failed to render no-alloc template")
}

fn resolve_target_json() -> Result<(PathBuf, String)> {
    let mut args = polkavm_linker::TargetJsonArgs::default();
    args.is_64_bit = true;
    let target_json = polkavm_linker::target_json_path(args)
        .map_err(|e| anyhow::anyhow!("Failed to get target JSON: {e}"))?;

    let target_name = target_json
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Target JSON path is missing a file name"))?
        .to_string();

    Ok((target_json, target_name))
}

fn generate_cargo_toml(contract_name: &str, bin_source: &str, use_alloc: bool) -> Result<String> {
    let template = CargoTomlTemplate {
        contract_name,
        bin_source,
        use_alloc,
    };
    template
        .render()
        .context("Failed to render Cargo.toml template")
}

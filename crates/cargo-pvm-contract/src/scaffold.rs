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

struct MacroFunctionInfo {
    name_snake: String,
    params: String,
    return_type: String,
    /// Rust receiver derived from the Solidity `stateMutability` field, set by
    /// `receiver_from_mutability`. Empty for `pure`, in which case the template
    /// omits the leading comma between receiver and params.
    receiver: String,
    /// `#[pvm_contract_sdk::payable]` attribute line if the function is
    /// payable; empty otherwise. Emitted on a line above `#[method]`.
    payable_attr: String,
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
        state_mutability: String,
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

    let cargo_toml_content =
        generate_cargo_toml(&contract_name, &contract_name, use_dsl, use_alloc)?;
    fs::write(target_dir.join("Cargo.toml"), cargo_toml_content)?;

    println!("Successfully initialized contract project: {target_dir:?}");
    println!("\nNext steps:");
    println!("  cd {contract_name}");
    println!("  cargo pvm-contract build");
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

    // Reject at scaffold time when the DSL scaffolder would emit Rust that
    // won't compile. The DSL template's generated decoder requires every
    // parameter and return type to implement `StaticEncodedLen`, which only
    // static types do. Two shapes always break:
    //   - Any function with a dynamic return type.
    //   - Multi-parameter functions where any parameter is dynamic.
    // Single-param dynamic input with a non-dynamic return happens to compile
    // (the `StaticEncodedLen` reference is skipped at the first parameter).
    // Skipped on the `--example` path, which uses pre-written Rust instead of
    // generating it from the ABI.
    if use_dsl && rust_contents.is_none() {
        for item in &metadata.output.abi {
            if let AbiItem::Function {
                name,
                inputs,
                outputs,
                ..
            } = item
            {
                let dynamic_return = outputs.iter().any(|o| is_dynamic_sol_type(&o.type_name));
                let multi_param_with_dynamic =
                    inputs.len() > 1 && inputs.iter().any(|p| is_dynamic_sol_type(&p.type_name));
                if dynamic_return {
                    anyhow::bail!(
                        "DSL scaffolding does not support `{name}`: dynamic return types \
                         (`bytes`, `string`, `T[]`) require an offset/length encoding that the \
                         DSL template does not yet emit. Re-run with `--api-style macro`."
                    );
                }
                if multi_param_with_dynamic {
                    anyhow::bail!(
                        "DSL scaffolding does not support `{name}`: multi-parameter signatures \
                         containing dynamic types are not supported (the DSL template's offset \
                         accumulator requires `StaticEncodedLen`, which dynamic types don't \
                         implement). Re-run with `--api-style macro`."
                    );
                }
            }
        }
    }

    // Reject `--allocator no-alloc` paired with a `.sol` containing dynamic
    // types. `bytes`/`string`/`T[]` map to `Bytes`/`String`/`Vec`, which need
    // `alloc` — unreachable in a no-alloc contract. Skipped on the `--example`
    // path, which uses pre-written Rust instead of generating it from the ABI.
    if !use_alloc && rust_contents.is_none() {
        let uses_dynamic = metadata.output.abi.iter().any(|item| {
            if let AbiItem::Function {
                inputs, outputs, ..
            } = item
            {
                inputs.iter().any(|p| is_dynamic_sol_type(&p.type_name))
                    || outputs.iter().any(|o| is_dynamic_sol_type(&o.type_name))
            } else {
                false
            }
        });
        if uses_dynamic {
            anyhow::bail!(
                "The Solidity interface uses dynamic types (`bytes`, `string`, `T[]`, \
                 or a fixed array containing one of those) which require an allocator. \
                 Re-run with `--allocator bump`."
            );
        }
    }

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
        let functions = extract_dsl_function_info(&metadata)?;
        generate_dsl_contract(use_alloc, functions)?
    } else {
        let functions = extract_function_info(&metadata)?;
        generate_macro_contract(use_alloc, Some(&sol_file_name), functions)?
    };
    fs::write(
        target_dir.join(format!("src/{actual_contract_kebab}.rs")),
        lib_rs_content,
    )?;

    let cargo_toml_content =
        generate_cargo_toml(&contract_name, &actual_contract_kebab, use_dsl, use_alloc)?;
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

fn extract_function_info(metadata: &ContractMetadata) -> Result<Vec<MacroFunctionInfo>> {
    metadata
        .output
        .abi
        .iter()
        .filter_map(|item| match item {
            AbiItem::Function {
                name,
                inputs,
                outputs,
                state_mutability,
            } => Some((name, inputs, outputs, state_mutability)),
            _ => None,
        })
        .map(
            |(name, inputs, outputs, state_mutability)| -> Result<MacroFunctionInfo> {
                let name_snake = name.to_case(Case::Snake);
                let params = inputs
                    .iter()
                    .enumerate()
                    .map(|(i, p)| -> Result<String> {
                        let param_name = if p.name.is_empty() {
                            format!("arg{i}")
                        } else {
                            p.name.to_case(Case::Snake)
                        };
                        Ok(format!(
                            "{param_name}: {}",
                            solidity_to_rust_type(&p.type_name)?
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .join(", ");
                // Scaffolded bodies are `todo!()` so the error variant only
                // needs to be in scope; matches the constructor's choice in
                // the template. Users replace `EmptyError` with their own
                // error type when they fill in real bodies.
                let return_type = if outputs.is_empty() {
                    "Result<(), pvm_contract_sdk::EmptyError>".to_string()
                } else if outputs.len() == 1 {
                    format!(
                        "Result<{}, pvm_contract_sdk::EmptyError>",
                        solidity_to_rust_type(&outputs[0].type_name)?
                    )
                } else {
                    let types = outputs
                        .iter()
                        .map(|o| solidity_to_rust_type(&o.type_name))
                        .collect::<Result<Vec<_>>>()?
                        .join(", ");
                    format!("Result<({types}), pvm_contract_sdk::EmptyError>")
                };
                let (receiver, payable_attr) = receiver_from_mutability(state_mutability)?;
                Ok(MacroFunctionInfo {
                    name_snake,
                    params,
                    return_type,
                    receiver,
                    payable_attr,
                })
            },
        )
        .collect()
}

/// Map a Solidity `stateMutability` string to the Rust receiver and (optional)
/// `#[payable]` attribute the SDK macro expects. Mirrors the inference table
/// documented in CLAUDE.md ("Mutability Inference"):
///
/// - `pure` -> no receiver. The SDK macro infers `pure` from the absence of a
///   `self` argument; emitting `&self` would be inferred as `view`, mismatching
///   the `.sol` declaration.
/// - `view` -> `&self`.
/// - `nonpayable` -> `&mut self`.
/// - `payable` -> `&mut self` + `#[pvm_contract_sdk::payable]`.
fn receiver_from_mutability(sm: &str) -> Result<(String, String)> {
    Ok(match sm {
        "pure" => (String::new(), String::new()),
        "view" => ("&self".to_string(), String::new()),
        "nonpayable" => ("&mut self".to_string(), String::new()),
        "payable" => (
            "&mut self".to_string(),
            "#[pvm_contract_sdk::payable]\n        ".to_string(),
        ),
        other => anyhow::bail!("unrecognised Solidity stateMutability: {other:?}"),
    })
}

fn extract_dsl_function_info(metadata: &ContractMetadata) -> Result<Vec<DslFunctionInfo>> {
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
            } => Some((name, inputs, outputs)),
            _ => None,
        })
        .map(|(name, inputs, outputs)| -> Result<DslFunctionInfo> {
            let name_snake = name.to_case(Case::Snake);
            let screaming = name_snake.to_case(Case::ScreamingSnake);
            let selector_const = format!("{screaming}_SELECTOR");

            // Build Solidity signature like "transfer(address,uint256)"
            let sol_param_types: Vec<&str> = inputs.iter().map(|p| p.type_name.as_str()).collect();
            let sol_params = sol_param_types.join(",");
            let solidity_signature = format!("{name}({sol_params})");

            // Build decode expressions for each parameter. `offset_expr`
            // accumulates the cumulative offset expression across iterations
            // (e.g. `<T0 as StaticEncodedLen>::ENCODED_SIZE + <T1 as ...>`),
            // so the loop carries state and must stay imperative — keep this
            // out of `.fold(...)`.
            let mut offset_expr = String::new();
            let params: Vec<DslParam> = inputs
                .iter()
                .enumerate()
                .map(|(i, p)| -> Result<DslParam> {
                    let param_name = if p.name.is_empty() {
                        format!("arg{i}")
                    } else {
                        p.name.to_case(Case::Snake)
                    };
                    let rust_type = solidity_to_rust_type(&p.type_name)?;
                    // Angle-bracket the type so compound shapes like
                    // `[U256; 3]` / `Vec<U256>` parse as qualified paths;
                    // the bare form would be a Rust syntax error.
                    let decode_expr = if i == 0 {
                        format!("<{rust_type}>::decode_at(input, 0)")
                    } else {
                        format!("<{rust_type}>::decode_at(input, {offset_expr})")
                    };

                    // Accumulate offset for next parameter.
                    let size_expr = format!("<{rust_type} as StaticEncodedLen>::ENCODED_SIZE");
                    if i == 0 {
                        offset_expr = size_expr;
                    } else {
                        offset_expr = format!("{offset_expr} + {size_expr}");
                    }

                    Ok(DslParam {
                        name: param_name,
                        decode_expr,
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            let return_rust_type = if outputs.is_empty() {
                "()".to_string()
            } else if outputs.len() == 1 {
                solidity_to_rust_type(&outputs[0].type_name)?
            } else {
                let types = outputs
                    .iter()
                    .map(|o| solidity_to_rust_type(&o.type_name))
                    .collect::<Result<Vec<_>>>()?
                    .join(", ");
                format!("({types})")
            };

            Ok(DslFunctionInfo {
                selector_const,
                solidity_signature,
                name_snake,
                params,
                return_rust_type,
            })
        })
        .collect()
}

/// Map a Solidity ABI type string (as emitted by solc) to a Rust SDK type.
///
/// Unrecognized or unsupported types return `Err` rather than silently mapping
/// to `U256`. The returned type name is unqualified and is inserted directly
/// into the scaffolded source, so the templates must `use` the names this
/// function emits (`Address`, `Bytes`, `String`, `Vec`, `I256`).
fn solidity_to_rust_type(sol_type: &str) -> Result<String> {
    // 1. Dynamic array T[]. Recurse on the element type.
    if let Some(inner) = sol_type.strip_suffix("[]") {
        let inner_type = solidity_to_rust_type(inner)?;
        return Ok(format!("Vec<{inner_type}>"));
    }

    // 2. Fixed array T[N]. Must come before `uintN`/`intN`/`bytesN` so
    //    `uint256[2]` parses as an array, not as a width.
    if let Some(bracket_pos) = sol_type.rfind('[')
        && let Some(n_str) = sol_type[bracket_pos + 1..].strip_suffix(']')
        && let Ok(n) = n_str.parse::<usize>()
    {
        let inner = &sol_type[..bracket_pos];
        let inner_type = solidity_to_rust_type(inner)?;
        return Ok(format!("[{inner_type}; {n}]"));
    }

    // 3. Named primitives.
    match sol_type {
        "address" => return Ok("Address".to_string()),
        "bool" => return Ok("bool".to_string()),
        "string" => return Ok("String".to_string()),
        "bytes" => return Ok("Bytes".to_string()),
        _ => {}
    }

    // 4. uintN — only canonical widths.
    if let Some(n_str) = sol_type.strip_prefix("uint") {
        if n_str.is_empty() {
            return Ok("U256".to_string()); // Solidity `uint` aliases `uint256`.
        }
        let bits: u32 = n_str
            .parse()
            .map_err(|_| anyhow::anyhow!("unsupported Solidity type: {sol_type:?}"))?;
        return Ok(match bits {
            8 => "u8",
            16 => "u16",
            32 => "u32",
            64 => "u64",
            128 => "u128",
            256 => "U256",
            _ => anyhow::bail!(
                "unsupported uintN width: {sol_type:?} \
                 (only 8, 16, 32, 64, 128, 256 are scaffolded)"
            ),
        }
        .to_string());
    }

    // 5. intN — analogous to uintN.
    if let Some(n_str) = sol_type.strip_prefix("int") {
        if n_str.is_empty() {
            return Ok("I256".to_string()); // Solidity `int` aliases `int256`.
        }
        let bits: u32 = n_str
            .parse()
            .map_err(|_| anyhow::anyhow!("unsupported Solidity type: {sol_type:?}"))?;
        return Ok(match bits {
            8 => "i8",
            16 => "i16",
            32 => "i32",
            64 => "i64",
            128 => "i128",
            256 => "I256",
            _ => anyhow::bail!(
                "unsupported intN width: {sol_type:?} \
                 (only 8, 16, 32, 64, 128, 256 are scaffolded)"
            ),
        }
        .to_string());
    }

    // 6. bytesN — widths 1..=32.
    if let Some(n_str) = sol_type.strip_prefix("bytes") {
        let n: usize = n_str
            .parse()
            .map_err(|_| anyhow::anyhow!("unsupported Solidity type: {sol_type:?}"))?;
        if !(1..=32).contains(&n) {
            anyhow::bail!("invalid bytesN width: {sol_type:?} (must be 1..=32)");
        }
        return Ok(format!("[u8; {n}]"));
    }

    // 7. Tuple — AbiInput/AbiOutput drop the `components` field, so the
    //    sub-structure is gone by the time we see `"tuple"` here.
    //    Tuple-decoder codegen will be added later; reject for now.
    if sol_type == "tuple" {
        anyhow::bail!(
            "tuple types are not yet supported by the scaffolder. \
             Please edit the generated file manually or use a non-tuple parameter shape."
        );
    }

    // 8. Reject unknown shapes. Never interpolate `sol_type` into a returned
    //    type string; silently fabricating types produces wrong decoders that
    //    revert at the dispatch boundary.
    anyhow::bail!("unsupported Solidity type: {sol_type:?}")
}

/// Recursively check whether a Solidity type, when mapped to its Rust SDK type,
/// uses `Bytes` / `String` / `Vec<T>` and therefore needs the `alloc` feature.
/// Mirrors the parse structure of `solidity_to_rust_type`.
fn is_dynamic_sol_type(t: &str) -> bool {
    // Dynamic array T[] — always Vec<...>.
    if t.ends_with("[]") {
        return true;
    }
    // Fixed array T[N] — dynamic iff inner is dynamic (`bytes[5]` -> `[Bytes; 5]`).
    if let Some(bracket_pos) = t.rfind('[')
        && let Some(n_str) = t[bracket_pos + 1..].strip_suffix(']')
        && n_str.parse::<usize>().is_ok()
    {
        return is_dynamic_sol_type(&t[..bracket_pos]);
    }
    // Bare `bytes` / `string`. Sized variants like `bytes32` map to `[u8; N]`
    // which is static, so they fall through this match and return false.
    matches!(t, "bytes" | "string")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn map(s: &str) -> String {
        solidity_to_rust_type(s).expect("expected Ok")
    }
    fn err(s: &str) {
        assert!(
            solidity_to_rust_type(s).is_err(),
            "expected Err for {s:?}, got {:?}",
            solidity_to_rust_type(s)
        );
    }

    #[test]
    fn primitives() {
        assert_eq!(map("address"), "Address");
        assert_eq!(map("bool"), "bool");
        assert_eq!(map("string"), "String");
        assert_eq!(map("bytes"), "Bytes");
    }

    #[test]
    fn uint_widths() {
        for (sol, rust) in [
            ("uint8", "u8"),
            ("uint16", "u16"),
            ("uint32", "u32"),
            ("uint64", "u64"),
            ("uint128", "u128"),
            ("uint256", "U256"),
        ] {
            assert_eq!(map(sol), rust);
        }
        assert_eq!(map("uint"), "U256"); // Solidity alias for uint256
        for invalid in ["uint24", "uint40", "uint512"] {
            err(invalid);
        }
    }

    #[test]
    fn int_widths() {
        for (sol, rust) in [
            ("int8", "i8"),
            ("int16", "i16"),
            ("int32", "i32"),
            ("int64", "i64"),
            ("int128", "i128"),
            ("int256", "I256"),
        ] {
            assert_eq!(map(sol), rust);
        }
        assert_eq!(map("int"), "I256");
        for invalid in ["int24", "int40", "int512"] {
            err(invalid);
        }
    }

    #[test]
    fn bytes_n() {
        for (sol, rust) in [
            ("bytes1", "[u8; 1]"),
            ("bytes20", "[u8; 20]"),
            ("bytes32", "[u8; 32]"),
        ] {
            assert_eq!(map(sol), rust);
        }
        for invalid in ["bytes0", "bytes33", "bytes100"] {
            err(invalid);
        }
    }

    #[test]
    fn dynamic_arrays() {
        assert_eq!(map("uint256[]"), "Vec<U256>");
        assert_eq!(map("bytes[]"), "Vec<Bytes>");
        assert_eq!(map("string[]"), "Vec<String>");
        assert_eq!(map("address[]"), "Vec<Address>");
        assert_eq!(map("uint256[][]"), "Vec<Vec<U256>>");
    }

    #[test]
    fn fixed_arrays() {
        assert_eq!(map("uint256[2]"), "[U256; 2]");
        assert_eq!(map("address[5]"), "[Address; 5]");
        assert_eq!(map("bool[3]"), "[bool; 3]");
        // Non-numeric and malformed sizes are rejected.
        err("uint256[N]");
        err("uint256[]extra");
    }

    #[test]
    fn nested_array_kinds() {
        assert_eq!(map("uint256[][3]"), "[Vec<U256>; 3]");
        assert_eq!(map("uint256[2][]"), "Vec<[U256; 2]>");
    }

    #[test]
    fn unknown_type_rejected() {
        err("mapping(address => uint256)");
        err("unknown_t");
        err("function");
    }

    #[test]
    fn tuple_rejected_with_clear_message() {
        // Special-cased because we also check the message content — users
        // need a "tuple" mention to know how to work around the limitation.
        let e = solidity_to_rust_type("tuple").unwrap_err();
        assert!(
            e.to_string().contains("tuple"),
            "expected tuple-mentioning error, got {e}"
        );
    }

    #[test]
    fn malformed_numeric_suffix_rejected() {
        // Catches the old `unwrap_or` silent-fallback path across all three
        // numeric-suffix arms (uintN / intN / bytesN).
        err("uintXY");
        err("bytesXY");
        err("intABC");
    }

    #[test]
    fn is_dynamic_sol_type_classification() {
        for dynamic in [
            "bytes",
            "string",
            "uint256[]",
            "address[]",
            // Fixed array of dynamic still needs alloc.
            "bytes[5]",
            "string[3]",
        ] {
            assert!(
                is_dynamic_sol_type(dynamic),
                "expected {dynamic:?} to be dynamic"
            );
        }
        for static_t in [
            "uint256",
            "address",
            "bool",
            "bytes32",
            "int128",
            // Fixed array of static stays static.
            "uint256[2]",
            "address[10]",
        ] {
            assert!(
                !is_dynamic_sol_type(static_t),
                "expected {static_t:?} to be static"
            );
        }
    }
}

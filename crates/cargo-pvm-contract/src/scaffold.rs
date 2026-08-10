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
    /// `#[derive(SolType)]` structs generated for the Solidity `struct` (ABI
    /// `tuple`) types referenced by the interface. Emitted inside the contract
    /// module, above the storage struct.
    structs: Vec<GeneratedStruct>,
}

/// A `#[derive(SolType)]` struct the scaffolder emits for a Solidity `struct`
/// (ABI `tuple`) parameter or return.
struct GeneratedStruct {
    name: String,
    fields: Vec<GeneratedField>,
}

struct GeneratedField {
    name: String,
    /// The field's Solidity name, kept only so a snake_case collision can name
    /// both offending members in the error.
    sol_name: String,
    rust_type: String,
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
    #[serde(default)]
    name: String,
    #[serde(rename = "type")]
    type_name: String,
    /// solc's fully-qualified name for the type, e.g. `struct IFoo.Point`.
    /// Present only for tuple (struct) parameters; used to name the generated
    /// Rust struct.
    #[serde(rename = "internalType", default)]
    internal_type: Option<String>,
    /// Field descriptors for a tuple (Solidity `struct`) parameter. `None` for
    /// every non-tuple type.
    #[serde(default)]
    components: Option<Vec<AbiInput>>,
    #[serde(rename = "indexed")]
    _indexed: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
struct AbiOutput {
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "type")]
    type_name: String,
    #[serde(rename = "internalType", default)]
    internal_type: Option<String>,
    #[serde(default)]
    components: Option<Vec<AbiInput>>,
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
        generate_macro_contract(use_alloc, None, vec![], vec![])?
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

    // solc resolves imports for the ABI, but only this entry file is copied into
    // the project, so the build-time macro re-parse would see an unresolved
    // import and hash a wrong selector. Fail here with the same message the
    // build-time parsers give.
    if let Ok(source) = std::str::from_utf8(sol_contents) {
        cargo_pvm_contract_builder::reject_sol_imports(source)?;
    }

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
                inputs
                    .iter()
                    .any(|p| param_is_dynamic(&p.type_name, p.components.as_deref()))
                    || outputs
                        .iter()
                        .any(|o| param_is_dynamic(&o.type_name, o.components.as_deref()))
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
        let (functions, structs) = extract_function_info(&metadata)?;
        generate_macro_contract(use_alloc, Some(&sol_file_name), functions, structs)?
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
    structs: Vec<GeneratedStruct>,
) -> Result<String> {
    ContractMacroTemplate {
        use_alloc,
        sol_file_name,
        functions,
        structs,
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

fn extract_function_info(
    metadata: &ContractMetadata,
) -> Result<(Vec<MacroFunctionInfo>, Vec<GeneratedStruct>)> {
    let mut registry = StructRegistry::default();
    let mut functions = Vec::new();

    for item in &metadata.output.abi {
        let AbiItem::Function {
            name,
            inputs,
            outputs,
            state_mutability,
        } = item
        else {
            continue;
        };

        let name_snake = sanitize_rust_ident(&name.to_case(Case::Snake));
        let mut param_strs = Vec::with_capacity(inputs.len());
        for (i, p) in inputs.iter().enumerate() {
            let param_name = if p.name.is_empty() {
                format!("arg{i}")
            } else {
                // A Solidity parameter may be named after a Rust keyword
                // (`ref`, `gen`, `move`), which would otherwise emit a
                // signature that doesn't parse.
                sanitize_rust_ident(&p.name.to_case(Case::Snake))
            };
            let rust_type = abi_param_rust_type(
                &p.type_name,
                p.internal_type.as_deref(),
                p.components.as_deref(),
                &mut registry,
            )
            .with_context(|| format!("in parameter `{param_name}` of `{name}`"))?;
            param_strs.push(format!("{param_name}: {rust_type}"));
        }
        let params = param_strs.join(", ");

        // Scaffolded bodies are `todo!()` so the error variant only needs to be
        // in scope; matches the constructor's choice in the template. Users
        // replace `EmptyError` with their own error type when they fill in real
        // bodies.
        let return_type = if outputs.is_empty() {
            "Result<(), pvm_contract_sdk::EmptyError>".to_string()
        } else if outputs.len() == 1 {
            let ret = abi_param_rust_type(
                &outputs[0].type_name,
                outputs[0].internal_type.as_deref(),
                outputs[0].components.as_deref(),
                &mut registry,
            )
            .with_context(|| format!("in return type of `{name}`"))?;
            format!("Result<{ret}, pvm_contract_sdk::EmptyError>")
        } else {
            let mut types = Vec::with_capacity(outputs.len());
            for o in outputs {
                types.push(
                    abi_param_rust_type(
                        &o.type_name,
                        o.internal_type.as_deref(),
                        o.components.as_deref(),
                        &mut registry,
                    )
                    .with_context(|| format!("in return type of `{name}`"))?,
                );
            }
            format!(
                "Result<({}), pvm_contract_sdk::EmptyError>",
                types.join(", ")
            )
        };
        let (receiver, payable_attr) = receiver_from_mutability(state_mutability)?;
        functions.push(MacroFunctionInfo {
            name_snake,
            params,
            return_type,
            receiver,
            payable_attr,
        });
    }

    Ok((functions, registry.order))
}

/// Accumulates `#[derive(SolType)]` struct definitions discovered while mapping
/// Solidity `tuple` (struct) parameters to Rust types.
#[derive(Default)]
struct StructRegistry {
    /// Canonical Solidity path (e.g. `IFoo.Point`) -> generated Rust name.
    /// Doubles as the recursion guard for self-referential structs.
    by_path: std::collections::HashMap<String, String>,
    /// Generated Rust name -> the Solidity path that owns it, to detect two
    /// distinct structs colliding on the same simple name.
    name_owner: std::collections::HashMap<String, String>,
    /// Emission order; a nested struct is pushed before the struct that
    /// contains it.
    order: Vec<GeneratedStruct>,
}

/// Map an ABI parameter (which may be a `tuple`/struct, possibly wrapped in
/// array suffixes) to a Rust type, registering any `#[derive(SolType)]` structs
/// it needs into `reg`. Non-tuple types fall through to
/// [`solidity_to_rust_type`].
fn abi_param_rust_type(
    type_name: &str,
    internal_type: Option<&str>,
    components: Option<&[AbiInput]>,
    reg: &mut StructRegistry,
) -> Result<String> {
    // solc emits `components` iff the type is (an array of) a tuple.
    match components {
        Some(comps) => tuple_rust_type(type_name, internal_type, comps, reg),
        None => solidity_to_rust_type(type_name),
    }
}

/// Handle a tuple-based ABI type: `tuple`, `tuple[]`, `tuple[N]`, and nested
/// combinations. `components` always describes the base tuple's fields;
/// `internal_type` carries the struct's Solidity name (with any array suffix).
fn tuple_rust_type(
    type_name: &str,
    internal_type: Option<&str>,
    components: &[AbiInput],
    reg: &mut StructRegistry,
) -> Result<String> {
    if let Some(inner) = type_name.strip_suffix("[]") {
        let elem = tuple_rust_type(inner, internal_type, components, reg)?;
        return Ok(format!("Vec<{elem}>"));
    }
    if let Some((inner, n)) = split_fixed_array(type_name) {
        let elem = tuple_rust_type(inner, internal_type, components, reg)?;
        return Ok(format!("[{elem}; {n}]"));
    }
    if type_name != "tuple" {
        anyhow::bail!("unsupported tuple type: {type_name:?}");
    }
    let (path, rust_name) = struct_names_from_internal_type(internal_type)?;
    register_struct(&path, &rust_name, components, reg)?;
    Ok(rust_name)
}

/// Split a fixed-array suffix `T[N]` into `(T, N)`. Mirrors the parse in
/// [`solidity_to_rust_type`].
fn split_fixed_array(t: &str) -> Option<(&str, usize)> {
    let bracket = t.rfind('[')?;
    let n: usize = t[bracket + 1..].strip_suffix(']')?.parse().ok()?;
    Some((&t[..bracket], n))
}

/// Derive `(solidity_path, rust_name)` from a tuple's `internalType`, e.g.
/// `struct IFoo.Point[]` -> (`IFoo.Point`, `Point`). The path uniquely
/// identifies the struct; the Rust name is its final `.`-separated segment.
fn struct_names_from_internal_type(internal_type: Option<&str>) -> Result<(String, String)> {
    let it = internal_type.ok_or_else(|| {
        anyhow::anyhow!(
            "tuple parameter has no `internalType`, so the scaffolder cannot derive a \
             struct name for it. Edit the generated file manually."
        )
    })?;
    // Strip solc's `struct ` prefix and any trailing array groups
    // (`Point[][3]` -> `Point`).
    let mut base = it.strip_prefix("struct ").unwrap_or(it).trim();
    while let Some(b) = base.rfind('[') {
        if base[b..].ends_with(']') {
            base = base[..b].trim_end();
        } else {
            break;
        }
    }
    let segment = base
        .rsplit('.')
        .next()
        .filter(|s| is_valid_ident(s))
        .ok_or_else(|| {
            anyhow::anyhow!("could not derive a valid Rust struct name from internalType {it:?}")
        })?;
    Ok((base.to_string(), sanitize_rust_ident(segment)))
}

fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Identifiers the generated module already binds, which a generated struct
/// must not reuse. `Contract` is the contract struct in the template; the rest
/// come from `use pvm_contract_sdk::prelude::*` and the alloc-mode imports
/// (`Bytes`, `String`, `Vec`).
///
/// A collision here is not merely a name clash: an explicit item silently
/// *shadows* a glob import, so a `.sol` declaring `struct Address` would make
/// every `address` parameter decode as that struct — the project compiles and
/// the method reverts (or mis-decodes) on-chain. Reject at scaffold time
/// instead, per this module's "never emit code we know is wrong" rule.
const RESERVED_STRUCT_NAMES: [&str; 18] = [
    // Declared by the template.
    "Contract",
    // `pvm_contract_sdk::prelude::*`.
    "Address",
    "DecodeError",
    "EmptyError",
    "Host",
    "HostApi",
    "I256",
    "PolkaVmHost",
    "ReturnFlags",
    "SolDecode",
    "SolEncode",
    "SolError",
    "StaticEncodedLen",
    "StorageFlags",
    "U256",
    // Alloc-mode imports.
    "Bytes",
    "String",
    "Vec",
];

/// Make a Solidity identifier safe to emit as a Rust identifier. A Solidity
/// `struct`/field name can be a Rust keyword (e.g. a field named `ref`), which
/// would otherwise produce non-compiling code. Raw-identify where allowed;
/// suffix `_` for the few keywords that cannot be raw idents.
fn sanitize_rust_ident(name: &str) -> String {
    // Keywords that `r#` cannot escape.
    const NON_RAW: [&str; 4] = ["crate", "self", "super", "Self"];
    if NON_RAW.contains(&name) {
        return format!("{name}_");
    }
    if is_rust_keyword(name) {
        return format!("r#{name}");
    }
    name.to_string()
}

fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        // Strict keywords.
        "as" | "break" | "const" | "continue" | "crate" | "else" | "enum" | "extern" | "false"
            | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop" | "match" | "mod" | "move"
            | "mut" | "pub" | "ref" | "return" | "self" | "Self" | "static" | "struct" | "super"
            | "trait" | "true" | "type" | "unsafe" | "use" | "where" | "while"
            // Edition-2018+ strict keywords (`gen` reserved in 2024).
            | "async" | "await" | "dyn" | "gen"
            // Reserved for future use.
            | "abstract" | "become" | "box" | "do" | "final" | "macro" | "override" | "priv"
            | "typeof" | "unsized" | "virtual" | "yield" | "try"
    )
}

/// Register a struct (and, recursively, its tuple-typed fields) into `reg`.
fn register_struct(
    path: &str,
    rust_name: &str,
    components: &[AbiInput],
    reg: &mut StructRegistry,
) -> Result<()> {
    // Already registered, or currently registering (self-referential struct).
    if reg.by_path.contains_key(path) {
        return Ok(());
    }
    if RESERVED_STRUCT_NAMES.contains(&rust_name) {
        anyhow::bail!(
            "the Solidity struct `{path}` maps to the Rust type `{rust_name}`, which the \
             generated contract module already binds (the contract struct or a \
             `pvm_contract_sdk` prelude import). A struct by that name would shadow it and \
             silently change how other parameters decode. Rename it in the interface, or \
             edit the generated file manually."
        );
    }
    if let Some(other) = reg.name_owner.get(rust_name) {
        anyhow::bail!(
            "two Solidity structs (`{other}` and `{path}`) both map to the Rust type \
             `{rust_name}`; rename one in the interface, or edit the generated file manually"
        );
    }
    // Mark as seen before recursing so a self-referential struct terminates.
    reg.by_path.insert(path.to_string(), rust_name.to_string());
    reg.name_owner
        .insert(rust_name.to_string(), path.to_string());

    let mut fields = Vec::with_capacity(components.len());
    for (i, c) in components.iter().enumerate() {
        let field_name = if c.name.is_empty() {
            format!("field{i}")
        } else {
            sanitize_rust_ident(&c.name.to_case(Case::Snake))
        };
        let rust_type = abi_param_rust_type(
            &c.type_name,
            c.internal_type.as_deref(),
            c.components.as_deref(),
            reg,
        )
        .with_context(|| format!("in field `{field_name}` of struct `{rust_name}`"))?;
        // Snake-casing is lossy: `myField` and `my_field` are distinct Solidity
        // members that collapse to the same Rust ident, which would emit a
        // struct with a duplicate field (`E0124`) only discovered when the user
        // builds. Reject here instead.
        if let Some(prev) = fields
            .iter()
            .find(|f: &&GeneratedField| f.name == field_name)
        {
            anyhow::bail!(
                "fields `{}` and `{}` of Solidity struct `{rust_name}` both map to the Rust \
                 field name `{field_name}`; rename one in the interface, or edit the \
                 generated file manually",
                prev.sol_name,
                c.name,
            );
        }
        fields.push(GeneratedField {
            name: field_name,
            sol_name: c.name.clone(),
            rust_type,
        });
    }
    reg.order.push(GeneratedStruct {
        name: rust_name.to_string(),
        fields,
    });
    Ok(())
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
            let name_snake = sanitize_rust_ident(&name.to_case(Case::Snake));
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
                        sanitize_rust_ident(&p.name.to_case(Case::Snake))
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

    // 7. Tuple — the macro scaffolder maps these through `abi_param_rust_type`
    //    (which has the `components` sub-structure) into generated
    //    `#[derive(SolType)]` structs. Reaching this arm means a tuple hit a
    //    path without component info (the DSL scaffolder, which cannot emit the
    //    SolType derive it would need).
    if sol_type == "tuple" {
        anyhow::bail!(
            "tuple (struct) types are not supported by the DSL scaffolder. \
             Re-run with `--api-style macro`, which generates a `#[derive(SolType)]` \
             struct for each Solidity struct."
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

/// Component-aware dynamic-type check. A tuple is dynamic if it is a dynamic
/// array (`tuple[]`) or if any of its fields is dynamic; otherwise this defers
/// to [`is_dynamic_sol_type`] on the type name.
fn param_is_dynamic(type_name: &str, components: Option<&[AbiInput]>) -> bool {
    let Some(components) = components else {
        return is_dynamic_sol_type(type_name);
    };
    if type_name.ends_with("[]") {
        return true;
    }
    // Fixed array `tuple[N]`: dynamic iff the element is. Recurse on the inner
    // type name (keeping the same `components`, which describe the base tuple)
    // before falling through to the field check.
    if let Some((inner, _)) = split_fixed_array(type_name) {
        return param_is_dynamic(inner, Some(components));
    }
    components
        .iter()
        .any(|c| param_is_dynamic(&c.type_name, c.components.as_deref()))
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
    fn tuple_rejected_by_bare_string_mapper() {
        // `solidity_to_rust_type` has no component info, so it can only reject
        // tuples (the DSL scaffolder path). The macro path routes tuples through
        // `abi_param_rust_type` instead. Message must point at `--api-style macro`.
        let e = solidity_to_rust_type("tuple").unwrap_err();
        let msg = e.to_string();
        assert!(
            msg.contains("tuple"),
            "expected tuple-mentioning error, got {e}"
        );
        assert!(msg.contains("macro"), "expected macro hint, got {e}");
    }

    fn input(name: &str, ty: &str) -> AbiInput {
        AbiInput {
            name: name.to_string(),
            type_name: ty.to_string(),
            internal_type: None,
            components: None,
            _indexed: None,
        }
    }

    fn tuple_input(name: &str, ty: &str, internal: &str, comps: Vec<AbiInput>) -> AbiInput {
        AbiInput {
            name: name.to_string(),
            type_name: ty.to_string(),
            internal_type: Some(internal.to_string()),
            components: Some(comps),
            _indexed: None,
        }
    }

    fn field_pairs(s: &GeneratedStruct) -> Vec<(&str, &str)> {
        s.fields
            .iter()
            .map(|f| (f.name.as_str(), f.rust_type.as_str()))
            .collect()
    }

    #[test]
    fn tuple_generates_sol_type_struct() {
        let comps = vec![input("x", "uint64"), input("y", "uint64")];
        let mut reg = StructRegistry::default();
        let ty = abi_param_rust_type("tuple", Some("struct IFoo.Point"), Some(&comps), &mut reg)
            .unwrap();
        assert_eq!(ty, "Point");
        assert_eq!(reg.order.len(), 1);
        assert_eq!(reg.order[0].name, "Point");
        assert_eq!(field_pairs(&reg.order[0]), vec![("x", "u64"), ("y", "u64")]);
    }

    #[test]
    fn tuple_array_shapes() {
        let comps = vec![input("x", "uint64"), input("y", "uint64")];
        let mut reg = StructRegistry::default();
        assert_eq!(
            abi_param_rust_type(
                "tuple[]",
                Some("struct IFoo.Point[]"),
                Some(&comps),
                &mut reg
            )
            .unwrap(),
            "Vec<Point>"
        );
        let mut reg = StructRegistry::default();
        assert_eq!(
            abi_param_rust_type(
                "tuple[3]",
                Some("struct IFoo.Point[3]"),
                Some(&comps),
                &mut reg
            )
            .unwrap(),
            "[Point; 3]"
        );
    }

    #[test]
    fn nested_struct_registered_before_parent() {
        let point = vec![input("x", "uint64"), input("y", "uint64")];
        let nested = vec![
            tuple_input("p", "tuple", "struct IFoo.Point", point),
            input("label", "uint256"),
        ];
        let mut reg = StructRegistry::default();
        let ty = abi_param_rust_type("tuple", Some("struct IFoo.Nested"), Some(&nested), &mut reg)
            .unwrap();
        assert_eq!(ty, "Nested");
        // Dependency (`Point`) must precede the struct that embeds it.
        let names: Vec<&str> = reg.order.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Point", "Nested"]);
        assert_eq!(
            field_pairs(&reg.order[1]),
            vec![("p", "Point"), ("label", "U256")]
        );
    }

    #[test]
    fn duplicate_struct_registered_once() {
        let comps = vec![input("x", "uint64"), input("y", "uint64")];
        let mut reg = StructRegistry::default();
        abi_param_rust_type("tuple", Some("struct IFoo.Point"), Some(&comps), &mut reg).unwrap();
        abi_param_rust_type(
            "tuple[]",
            Some("struct IFoo.Point[]"),
            Some(&comps),
            &mut reg,
        )
        .unwrap();
        assert_eq!(reg.order.len(), 1);
    }

    #[test]
    fn colliding_struct_names_rejected() {
        let comps = vec![input("x", "uint64")];
        let mut reg = StructRegistry::default();
        abi_param_rust_type("tuple", Some("struct A.Point"), Some(&comps), &mut reg).unwrap();
        let err = abi_param_rust_type("tuple", Some("struct B.Point"), Some(&comps), &mut reg)
            .unwrap_err();
        assert!(err.to_string().contains("Point"), "got {err}");
    }

    #[test]
    fn tuple_without_internal_type_rejected() {
        let comps = vec![input("x", "uint64")];
        let mut reg = StructRegistry::default();
        let err = abi_param_rust_type("tuple", None, Some(&comps), &mut reg).unwrap_err();
        assert!(err.to_string().contains("internalType"), "got {err}");
    }

    #[test]
    fn internal_type_name_parsing() {
        for it in [
            "struct IFoo.Point",
            "struct IFoo.Point[]",
            "struct IFoo.Point[][3]",
        ] {
            assert_eq!(
                struct_names_from_internal_type(Some(it)).unwrap(),
                ("IFoo.Point".to_string(), "Point".to_string())
            );
        }
    }

    #[test]
    fn keyword_idents_sanitized() {
        // Raw-identifiable keywords.
        assert_eq!(sanitize_rust_ident("ref"), "r#ref");
        assert_eq!(sanitize_rust_ident("move"), "r#move");
        assert_eq!(sanitize_rust_ident("type"), "r#type");
        assert_eq!(sanitize_rust_ident("gen"), "r#gen");
        // Keywords that cannot be raw idents get a trailing underscore.
        assert_eq!(sanitize_rust_ident("Self"), "Self_");
        assert_eq!(sanitize_rust_ident("crate"), "crate_");
        // Non-keywords pass through untouched.
        assert_eq!(sanitize_rust_ident("from"), "from");
        assert_eq!(sanitize_rust_ident("amount"), "amount");
    }

    #[test]
    fn reserved_struct_name_rejected() {
        // `Address` comes in through the template's `prelude::*` glob, and a
        // local item silently shadows a glob import — so a generated `Address`
        // would make every `address` parameter decode as this struct while the
        // project still compiles. Reject at scaffold time.
        for reserved in ["Address", "U256", "Contract", "Vec"] {
            let comps = vec![input("x", "uint64")];
            let mut reg = StructRegistry::default();
            let err = abi_param_rust_type(
                "tuple",
                Some(&format!("struct IFoo.{reserved}")),
                Some(&comps),
                &mut reg,
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains(reserved), "expected `{reserved}` in: {err}");
            assert!(err.contains("shadow"), "expected shadowing note in: {err}");
        }
    }

    #[test]
    fn non_reserved_struct_name_accepted() {
        // Guard against the deny-list being over-broad: a name that merely
        // resembles a prelude item must still scaffold.
        let comps = vec![input("x", "uint64")];
        let mut reg = StructRegistry::default();
        let ty = abi_param_rust_type(
            "tuple",
            Some("struct IFoo.AddressBook"),
            Some(&comps),
            &mut reg,
        )
        .unwrap();
        assert_eq!(ty, "AddressBook");
    }

    #[test]
    fn fields_colliding_after_snake_case_rejected() {
        // `myField` and `my_field` are distinct Solidity members that both
        // snake_case to `my_field`, which would emit a struct with a duplicate
        // field (E0124) only discovered when the user builds.
        let comps = vec![input("myField", "uint64"), input("my_field", "uint64")];
        let mut reg = StructRegistry::default();
        let err = abi_param_rust_type("tuple", Some("struct IFoo.S"), Some(&comps), &mut reg)
            .unwrap_err()
            .to_string();
        assert!(err.contains("my_field"), "{err}");
        assert!(err.contains("myField"), "{err}");
    }

    #[test]
    fn keyword_param_and_function_names_sanitized() {
        // A Solidity parameter or function named after a Rust keyword must be
        // raw-identified, or the generated signature does not parse. The macro
        // strips the `r#` when deriving the Solidity name, so the `.sol` lookup
        // still matches.
        let metadata = ContractMetadata {
            output: MetadataOutput {
                abi: vec![AbiItem::Function {
                    name: "move".to_string(),
                    inputs: vec![input("ref", "uint256"), input("gen", "uint256")],
                    outputs: vec![],
                    state_mutability: "nonpayable".to_string(),
                }],
            },
        };
        let (functions, _) = extract_function_info(&metadata).unwrap();
        assert_eq!(functions[0].name_snake, "r#move");
        assert_eq!(functions[0].params, "r#ref: U256, r#gen: U256");
    }

    #[test]
    fn tuple_keyword_field_raw_identified() {
        let comps = vec![input("ref", "uint256"), input("from", "address")];
        let mut reg = StructRegistry::default();
        let ty = abi_param_rust_type("tuple", Some("struct IFoo.Order"), Some(&comps), &mut reg)
            .unwrap();
        assert_eq!(ty, "Order");
        assert_eq!(
            field_pairs(&reg.order[0]),
            vec![("r#ref", "U256"), ("from", "Address")]
        );
    }

    #[test]
    fn scaffold_rejects_sol_import() {
        // The import check runs before solc/filesystem work, so this fails fast
        // rather than scaffolding a project that won't build.
        let sol = b"import \"./Types.sol\";\ninterface I { function f() external; }";
        let err = init_from_example_files_inner(sol, "I.sol", None, "import-test", false, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("import"), "{err}");
    }

    #[test]
    fn tuple_dynamic_classification() {
        let static_comps = vec![input("x", "uint64"), input("y", "address")];
        let dyn_comps = vec![input("name", "string")];
        assert!(!param_is_dynamic("tuple", Some(&static_comps)));
        assert!(!param_is_dynamic("tuple[3]", Some(&static_comps)));
        assert!(param_is_dynamic("tuple", Some(&dyn_comps)));
        assert!(param_is_dynamic("tuple[]", Some(&static_comps)));
        assert!(param_is_dynamic("tuple[][3]", Some(&static_comps)));
        assert!(!param_is_dynamic("tuple[2][3]", Some(&static_comps)));
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

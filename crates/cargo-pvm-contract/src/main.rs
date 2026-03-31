use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use include_dir::{Dir, include_dir};
use inquire::{Select, Text};
use log::debug;
use std::path::PathBuf;

mod build;
mod encode_decode;
mod extrinsics;
mod scaffold;

// Embed the templates directory into the binary
static TEMPLATES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

#[derive(Parser, Debug)]
#[command(name = "cargo", bin_name = "cargo", author, version)]
struct Cli {
    #[command(subcommand)]
    command: CargoSubcommand,
}

#[derive(Subcommand, Debug)]
enum CargoSubcommand {
    /// Tools for building and interacting with PVM smart contracts
    PvmContract(PvmContractArgs),
}

#[derive(Args, Debug)]
struct PvmContractArgs {
    #[command(subcommand)]
    command: PvmContractCommand,
}

#[derive(Subcommand, Debug)]
enum PvmContractCommand {
    /// Initialize a new contract project
    Init(InitArgs),
    /// Build the contract project
    Build(build::BuildArgs),
    /// Encode a function call or constructor into ABI-encoded hex calldata
    Encode(EncodeArgs),
    /// Decode ABI-encoded hex calldata back to human-readable format
    Decode(DecodeArgs),
    /// Upload contract code to the chain
    Upload(UploadArgs),
    /// Instantiate a contract (upload + deploy in one step)
    Instantiate(CliInstantiateArgs),
    /// Call an existing contract
    Call(CallArgs),
    /// Remove uploaded contract code
    Remove(RemoveArgs),
    /// Map a Substrate account to an EVM address
    MapAccount(MapAccountArgs),
    /// Query on-chain contract info by address
    Info(InfoArgs),
    /// Execute a raw RPC call against the node
    Rpc(RpcArgs),
    /// Display account info for an H160 address (balance, account ID)
    Account(AccountArgs),
}

#[derive(Args, Debug)]
struct InitArgs {
    #[arg(long, value_enum)]
    init_type: Option<InitType>,
    #[arg(long)]
    example: Option<String>,
    #[arg(long, value_enum)]
    api_style: Option<ApiStyle>,
    #[arg(long, value_enum)]
    allocator: Option<Allocator>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    sol_file: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct EncodeArgs {
    /// Path to the ABI JSON file
    #[arg(long)]
    abi: PathBuf,
    /// Function name to encode (omit for constructor)
    #[arg(long)]
    function: Option<String>,
    /// Arguments to encode (space-separated)
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,
}

#[derive(Args, Debug)]
struct DecodeArgs {
    /// Path to the ABI JSON file
    #[arg(long)]
    abi: PathBuf,
    /// Hex-encoded calldata to decode (0x-prefixed)
    #[arg(long)]
    data: String,
    /// Decode as constructor (no 4-byte selector)
    #[arg(long, default_value_t = false)]
    constructor: bool,
}

/// Common options for extrinsics commands.
#[derive(Args, Debug)]
struct ExtrinsicArgs {
    /// Websocket URL of the Substrate node
    #[arg(long, default_value = "ws://localhost:9944")]
    url: String,
    /// Secret key URI for signing (e.g. //Alice)
    #[arg(long, default_value = "//Alice")]
    suri: String,
    /// Storage deposit limit
    #[arg(long)]
    storage_deposit_limit: Option<u128>,
}

#[derive(Args, Debug)]
struct UploadArgs {
    /// Path to the .polkavm contract binary
    #[arg(long)]
    code: PathBuf,
    /// Dry-run only (no on-chain submission)
    #[arg(long, default_value_t = false)]
    dry_run: bool,
    #[command(flatten)]
    extrinsic: ExtrinsicArgs,
}

#[derive(Args, Debug)]
struct CliInstantiateArgs {
    /// Path to the .polkavm contract binary
    #[arg(long)]
    code: PathBuf,
    /// Hex-encoded constructor arguments (0x-prefixed, optional)
    #[arg(long)]
    data: Option<String>,
    /// Value to transfer to the contract
    #[arg(long, default_value_t = 0)]
    value: u128,
    /// Salt for address derivation (hex, optional)
    #[arg(long)]
    salt: Option<String>,
    /// Dry-run only (no on-chain submission)
    #[arg(long, default_value_t = false)]
    dry_run: bool,
    #[command(flatten)]
    extrinsic: ExtrinsicArgs,
}

#[derive(Args, Debug)]
struct CallArgs {
    /// Contract address (0x-prefixed H160)
    #[arg(long)]
    contract: String,
    /// Hex-encoded calldata (0x-prefixed)
    #[arg(long)]
    data: String,
    /// Value to transfer with the call
    #[arg(long, default_value_t = 0)]
    value: u128,
    /// Dry-run only (no on-chain submission)
    #[arg(long, default_value_t = false)]
    dry_run: bool,
    #[command(flatten)]
    extrinsic: ExtrinsicArgs,
}

#[derive(Args, Debug)]
struct RemoveArgs {
    /// Code hash to remove (0x-prefixed H256)
    #[arg(long)]
    code_hash: String,
    #[command(flatten)]
    extrinsic: ExtrinsicArgs,
}

#[derive(Args, Debug)]
struct MapAccountArgs {
    /// Dry-run only (no on-chain submission)
    #[arg(long, default_value_t = false)]
    dry_run: bool,
    #[command(flatten)]
    extrinsic: ExtrinsicArgs,
}

#[derive(Args, Debug)]
struct InfoArgs {
    /// Contract address (0x-prefixed H160)
    #[arg(long)]
    contract: String,
    /// Websocket URL of the Substrate node
    #[arg(long, default_value = "ws://localhost:9944")]
    url: String,
}

#[derive(Args, Debug)]
struct RpcArgs {
    /// RPC method name (e.g. system_chain, system_version)
    #[arg(long)]
    method: String,
    /// RPC parameters (JSON values, space-separated)
    #[arg(trailing_var_arg = true)]
    params: Vec<String>,
    /// Websocket URL of the Substrate node
    #[arg(long, default_value = "ws://localhost:9944")]
    url: String,
}

#[derive(Args, Debug)]
struct AccountArgs {
    /// H160 address to look up (0x-prefixed)
    #[arg(long)]
    addr: String,
    /// Output in JSON format
    #[arg(long, default_value_t = false)]
    output_json: bool,
    /// Websocket URL of the Substrate node
    #[arg(long, default_value = "ws://localhost:9944")]
    url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
enum InitType {
    New,
    Example,
}

impl std::fmt::Display for InitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitType::New => write!(f, "New contract"),
            InitType::Example => write!(f, "From an example contract"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
enum ApiStyle {
    Macro,
    Dsl,
}

impl std::fmt::Display for ApiStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiStyle::Macro => write!(f, "Macro (recommended)"),
            ApiStyle::Dsl => write!(f, "DSL (builder pattern)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
enum Allocator {
    Bump,
    NoAlloc,
}

impl std::fmt::Display for Allocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Allocator::Bump => write!(f, "Yes - bump allocator (dynamic types)"),
            Allocator::NoAlloc => write!(f, "No - stack only (smaller binary)"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExampleContract {
    name: String,
    folder: String,
    sol_filename: String,
    rust_macro: String,
    rust_dsl: String,
}

impl ExampleContract {
    fn from_dir(dir: &Dir) -> Option<Self> {
        let sol_file = dir
            .files()
            .find(|file| file.path().extension().and_then(|ext| ext.to_str()) == Some("sol"))?;
        let sol_filename = sol_file.path().file_name()?.to_str()?.to_string();
        let name = sol_file.path().file_stem()?.to_str()?.to_string();

        let rust_macro = dir
            .files()
            .find(|file| {
                file.path()
                    .file_name()
                    .and_then(|filename| filename.to_str())
                    .is_some_and(|filename| filename.ends_with("_no_alloc.rs"))
            })?
            .path()
            .file_name()?
            .to_str()?
            .to_string();
        let rust_dsl = dir
            .files()
            .find(|file| {
                file.path()
                    .file_name()
                    .and_then(|filename| filename.to_str())
                    .is_some_and(|filename| filename.ends_with("_dsl.rs"))
            })?
            .path()
            .file_name()?
            .to_str()?
            .to_string();

        Some(Self {
            name,
            folder: dir.path().to_str()?.to_string(),
            sol_filename,
            rust_macro,
            rust_dsl,
        })
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        let name = self.name.to_ascii_lowercase();
        let filename = self.sol_filename.to_ascii_lowercase();
        query == name || query == filename
    }
}

impl std::fmt::Display for ExampleContract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

fn load_examples() -> Result<Vec<ExampleContract>> {
    let examples_dir = TEMPLATES_DIR
        .get_dir("examples")
        .ok_or_else(|| anyhow::anyhow!("Examples directory not found in templates"))?;
    let mut examples: Vec<ExampleContract> = examples_dir
        .dirs()
        .filter_map(ExampleContract::from_dir)
        .collect();

    examples.sort_by(|left, right| left.name.cmp(&right.name));

    if examples.is_empty() {
        anyhow::bail!("No example contracts found in templates/examples");
    }

    Ok(examples)
}

fn find_example(examples: &[ExampleContract], query: &str) -> Result<ExampleContract> {
    examples
        .iter()
        .find(|example| example.matches(query))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Unknown example: {query}"))
}

fn main() -> Result<()> {
    env_logger::init();

    let Cli { command } = Cli::parse();
    match command {
        CargoSubcommand::PvmContract(args) => handle_pvm_contract(args),
    }
}

fn handle_pvm_contract(args: PvmContractArgs) -> Result<()> {
    match args.command {
        PvmContractCommand::Init(init_args) => init_command(init_args),
        PvmContractCommand::Build(build_args) => build::build_contracts(build_args),
        PvmContractCommand::Encode(encode_args) => encode_command(encode_args),
        PvmContractCommand::Decode(decode_args) => decode_command(decode_args),
        PvmContractCommand::Upload(a) => extrinsics::upload_command(a),
        PvmContractCommand::Instantiate(a) => extrinsics::instantiate_command(a),
        PvmContractCommand::Call(a) => extrinsics::call_command(a),
        PvmContractCommand::Remove(a) => extrinsics::remove_command(a),
        PvmContractCommand::MapAccount(a) => extrinsics::map_account_command(a),
        PvmContractCommand::Info(a) => extrinsics::info_command(a),
        PvmContractCommand::Rpc(a) => extrinsics::rpc_command(a),
        PvmContractCommand::Account(a) => extrinsics::account_command(a),
    }
}

fn encode_command(args: EncodeArgs) -> Result<()> {
    let calldata = match &args.function {
        Some(function_name) => encode_decode::encode_call(&args.abi, function_name, &args.args)?,
        None => encode_decode::encode_constructor(&args.abi, &args.args)?,
    };
    println!("0x{}", hex::encode(&calldata));
    Ok(())
}

fn decode_command(args: DecodeArgs) -> Result<()> {
    if args.constructor {
        let params = encode_decode::decode_constructor(&args.abi, &args.data)?;
        println!("Constructor parameters:");
        for p in &params {
            println!("  {} ({}): {}", p.name, p.sol_type, p.value);
        }
    } else {
        let (function_name, params) = encode_decode::decode_call(&args.abi, &args.data)?;
        println!("Function: {function_name}");
        for p in &params {
            println!("  {} ({}): {}", p.name, p.sol_type, p.value);
        }
    }
    Ok(())
}

fn init_command(args: InitArgs) -> Result<()> {
    let init_type = match args.init_type {
        Some(t) => t,
        None => {
            let init_types = vec![InitType::New, InitType::Example];
            Select::new("How do you want to initialize the project?", init_types)
                .prompt()
                .context("Failed to get initialization type")?
        }
    };

    match init_type {
        InitType::New => {
            let contract_name = prompt_name(args.name, None)?;
            let api_style = prompt_api_style(args.api_style)?;
            let use_dsl = api_style == ApiStyle::Dsl;
            let use_alloc = prompt_allocator(args.allocator)? == Allocator::Bump;
            let sol_path = prompt_sol_file(args.sol_file)?;

            check_dir_exists(&contract_name)?;

            if let Some(sol_path) = sol_path {
                debug!(
                    "Initializing from Solidity file: {} with api style: {:?}",
                    sol_path.display(),
                    api_style
                );
                let sol_file = sol_path.to_str().ok_or_else(|| {
                    anyhow::anyhow!("Solidity file path is not valid UTF-8: {sol_path:?}")
                })?;
                scaffold::init_from_solidity_file(sol_file, &contract_name, use_dsl, use_alloc)
            } else {
                debug!("Initializing new contract: {contract_name}");
                scaffold::init_new_contract(&contract_name, use_dsl, use_alloc)
            }
        }
        InitType::Example => {
            let examples = load_examples()?;

            let example = match args.example {
                Some(example_name) => find_example(&examples, &example_name)?,
                None => Select::new("Select an example:", examples)
                    .prompt()
                    .context("Failed to get example choice")?,
            };

            let api_style = prompt_api_style(args.api_style)?;
            let contract_name = prompt_name(args.name, Some(&example.name))?;

            check_dir_exists(&contract_name)?;
            debug!(
                "Initializing from example: {} with api style: {:?}",
                example.sol_filename, api_style
            );

            init_from_example(&example, &contract_name, api_style)
        }
    }
}

fn prompt_api_style(arg: Option<ApiStyle>) -> Result<ApiStyle> {
    match arg {
        Some(s) => Ok(s),
        None => {
            let styles = vec![ApiStyle::Macro, ApiStyle::Dsl];
            Select::new("Which API style?", styles)
                .prompt()
                .context("Failed to get API style choice")
        }
    }
}

fn prompt_allocator(arg: Option<Allocator>) -> Result<Allocator> {
    match arg {
        Some(a) => Ok(a),
        None => {
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                return Ok(Allocator::NoAlloc);
            }
            let allocators = vec![Allocator::Bump, Allocator::NoAlloc];
            Select::new("Use dynamic types and allocator?", allocators)
                .prompt()
                .context("Failed to get allocator choice")
        }
    }
}

fn prompt_name(arg: Option<String>, default: Option<&str>) -> Result<String> {
    let contract_name = match arg {
        Some(name) => name,
        None => {
            let mut prompt = Text::new("What is your contract name?")
                .with_help_message("This will be the name of the project directory");
            if let Some(d) = default {
                prompt = prompt.with_default(d);
            }
            prompt.prompt().context("Failed to get contract name")?
        }
    };

    if contract_name.is_empty() {
        anyhow::bail!("Contract name cannot be empty");
    }

    Ok(contract_name)
}

fn prompt_sol_file(arg: Option<PathBuf>) -> Result<Option<PathBuf>> {
    match arg {
        Some(path) => {
            if !path.exists() {
                anyhow::bail!("Solidity file not found: {}", path.display());
            }
            Ok(Some(path))
        }
        None => {
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                return Ok(None);
            }

            let sol_file = Text::new("Enter path to your .sol file (optional):")
                .with_help_message("Leave empty to skip, or provide a Solidity interface file")
                .prompt()
                .context("Failed to get .sol file path")?;

            if sol_file.trim().is_empty() {
                Ok(None)
            } else {
                let path = PathBuf::from(sol_file);
                if !path.exists() {
                    anyhow::bail!("Solidity file not found: {}", path.display());
                }
                Ok(Some(path))
            }
        }
    }
}

fn init_from_example(
    example: &ExampleContract,
    contract_name: &str,
    api_style: ApiStyle,
) -> Result<()> {
    let sol_path = format!("{}/{}", example.folder, example.sol_filename);
    let sol_file = TEMPLATES_DIR
        .get_file(&sol_path)
        .ok_or_else(|| anyhow::anyhow!("Example file not found: {sol_path}"))?;

    let use_dsl = api_style == ApiStyle::Dsl;
    let rust_example_name = if use_dsl {
        example.rust_dsl.as_str()
    } else {
        example.rust_macro.as_str()
    };

    let rust_path = format!("{}/{}", example.folder, rust_example_name);
    let rust_file = TEMPLATES_DIR
        .get_file(&rust_path)
        .ok_or_else(|| anyhow::anyhow!("Example file not found: {rust_path}"))?;

    scaffold::init_from_example_files(
        sol_file.contents(),
        &example.sol_filename,
        rust_file.contents(),
        contract_name,
        use_dsl,
    )
}

fn check_dir_exists(contract_name: &str) -> Result<()> {
    let target_dir = std::env::current_dir()?.join(contract_name);
    if target_dir.exists() {
        anyhow::bail!("Directory already exists: {target_dir:?}");
    }
    Ok(())
}

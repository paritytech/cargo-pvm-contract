use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use include_dir::{include_dir, Dir};
use log::debug;
use std::io::Write;
use std::{fs, path::PathBuf, process::Command};

// Embed the templates directory into the binary
static TEMPLATES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

/// Build contracts to PolkaVM bytecode
#[derive(Parser, Debug)]
#[command(name = "cargo")]
#[command(bin_name = "cargo")]
enum CargoCli {
    PvmContract(PvmContractArgs),
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct PvmContractArgs {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Build a contract to PolkaVM bytecode
    Build {
        /// Name of the binary to build (defaults to first binary in Cargo.toml)
        #[arg(short, long)]
        bin_name: Option<String>,

        /// Output path for the PolkaVM bytecode (defaults to ./<bin_name>.polkavm)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Initialize a new contract project from template
    Init {
        /// Name of the contract
        #[arg(value_name = "CONTRACT_NAME")]
        name: String,

        /// Template to use (defaults to pico-alloc)
        #[arg(short, long, default_value = "pico-alloc")]
        template: String,
    },
}

fn main() -> Result<()> {
    env_logger::init();

    let CargoCli::PvmContract(args) = CargoCli::parse();

    match args.command {
        Commands::Build { bin_name, output } => build_command(bin_name, output),
        Commands::Init { name, template } => init_command(name, template),
    }
}

fn build_command(bin_name: Option<String>, output: Option<PathBuf>) -> Result<()> {
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    let manifest_path = find_manifest(&current_dir)?
        .context("Could not find Cargo.toml in current directory or parent directories")?;

    debug!("Found Cargo.toml at: {}", manifest_path.display());

    let cargo_toml_content = fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read Cargo.toml at {manifest_path:?}"))?;

    let doc = cargo_toml_content
        .parse::<toml_edit::DocumentMut>()
        .context("Failed to parse Cargo.toml")?;

    let bin_name = if let Some(name) = bin_name {
        debug!("Using specified binary name: {name}");
        name
    } else {
        let first_bin_name = doc
            .get("bin")
            .and_then(|b| b.as_array_of_tables())
            .and_then(|arr| arr.get(0))
            .and_then(|bin| bin.get("name"))
            .and_then(|name| name.as_str())
            .context("No [[bin]] section found in Cargo.toml. Please specify a binary name.")?;

        debug!("Using first binary from Cargo.toml: {first_bin_name}");
        first_bin_name.to_string()
    };

    let work_dir = manifest_path.parent().unwrap();
    let build_dir = work_dir.join("target");
    let elf_path = build_contract(&manifest_path, &build_dir, &bin_name)?;
    let output_path = output.unwrap_or_else(|| PathBuf::from(format!("./{bin_name}.polkavm")));
    link_to_polkavm(&elf_path, &output_path)?;

    println!("Successfully built contract: {output_path:?}");
    Ok(())
}

fn init_command(name: String, template: String) -> Result<()> {
    debug!("Initializing new contract project: {name} with template: {template}");

    // Get the template from embedded templates
    let template_dir = TEMPLATES_DIR.get_dir(&template).ok_or_else(|| {
        anyhow::anyhow!(
            "Template '{template}' not found. Available templates: {}",
            TEMPLATES_DIR
                .dirs()
                .map(|d| d.path().file_name().unwrap().to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    let target_dir = std::env::current_dir()?.join(&name);
    if target_dir.exists() {
        anyhow::bail!("Directory already exists: {target_dir:?}");
    }

    // Create target directory
    fs::create_dir(&target_dir)
        .with_context(|| format!("Failed to create directory: {target_dir:?}"))?;

    // Copy template files from embedded directory
    copy_embedded_template(template_dir, &target_dir, &name)?;

    println!("Successfully initialized contract project: {target_dir:?}");
    println!("\nNext steps:");
    println!("  cd {name}");
    println!("  cargo pvm-contract build");
    Ok(())
}

fn copy_embedded_template(
    template_dir: &Dir,
    target_dir: &PathBuf,
    project_name: &str,
) -> Result<()> {
    use std::io::Write;

    extract_embedded_dir(template_dir, target_dir)?;
    log::debug!("Extracted template files to {template_dir:?}");

    let cargo_toml_path = template_dir.path().join("_Cargo.toml");
    let cargo_toml_file = template_dir
        .get_file(&cargo_toml_path)
        .ok_or_else(|| anyhow::anyhow!("Template missing _Cargo.toml at {cargo_toml_path:?}"))?;

    let cargo_toml_content = std::str::from_utf8(cargo_toml_file.contents())
        .context("Invalid UTF-8 in template Cargo.toml")?;

    let mut doc = cargo_toml_content
        .parse::<toml_edit::DocumentMut>()
        .context("Failed to parse template Cargo.toml")?;

    // Update the package name
    doc["package"]["name"] = toml_edit::value(project_name);

    let updated_cargo_toml = doc.to_string();
    let cargo_toml_path = target_dir.join("Cargo.toml");

    debug!("Creating Cargo.toml at {cargo_toml_path:?}");
    let mut file = fs::File::create(&cargo_toml_path)
        .with_context(|| format!("Failed to create Cargo.toml at {cargo_toml_path:?}"))?;
    file.write_all(updated_cargo_toml.as_bytes())
        .context("Failed to write Cargo.toml")?;

    Ok(())
}

fn extract_embedded_dir(embedded_dir: &Dir, target_dir: &PathBuf) -> Result<()> {
    extract_embedded_dir_impl(embedded_dir, target_dir, embedded_dir.path())
}

fn extract_embedded_dir_impl(
    embedded_dir: &Dir,
    target_dir: &PathBuf,
    base_path: &std::path::Path,
) -> Result<()> {
    for file in embedded_dir.files() {
        let relative_path = file
            .path()
            .strip_prefix(base_path)
            .context("Failed to strip template prefix from file path")?;

        // Skip _Cargo.toml as it's handled separately in copy_embedded_template
        if relative_path.file_name().and_then(|n| n.to_str()) == Some("_Cargo.toml") {
            continue;
        }

        let file_path = target_dir.join(relative_path);
        debug!("Extracting file: {relative_path:?}");

        // Create parent directories if needed
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {parent:?}"))?;
        }

        // Write file contents
        let mut output_file = fs::File::create(&file_path)
            .with_context(|| format!("Failed to create file: {file_path:?}"))?;
        output_file
            .write_all(file.contents())
            .with_context(|| format!("Failed to write file: {file_path:?}"))?;
    }

    // Recursively extract subdirectories
    for subdir in embedded_dir.dirs() {
        let relative_path = subdir
            .path()
            .strip_prefix(base_path)
            .context("Failed to strip template prefix from directory path")?;

        debug!("Extracting directory: {relative_path:?}");
        extract_embedded_dir_impl(subdir, target_dir, base_path)?;
    }

    Ok(())
}

fn find_manifest(start_dir: &std::path::Path) -> Result<Option<PathBuf>> {
    let mut current = start_dir.canonicalize()?;
    loop {
        let manifest = current.join("Cargo.toml");
        if manifest.exists() {
            return Ok(Some(manifest));
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return Ok(None),
        }
    }
}

fn build_contract(manifest_path: &PathBuf, build_dir: &PathBuf, bin_name: &str) -> Result<PathBuf> {
    debug!("Building RISC-V ELF binary for binary: {bin_name}");

    let mut args = polkavm_linker::TargetJsonArgs::default();
    args.is_64_bit = true;

    let target_json = polkavm_linker::target_json_path(args).map_err(|e| anyhow::anyhow!(e))?;

    let work_dir = manifest_path.parent().unwrap();

    let mut build_command = Command::new("cargo");
    build_command
        .current_dir(work_dir)
        .env("RUSTC_BOOTSTRAP", "1")
        .args(["build", "--release", "--manifest-path"])
        .arg(manifest_path)
        .args([
            "-Zbuild-std=core,alloc",
            "-Zbuild-std-features=panic_immediate_abort",
            "--bin",
            bin_name,
            "--target",
            &target_json.to_string_lossy(),
        ]);

    debug!("Running: {build_command:?}");
    let mut child = build_command
        .spawn()
        .context("Failed to execute cargo build")?;

    let status = child.wait().context("Failed to wait for cargo build")?;

    if !status.success() {
        anyhow::bail!("Failed to build binary {bin_name}");
    }

    let elf_path = build_dir
        .join("riscv64emac-unknown-none-polkavm/release")
        .join(bin_name);

    if !elf_path.exists() {
        anyhow::bail!("ELF binary was not generated at: {elf_path:?}");
    }

    Ok(elf_path)
}

fn link_to_polkavm(elf_path: &PathBuf, output_path: &PathBuf) -> Result<()> {
    debug!("Linking to PolkaVM bytecode...");

    let mut config = polkavm_linker::Config::default();
    config.set_strip(true);
    config.set_optimize(true);

    let elf_bytes =
        fs::read(elf_path).with_context(|| format!("Failed to read ELF from {elf_path:?}"))?;

    let linked = polkavm_linker::program_from_elf(
        config,
        polkavm_linker::TargetInstructionSet::ReviveV1,
        &elf_bytes,
    )
    .map_err(|err| anyhow::anyhow!("Failed to link PolkaVM program: {err:?}"))?;

    fs::write(output_path, &linked)
        .with_context(|| format!("Failed to write PolkaVM bytecode to {output_path:?}"))?;

    debug!("Wrote {} bytes to {output_path:?}", linked.len());
    Ok(())
}

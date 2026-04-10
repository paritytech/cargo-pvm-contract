#![doc = include_str!("../../../specs/build.md")]

mod abi;

use anyhow::{Context, Result};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub use abi::AbiJson;

/// Internal environment variable to prevent recursive builds.
const INTERNAL_BUILD_ENV: &str = "CARGO_PVM_CONTRACT_INTERNAL";

/// The builder for building a PolkaVM binary (build.rs API).
pub struct PvmBuilder {
    /// The path to the `Cargo.toml` of the project that should be built.
    project_cargo_toml: PathBuf,
    /// Specific binaries to build (None = all binaries).
    bin_names: Option<Vec<String>>,
    /// Skip ABI generation (useful for DSL contracts that don't have an abi-gen main).
    skip_abi: bool,
}

impl Default for PvmBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PvmBuilder {
    /// Create a new builder for the current project.
    pub fn new() -> Self {
        Self {
            project_cargo_toml: get_manifest_dir().join("Cargo.toml"),
            bin_names: None,
            skip_abi: false,
        }
    }

    /// Build only the specified binary.
    pub fn with_bin(mut self, name: impl Into<String>) -> Self {
        self.bin_names = Some(vec![name.into()]);
        self
    }

    /// Build only the specified binaries.
    pub fn with_bins<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.bin_names = Some(names.into_iter().map(Into::into).collect());
        self
    }

    /// Skip ABI generation. Useful for DSL contracts that don't use the `abi-gen` feature.
    pub fn skip_abi(mut self) -> Self {
        self.skip_abi = true;
        self
    }

    /// Build the PolkaVM binary.
    pub fn build(self) {
        // Check if we're in a recursive build
        if env::var(INTERNAL_BUILD_ENV).is_ok() {
            return;
        }

        if let Err(e) = build_project(&self.project_cargo_toml, self.bin_names, self.skip_abi) {
            eprintln!("PolkaVM build failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Returns the manifest dir from the `CARGO_MANIFEST_DIR` env.
fn get_manifest_dir() -> PathBuf {
    env::var("CARGO_MANIFEST_DIR")
        .expect("`CARGO_MANIFEST_DIR` is always set for `build.rs` files")
        .into()
}

/// Build profile.
#[derive(Clone)]
pub struct Profile {
    name: String,
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

impl Profile {
    /// Detect the build profile from the `PROFILE` environment variable (build.rs context).
    fn detect() -> Self {
        let name = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
        Self { name }
    }

    /// Create a profile from a name. Normalizes `"dev"` to `"debug"` internally.
    pub fn from_name(name: &str) -> Self {
        let name = if name == "dev" { "debug" } else { name }.to_string();
        Self { name }
    }

    /// The cargo `--profile` argument value.
    pub fn cargo_arg(&self) -> &str {
        if self.name == "debug" {
            "dev"
        } else {
            self.name.as_str()
        }
    }

    /// The directory name under `target/` for this profile.
    pub fn directory(&self) -> &str {
        self.name.as_str()
    }
}

/// Get the workspace target directory (build.rs context — derives from OUT_DIR).
fn get_target_root() -> PathBuf {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set"));

    for ancestor in out_dir.ancestors() {
        if ancestor.file_name().map(|n| n == "target").unwrap_or(false) {
            return ancestor.to_path_buf();
        }
    }

    out_dir
}

/// Get the build output directory (build.rs context).
fn get_build_dir() -> PathBuf {
    get_target_root().join("pvmbuild")
}

/// Get the list of binary targets from Cargo.toml.
pub fn get_bin_targets(cargo_toml: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(cargo_toml)
        .with_context(|| format!("Failed to read {}", cargo_toml.display()))?;

    let doc: toml_edit::DocumentMut = content.parse().context("Failed to parse Cargo.toml")?;

    let mut bins = Vec::new();

    if let Some(bin_array) = doc.get("bin").and_then(|b| b.as_array_of_tables()) {
        for bin in bin_array {
            if let Some(name) = bin.get("name").and_then(|n| n.as_str()) {
                bins.push(name.to_string());
            }
        }
    }

    if bins.is_empty()
        && let Some(name) = doc
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
    {
        bins.push(name.to_string());
    }

    Ok(bins)
}

/// Build a contract from the CLI. Outputs to `target/<profile>/<bin>.polkavm`.
pub fn build_contract(
    manifest_path: &Path,
    output_dir: &Path,
    profile: &Profile,
    bins: &[String],
    message_format: Option<&str>,
) -> Result<()> {
    let manifest_dir = manifest_path.parent().context("Invalid manifest path")?;
    let build_dir = output_dir.join("pvmbuild");

    build_elf(manifest_path, &build_dir, profile, bins, message_format)?;

    let elf_dir = build_dir
        .join("riscv64emac-unknown-none-polkavm")
        .join(profile.directory());

    let profile_dir = output_dir.join(profile.directory());
    fs::create_dir_all(&profile_dir).with_context(|| {
        format!(
            "Failed to create profile directory: {}",
            profile_dir.display()
        )
    })?;

    for bin in bins {
        let elf_path = elf_dir.join(bin);
        if !elf_path.exists() {
            anyhow::bail!("ELF binary not found at: {}", elf_path.display());
        }

        let output_path = profile_dir.join(format!("{bin}.polkavm"));
        link_to_polkavm(&elf_path, &output_path)?;

        let abi_path = profile_dir.join(format!("{bin}.abi.json"));
        generate_abi_file(manifest_dir, bin, &abi_path, Some(output_dir))?;
    }

    Ok(())
}

/// Build the project (build.rs context). Outputs to `target/<bin>.<profile>.polkavm`.
fn build_project(
    project_cargo_toml: &Path,
    bin_names: Option<Vec<String>>,
    skip_abi: bool,
) -> Result<()> {
    let profile = Profile::detect();
    let build_dir = get_build_dir();
    let target_root = get_target_root();
    let manifest_dir = project_cargo_toml
        .parent()
        .context("Invalid manifest path")?;

    let bins_to_build = match bin_names {
        Some(names) => names,
        None => get_bin_targets(project_cargo_toml)?,
    };

    if bins_to_build.is_empty() {
        anyhow::bail!("No binary targets found in Cargo.toml");
    }

    build_elf(
        project_cargo_toml,
        &build_dir,
        &profile,
        &bins_to_build,
        None,
    )?;

    let elf_dir = build_dir
        .join("riscv64emac-unknown-none-polkavm")
        .join(profile.directory());

    for bin in &bins_to_build {
        let elf_path = elf_dir.join(bin);
        if !elf_path.exists() {
            anyhow::bail!("ELF binary not found at: {}", elf_path.display());
        }

        let output_path = target_root.join(format!("{}.{}.polkavm", bin, profile.directory()));
        link_to_polkavm(&elf_path, &output_path)?;

        if !skip_abi {
            let abi_path = target_root.join(format!("{}.{}.abi.json", bin, profile.directory()));
            generate_abi_file(manifest_dir, bin, &abi_path, None)?;
        }
    }

    Ok(())
}

/// Build the ELF binary using cargo (shared by both CLI and build.rs paths).
fn build_elf(
    manifest_path: &Path,
    target_dir: &Path,
    profile: &Profile,
    bins: &[String],
    message_format: Option<&str>,
) -> Result<()> {
    let rustflags = "-Zunstable-options -Cpanic=immediate-abort";

    let mut target_args = polkavm_linker::TargetJsonArgs::default();
    target_args.is_64_bit = true;
    target_args.rustc_version = polkavm_linker::RustcVersion::Rustc_1_91;
    let target_json = polkavm_linker::target_json_path(target_args)
        .map_err(|e| anyhow::anyhow!("Failed to get target JSON: {e}"))?;

    let work_dir = manifest_path.parent().context("Invalid manifest path")?;

    let mut cmd = Command::new("cargo");
    cmd.current_dir(work_dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env_remove("RUSTUP_TOOLCHAIN")
        // Disable strip during ELF build - it conflicts with --emit-relocs required by PolkaVM.
        // Stripping is done later by polkavm_linker after processing relocations.
        .env("RUSTFLAGS", rustflags)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_PROFILE_RELEASE_STRIP", "false")
        .env("RUSTC_BOOTSTRAP", "1")
        .env(INTERNAL_BUILD_ENV, "1")
        .arg("build")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--profile")
        .arg(profile.cargo_arg())
        .arg("--target")
        .arg(&target_json)
        .arg("-Zbuild-std=core,alloc")
        .arg("-Zjson-target-spec");

    for bin in bins {
        cmd.arg("--bin").arg(bin);
    }

    if let Some(fmt) = message_format {
        cmd.arg("--message-format").arg(fmt);
    }

    eprintln!("Building PolkaVM binary with profile: {profile}");

    let status = cmd.status().context("Failed to execute cargo build")?;

    if !status.success() {
        anyhow::bail!("Cargo build failed");
    }

    Ok(())
}

fn generate_abi_file(
    manifest_dir: &Path,
    bin_name: &str,
    output_path: &Path,
    target_root: Option<&Path>,
) -> Result<()> {
    match abi::generate_abi_for_bin(manifest_dir, bin_name, target_root) {
        Ok(Some(abi)) => {
            let json =
                serde_json::to_string_pretty(&abi).context("Failed to serialize ABI to JSON")?;
            fs::write(output_path, json)
                .with_context(|| format!("Failed to write ABI to {}", output_path.display()))?;
            eprintln!("Created ABI: {}", output_path.display());
        }
        Ok(None) => {
            eprintln!("No pvm_contract found, skipping ABI generation");
        }
        Err(e) => {
            eprintln!("Warning: Failed to generate ABI: {e}");
        }
    }
    Ok(())
}

/// Link an ELF binary to PolkaVM bytecode.
fn link_to_polkavm(elf_path: &Path, output_path: &Path) -> Result<()> {
    let elf_bytes = fs::read(elf_path)
        .with_context(|| format!("Failed to read ELF from {}", elf_path.display()))?;

    let mut config = polkavm_linker::Config::default();
    config.set_strip(true);
    config.set_optimize(true);

    let linked = polkavm_linker::program_from_elf(
        config,
        polkavm_linker::TargetInstructionSet::ReviveV1,
        &elf_bytes,
    )
    .map_err(|e| anyhow::anyhow!("Failed to link PolkaVM program: {e}"))?;

    fs::write(output_path, &linked).with_context(|| {
        format!(
            "Failed to write PolkaVM bytecode to {}",
            output_path.display()
        )
    })?;

    eprintln!(
        "Created PolkaVM binary: {} ({} bytes)",
        output_path.display(),
        linked.len()
    );

    Ok(())
}

//! # PolkaVM Contract Builder
//!
//! A utility for building Rust projects as PolkaVM bytecode.
//!
//! ## Usage in `build.rs`
//!
//! ```no_run
//! cargo_pvm_contract_builder::PvmBuilder::new().build();
//! ```

mod abi;

use anyhow::{Context, Result};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub use abi::extract_abi_from_elf;

/// Internal environment variable to prevent recursive builds.
const INTERNAL_BUILD_ENV: &str = "CARGO_PVM_CONTRACT_INTERNAL";

/// The builder for building a PolkaVM binary.
pub struct PvmBuilder {
    /// The path to the `Cargo.toml` of the project that should be built.
    project_cargo_toml: PathBuf,
    /// Specific binaries to build (None = all binaries).
    bin_names: Option<Vec<String>>,
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

    /// Build the PolkaVM binary.
    pub fn build(self) {
        // Check if we're in a recursive build
        if env::var(INTERNAL_BUILD_ENV).is_ok() {
            return;
        }

        if let Err(e) = build_project(&self.project_cargo_toml, self.bin_names) {
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

/// Detect the build profile from the environment.
#[derive(Clone, Debug)]
struct Profile {
    name: String,
}

impl Profile {
    fn detect() -> Self {
        let name = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
        Self { name }
    }

    fn cargo_arg(&self) -> &str {
        if self.name == "debug" {
            "dev"
        } else {
            self.name.as_str()
        }
    }

    fn directory(&self) -> &str {
        self.name.as_str()
    }
}

/// Get the workspace target directory.
fn get_target_root() -> PathBuf {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set"));

    for ancestor in out_dir.ancestors() {
        if ancestor.file_name().map(|n| n == "target").unwrap_or(false) {
            return ancestor.to_path_buf();
        }
    }

    out_dir
}

/// Get the build output directory.
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

/// Get the package name from Cargo.toml.
pub fn get_package_name(cargo_toml: &Path) -> Result<String> {
    let content = fs::read_to_string(cargo_toml)
        .with_context(|| format!("Failed to read {}", cargo_toml.display()))?;
    let doc: toml_edit::DocumentMut = content.parse().context("Failed to parse Cargo.toml")?;
    doc.get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
        .context("No package name found in Cargo.toml")
}

/// Build a contract externally (called by `cargo pvm-contract build`).
///
/// Unlike `PvmBuilder::build()` which runs inside build.rs, this is called
/// from the CLI tool and takes explicit paths instead of reading env vars.
///
/// `pkg_name` is the entry-point crate name (used for `PVM_ENTRY_POINT_CRATE`).
/// `bin_names` are the binary targets to build.
pub fn build_contract(
    project_cargo_toml: &Path,
    output_dir: &Path,
    profile_name: &str,
    pkg_name: &str,
    bin_names: Vec<String>,
) -> Result<()> {
    let profile = Profile { name: profile_name.to_string() };
    let target_dir = output_dir.join("pvmbuild");

    if bin_names.is_empty() {
        anyhow::bail!("No binary targets to build");
    }

    build_elf(project_cargo_toml, &target_dir, &profile, &bin_names, pkg_name)?;

    let elf_dir = target_dir
        .join("riscv64emac-unknown-none-polkavm")
        .join(profile.directory());

    for bin in &bin_names {
        let elf_path = elf_dir.join(bin);
        if !elf_path.exists() {
            anyhow::bail!("ELF binary not found at: {}", elf_path.display());
        }

        let polkavm_path = output_dir.join(format!("{}.{}.polkavm", bin, profile.directory()));
        link_to_polkavm(&elf_path, &polkavm_path)?;

        let abi_path = output_dir.join(format!("{}.{}.abi.json", bin, profile.directory()));
        generate_abi_file(&elf_path, &abi_path)?;

        let cdm_path = output_dir.join(format!("{}.{}.cdm.json", bin, profile.directory()));
        generate_cdm_file(&elf_path, &cdm_path)?;
    }

    Ok(())
}

/// Build the project (called from build.rs context).
fn build_project(project_cargo_toml: &Path, bin_names: Option<Vec<String>>) -> Result<()> {
    let profile = Profile::detect();
    let build_dir = get_build_dir();
    let target_root = get_target_root();

    let bins_to_build = match bin_names {
        Some(names) => names,
        None => get_bin_targets(project_cargo_toml)?,
    };

    if bins_to_build.is_empty() {
        anyhow::bail!("No binary targets found in Cargo.toml");
    }

    let target_dir = build_dir;
    let pkg_name = get_package_name(project_cargo_toml)?;
    build_elf(project_cargo_toml, &target_dir, &profile, &bins_to_build, &pkg_name)?;

    // Link each ELF to PolkaVM
    let elf_dir = target_dir
        .join("riscv64emac-unknown-none-polkavm")
        .join(profile.directory());

    for bin in &bins_to_build {
        let elf_path = elf_dir.join(bin);
        if !elf_path.exists() {
            anyhow::bail!("ELF binary not found at: {}", elf_path.display());
        }

        let output_path = target_root.join(format!("{}.{}.polkavm", bin, profile.directory()));
        link_to_polkavm(&elf_path, &output_path)?;

        let abi_path = target_root.join(format!("{}.{}.abi.json", bin, profile.directory()));
        generate_abi_file(&elf_path, &abi_path)?;

        let cdm_path = target_root.join(format!("{}.{}.cdm.json", bin, profile.directory()));
        generate_cdm_file(&elf_path, &cdm_path)?;
    }

    Ok(())
}

pub fn generate_abi_file(elf_path: &Path, output_path: &Path) -> Result<()> {
    let elf_bytes = fs::read(elf_path)
        .with_context(|| format!("Failed to read ELF: {}", elf_path.display()))?;
    match abi::extract_abi_from_elf(&elf_bytes)? {
        Some(json) => {
            fs::write(output_path, &json)
                .with_context(|| format!("Failed to write ABI to {}", output_path.display()))?;
            eprintln!("Created ABI: {}", output_path.display());
        }
        None => {
            eprintln!("No ABI section found, skipping ABI generation");
        }
    }
    Ok(())
}

pub fn generate_cdm_file(elf_path: &Path, output_path: &Path) -> Result<()> {
    let elf_bytes = fs::read(elf_path)
        .with_context(|| format!("Failed to read ELF: {}", elf_path.display()))?;
    match abi::extract_cdm_from_elf(&elf_bytes)? {
        Some(json) => {
            fs::write(output_path, &json)
                .with_context(|| format!("Failed to write CDM metadata to {}", output_path.display()))?;
            eprintln!("Created CDM metadata: {}", output_path.display());
        }
        None => {
            // No CDM metadata - contract doesn't have cdm attribute. This is normal.
        }
    }
    Ok(())
}

/// Build the ELF binary using cargo.
fn build_elf(
    manifest_path: &Path,
    target_dir: &Path,
    profile: &Profile,
    bins: &[String],
    pkg_name: &str,
) -> Result<()> {
    // Include pkg_name in RUSTFLAGS to force cargo cache invalidation when
    // switching between contracts. Without this, cached proc macro expansions
    // from a different PVM_ENTRY_POINT_CRATE value could be reused incorrectly.
    let rustflags = format!(
        "-Zunstable-options -Cpanic=immediate-abort -Clink-arg=--undefined=__PVM_ABI -Clink-arg=--undefined=__PVM_CDM --cfg pvm_entry_crate=\"{}\"",
        pkg_name
    );

    let mut args = polkavm_linker::TargetJsonArgs::default();
    args.is_64_bit = true;
    let target_json = polkavm_linker::target_json_path(args)
        .map_err(|e| anyhow::anyhow!("Failed to get target JSON: {e}"))?;

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let work_dir = manifest_path.parent().context("Invalid manifest path")?;

    let mut cmd = Command::new(&cargo);
    cmd.current_dir(work_dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS") // We set RUSTFLAGS, but cargo prefers this one
        .env_remove("RUSTC") // Prevent host toolchain override from build.rs
        .env("RUSTFLAGS", &rustflags)
        .env("CARGO_TARGET_DIR", target_dir)
        // Disable strip during ELF build - it conflicts with --emit-relocs required by PolkaVM.
        // Stripping is done later by polkavm_linker after processing relocations.
        .env("CARGO_PROFILE_RELEASE_STRIP", "false")
        .env("RUSTC_BOOTSTRAP", "1")
        .env(INTERNAL_BUILD_ENV, "1")
        // Tell each crate's build.rs which crate should generate entry points.
        // Only the primary contract gets deploy/call; dependencies skip them.
        .env("PVM_ENTRY_POINT_CRATE", pkg_name)
        .arg("build")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--profile")
        .arg(profile.cargo_arg())
        .arg("--target")
        .arg(&target_json)
        .arg("-Zbuild-std=core,alloc");

    for bin in bins {
        cmd.arg("--bin").arg(bin);
    }

    eprintln!("Building PolkaVM binary with profile: {profile:?}");

    let output = cmd.output().context("Failed to execute cargo build")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Cargo build failed:\n{stderr}");
    }

    Ok(())
}

/// Link an ELF binary to PolkaVM bytecode.
pub fn link_to_polkavm(elf_path: &Path, output_path: &Path) -> Result<()> {
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

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
const BUILD_PLAN_REASON: &str = "cargo-pvm-contract-build-plan";

struct CargoBuildCommand<'a> {
    manifest_path: &'a Path,
    target_dir: &'a Path,
    profile: &'a Profile,
    bins: &'a [String],
    work_dir: &'a Path,
    target_json: &'a Path,
    has_toolchain_file: bool,
    use_json_target_spec: bool,
    features: Option<&'a str>,
    no_default_features: bool,
}

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
    features: Option<&str>,
    no_default_features: bool,
) -> Result<()> {
    let manifest_path = canonicalize_manifest(manifest_path)?;
    let manifest_dir = manifest_path.parent().context("Invalid manifest path")?;
    let build_dir = output_dir.join("pvmbuild");

    build_elf(
        &manifest_path,
        &build_dir,
        profile,
        bins,
        message_format,
        features,
        no_default_features,
    )?;

    let elf_dir = build_dir
        .join("riscv64emac-unknown-none-polkavm")
        .join(profile.directory());
    let profile_dir = output_dir.join(profile.directory());

    process_elf_binaries(
        &elf_dir,
        &profile_dir,
        bins,
        manifest_dir,
        Some(output_dir),
        true,
        features,
    )
}

/// Build the project (build.rs context). Outputs to `target/<profile>/<bin>.polkavm`.
fn build_project(
    project_cargo_toml: &Path,
    bin_names: Option<Vec<String>>,
    skip_abi: bool,
) -> Result<()> {
    let profile = Profile::detect();
    let build_dir = get_build_dir();
    let target_root = get_target_root();
    let project_cargo_toml = canonicalize_manifest(project_cargo_toml)?;
    let manifest_dir = project_cargo_toml
        .parent()
        .context("Invalid manifest path")?;

    let bins_to_build = match bin_names {
        Some(names) => names,
        None => get_bin_targets(&project_cargo_toml)?,
    };

    if bins_to_build.is_empty() {
        anyhow::bail!("No binary targets found in Cargo.toml");
    }

    build_elf(
        &project_cargo_toml,
        &build_dir,
        &profile,
        &bins_to_build,
        None,
        None,
        false,
    )?;

    let elf_dir = build_dir
        .join("riscv64emac-unknown-none-polkavm")
        .join(profile.directory());
    let profile_dir = target_root.join(profile.directory());

    process_elf_binaries(
        &elf_dir,
        &profile_dir,
        &bins_to_build,
        manifest_dir,
        None,
        !skip_abi,
        None,
    )
}

/// Link each ELF binary to PolkaVM bytecode and optionally generate its ABI JSON.
/// Creates `profile_dir` if it doesn't exist.
fn process_elf_binaries(
    elf_dir: &Path,
    profile_dir: &Path,
    bins: &[String],
    manifest_dir: &Path,
    abi_target_root: Option<&Path>,
    generate_abi: bool,
    features: Option<&str>,
) -> Result<()> {
    fs::create_dir_all(profile_dir).with_context(|| {
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
        let abi_path = profile_dir.join(format!("{bin}.abi.json"));

        link_to_polkavm(&elf_path, &output_path)?;

        // Clear any previous `.abi.json`. The re-emit below skips writing
        // when `generate_abi == false` (`PvmBuilder::skip_abi(true)`) or
        // the source has no `#[contract]` macro, so without this cleanup
        // a stale ABI would survive a `.polkavm` overwrite.
        // `NotFound` is the expected first-build case; everything else
        // (permissions, IO) is a real error worth surfacing.
        if let Err(e) = fs::remove_file(&abi_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(e)
                .with_context(|| format!("Failed to remove stale ABI: {}", abi_path.display()));
        }

        if generate_abi {
            generate_abi_file(manifest_dir, bin, &abi_path, abi_target_root, features)?;
        }
    }

    Ok(())
}

/// Resolve a `Cargo.toml` path to an absolute path so downstream `parent()`
/// calls always yield a real working directory (not the empty `OsStr` returned
/// for bare `"Cargo.toml"`, which would make `Command::current_dir` fail with
/// ENOENT before cargo is even spawned).
fn canonicalize_manifest(manifest_path: &Path) -> Result<PathBuf> {
    manifest_path.canonicalize().with_context(|| {
        format!(
            "Could not canonicalize manifest path: {}",
            manifest_path.display()
        )
    })
}

/// Build the ELF binary using cargo (shared by both CLI and build.rs paths).
fn build_elf(
    manifest_path: &Path,
    target_dir: &Path,
    profile: &Profile,
    bins: &[String],
    message_format: Option<&str>,
    features: Option<&str>,
    no_default_features: bool,
) -> Result<()> {
    let work_dir = manifest_path.parent().context("Invalid manifest path")?;

    // Remove RUSTUP_TOOLCHAIN only when the project has a rust-toolchain.toml that
    // should control the toolchain. Without it, we keep the inherited toolchain
    // (e.g. nightly passed via `cargo +nightly`).
    let has_toolchain_file =
        work_dir.join("rust-toolchain.toml").exists() || work_dir.join("rust-toolchain").exists();

    let mut target_args = polkavm_linker::TargetJsonArgs::default();
    target_args.is_64_bit = true;
    target_args.rustc_version = detect_rustc_version(work_dir, has_toolchain_file);
    let target_json = polkavm_linker::target_json_path(target_args)
        .map_err(|e| anyhow::anyhow!("Failed to get target JSON: {e}"))?;

    let use_json_target_spec =
        cargo_supports_z_flag("json-target-spec", work_dir, has_toolchain_file);
    let cargo_build = CargoBuildCommand {
        manifest_path,
        target_dir,
        profile,
        bins,
        work_dir,
        target_json: &target_json,
        has_toolchain_file,
        use_json_target_spec,
        features,
        no_default_features,
    };

    let mut cmd = cargo_build_command(&cargo_build);
    if let Some(fmt) = message_format {
        cmd.arg("--message-format").arg(fmt);
    }

    eprintln!("Building PolkaVM binary with profile: {profile}");

    if message_format.is_some() {
        if message_format.is_some_and(cargo_message_format_is_json) {
            emit_build_plan_if_available(&cargo_build);
        }

        let status = cmd.status().context("Failed to execute cargo build")?;
        if !status.success() {
            anyhow::bail!("Cargo build failed");
        }
        return Ok(());
    }

    let output = cmd
        .output()
        .with_context(|| format!("Failed to spawn cargo build (cwd: {})", work_dir.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Cargo build failed:\n{stderr}");
    }

    Ok(())
}

fn cargo_build_command(config: &CargoBuildCommand<'_>) -> Command {
    let rustflags = "-Zunstable-options -Cpanic=immediate-abort";
    let mut cmd = Command::new("cargo");
    cmd.current_dir(config.work_dir)
        // Avoid leaking parent-cargo state into the polkavm child build:
        // - CARGO_ENCODED_RUSTFLAGS would override the RUSTFLAGS we set
        // - RUSTC / RUSTC_WRAPPER / RUSTC_WORKSPACE_WRAPPER would force a
        //   wrapped or wrong-toolchain rustc (sccache/llvm-cov instrumentation
        //   breaks -Zbuild-std=core,alloc by injecting profiler runtime into
        //   no_std)
        // - CARGO points at the parent's cargo binary (was previously stripped
        //   by tests/test_cmd.rs)
        .env_remove("CARGO")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        // Disable strip during ELF build - it conflicts with --emit-relocs required by PolkaVM.
        // Stripping is done later by polkavm_linker after processing relocations.
        .env("RUSTFLAGS", rustflags)
        .env("CARGO_TARGET_DIR", config.target_dir)
        .env("CARGO_PROFILE_RELEASE_STRIP", "false")
        .env("RUSTC_BOOTSTRAP", "1")
        .env(INTERNAL_BUILD_ENV, "1")
        .arg("build")
        .arg("--manifest-path")
        .arg(config.manifest_path)
        .arg("--profile")
        .arg(config.profile.cargo_arg())
        .arg("--target")
        .arg(config.target_json)
        .arg("-Zbuild-std=core,alloc");

    if config.has_toolchain_file {
        cmd.env_remove("RUSTUP_TOOLCHAIN");
    }

    if config.use_json_target_spec {
        cmd.arg("-Zjson-target-spec");
    }

    for bin in config.bins {
        cmd.arg("--bin").arg(bin);
    }

    if let Some(list) = config.features {
        cmd.arg("--features").arg(list);
    }

    if config.no_default_features {
        cmd.arg("--no-default-features");
    }

    cmd
}

fn emit_build_plan_if_available(config: &CargoBuildCommand<'_>) {
    if let Ok(total) = planned_compiler_artifact_count(config) {
        println!(
            "{}",
            serde_json::json!({
                "reason": BUILD_PLAN_REASON,
                "schema_version": 1,
                "unit": "compiler-artifact",
                "total": total,
            })
        );
    }
}

fn cargo_message_format_is_json(format: &str) -> bool {
    format.split(',').any(|part| part.trim().contains("json"))
}

fn planned_compiler_artifact_count(config: &CargoBuildCommand<'_>) -> Result<usize> {
    let mut cmd = cargo_build_command(config);
    cmd.arg("--unit-graph").arg("-Zunstable-options");

    let output = cmd
        .output()
        .context("Failed to execute cargo build --unit-graph")?;

    if !output.status.success() {
        anyhow::bail!("cargo build --unit-graph failed");
    }

    compiler_artifact_count_from_unit_graph(&output.stdout)
}

fn compiler_artifact_count_from_unit_graph(unit_graph: &[u8]) -> Result<usize> {
    let unit_graph: serde_json::Value =
        serde_json::from_slice(unit_graph).context("Failed to parse cargo unit graph")?;
    let units = unit_graph
        .get("units")
        .and_then(serde_json::Value::as_array)
        .context("cargo unit graph missing `units`")?;

    Ok(units
        .iter()
        .filter(|unit| unit.get("mode").and_then(serde_json::Value::as_str) == Some("build"))
        .count())
}

/// Detect the rustc version that the build subprocess will use.
///
/// Probes `rustc --version` with the same env treatment as `build_elf` (RUSTC removed,
/// current_dir set to work_dir, RUSTUP_TOOLCHAIN conditionally removed) so that
/// rustup / rust-toolchain.toml resolve identically to the actual build.
fn detect_rustc_version(
    work_dir: &Path,
    remove_toolchain_env: bool,
) -> polkavm_linker::RustcVersion {
    let mut probe = Command::new("rustc");
    probe
        .current_dir(work_dir)
        .arg("--version")
        .env_remove("RUSTC")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if remove_toolchain_env {
        probe.env_remove("RUSTUP_TOOLCHAIN");
    }

    let output = match probe.output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => return polkavm_linker::RustcVersion::Rustc_1_91,
    };

    // Parse "rustc X.Y.Z ..." or "rustc X.Y.Z-nightly (hash YYYY-MM-DD)"
    // and check whether it meets the >=1.91 / nightly>=2025-09-01 cutoff
    // that polkavm-linker uses for the new target JSON format.
    // polkavm-linker's cutoff: stable >=1.91 or nightly >=2025-09-01
    if rustc_version_at_least(&output, (1, 91), (2025, 9, 1)) {
        polkavm_linker::RustcVersion::Rustc_1_91
    } else {
        polkavm_linker::RustcVersion::Legacy
    }
}

/// Check if a `rustc --version` output meets a minimum version requirement.
///
/// `stable_version`: `(major, minor)` — for stable releases, checks `version >= major.minor`.
/// `nightly_date`: `(year, month, day)` — for nightly, checks the commit date.
///
/// Mirrors polkavm-linker's `VersionDetector::check_feature()`.
fn rustc_version_at_least(
    version_output: &str,
    stable_version: (u32, u32),
    nightly_date: (u32, u32, u32),
) -> bool {
    let parts: Vec<&str> = version_output.split_whitespace().collect();
    let Some(version_str) = parts.get(1) else {
        return true;
    };
    let is_nightly = version_str.contains("-nightly");
    let mut nums = version_str.split(['.', '-']);
    let major: u32 = nums.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let minor: u32 = nums
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(stable_version.1);

    let (req_major, req_minor) = stable_version;
    if !is_nightly {
        return major > req_major || (major == req_major && minor >= req_minor);
    }

    if major > req_major || (major == req_major && minor > req_minor) {
        return true;
    }

    // Nightly with minor < req_minor is definitively below the cutoff.
    if major == req_major && minor < req_minor {
        return false;
    }

    // Nightly with minor == req_minor: check commit date from "(hash YYYY-MM-DD)"
    let (req_y, req_m, req_d) = nightly_date;
    parts
        .last()
        .and_then(|s| {
            let s = s.trim_end_matches(')');
            let mut it = s.split('-');
            let y: u32 = it.next()?.parse().ok()?;
            let m: u32 = it.next()?.parse().ok()?;
            let d: u32 = it.next()?.parse().ok()?;
            Some((y, m, d) >= (req_y, req_m, req_d))
        })
        .unwrap_or(true)
}

/// Check whether the cargo that will run the build still accepts `-Z<flag>`.
/// `remove_toolchain_env` mirrors the build command's `env_remove("RUSTUP_TOOLCHAIN")`
/// so that we probe the exact same cargo binary.
fn cargo_supports_z_flag(flag: &str, work_dir: &Path, remove_toolchain_env: bool) -> bool {
    let mut probe = Command::new("cargo");
    probe
        .current_dir(work_dir)
        .arg(format!("-Z{flag}"))
        .arg("version")
        .env("RUSTC_BOOTSTRAP", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if remove_toolchain_env {
        probe.env_remove("RUSTUP_TOOLCHAIN");
    }
    probe.status().map(|s| s.success()).unwrap_or(false)
}

fn generate_abi_file(
    manifest_dir: &Path,
    bin_name: &str,
    output_path: &Path,
    target_root: Option<&Path>,
    features: Option<&str>,
) -> Result<()> {
    let abi = match abi::generate_abi_for_bin(manifest_dir, bin_name, target_root, features) {
        Ok(Some(abi)) => abi,
        Ok(None) => {
            eprintln!("No pvm_contract found, skipping ABI generation");
            return Ok(());
        }
        Err(e) => {
            return Err(e).context("Failed to generate ABI");
        }
    };

    let storage_layout =
        abi::generate_storage_layout_for_bin(manifest_dir, bin_name, target_root, features)?;

    let json = if let Some(layout) = storage_layout {
        let abi_value = serde_json::to_value(&abi).context("Failed to serialize ABI")?;
        serde_json::to_string_pretty(&serde_json::json!({
            "abi": abi_value,
            "storageLayout": layout,
        }))
        .context("Failed to serialize ABI + storageLayout")?
    } else {
        serde_json::to_string_pretty(&abi).context("Failed to serialize ABI to JSON")?
    };

    fs::write(output_path, json)
        .with_context(|| format!("Failed to write ABI to {}", output_path.display()))?;
    eprintln!("Created ABI: {}", output_path.display());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: polkavm-linker 1.91 cutoff (stable >=1.91, nightly >=2025-09-01).
    fn check_1_91(version: &str) -> bool {
        rustc_version_at_least(version, (1, 91), (2025, 9, 1))
    }

    #[test]
    fn stable_at_or_above_cutoff() {
        assert!(check_1_91("rustc 1.91.0 (abcd1234 2025-10-30)"));
        assert!(check_1_91("rustc 1.92.0 (ded5c06cf 2025-12-08)"));
        assert!(check_1_91("rustc 1.93.0 (254b59607 2026-01-19)"));
    }

    #[test]
    fn stable_below_cutoff() {
        assert!(!check_1_91("rustc 1.90.0 (abcd1234 2025-09-18)"));
        assert!(!check_1_91("rustc 1.85.0 (abcd1234 2025-02-20)"));
    }

    #[test]
    fn nightly_above_cutoff() {
        assert!(check_1_91("rustc 1.96.0-nightly (f5eca4fcf 2026-04-09)"));
        assert!(check_1_91("rustc 1.92.0-nightly (abcd1234 2025-10-15)"));
        assert!(check_1_91("rustc 1.91.0-nightly (abcd1234 2025-09-01)"));
    }

    #[test]
    fn nightly_below_cutoff() {
        assert!(!check_1_91("rustc 1.91.0-nightly (abcd1234 2025-08-15)"));
        assert!(!check_1_91("rustc 1.91.0-nightly (abcd1234 2025-08-31)"));
        // minor < req_minor must return false even when date is after cutoff
        assert!(!check_1_91("rustc 1.90.0-nightly (abcd1234 2025-10-15)"));
    }

    #[test]
    fn different_cutoff() {
        // Example: hypothetical cutoff at stable >=1.95, nightly >=2026-03-01
        let check = |v: &str| rustc_version_at_least(v, (1, 95), (2026, 3, 1));
        assert!(check("rustc 1.95.0 (abcd1234 2026-05-01)"));
        assert!(check("rustc 1.96.0-nightly (abcd1234 2026-04-01)"));
        assert!(!check("rustc 1.94.0 (abcd1234 2026-04-01)"));
        assert!(!check("rustc 1.95.0-nightly (abcd1234 2026-02-28)"));
        // minor < req_minor must return false even when date is after cutoff
        assert!(!check("rustc 1.94.0-nightly (abcd1234 2026-04-01)"));
    }
}

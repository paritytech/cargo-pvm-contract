use anyhow::{Context, Result};
use cargo_pvm_contract_builder as builder;
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Args, Debug)]
pub struct BuildArgs {
    /// Path to Cargo.toml
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    /// Package to build. Selects a workspace member by name when the manifest
    /// is a workspace; also accepts the package's own name for a single crate.
    #[arg(short = 'p', long = "package")]
    package: Option<String>,

    /// Build profile (default: release)
    #[arg(long)]
    profile: Option<String>,

    /// Output directory for .polkavm and .abi.json files
    #[arg(short = 'o', long = "output-dir")]
    output_dir: Option<PathBuf>,

    /// Cargo message format
    #[arg(long)]
    message_format: Option<String>,

    /// Space or comma separated list of features to activate.
    /// Features must be host-buildable since ABI generation runs on the host triple.
    #[arg(long = "features", value_name = "FEATURES")]
    features: Option<String>,

    /// Do not activate the package's default features for the contract build.
    /// Default features stay enabled for ABI generation so the host-side
    /// abi-gen build keeps the broadest possible feature set available.
    #[arg(long = "no-default-features")]
    no_default_features: bool,
}

pub fn build_contracts(args: BuildArgs) -> Result<()> {
    let input_manifest = match args.manifest_path {
        Some(path) => path,
        None => std::env::current_dir()
            .context("Failed to determine current working directory")?
            .join("Cargo.toml"),
    };

    let input_manifest = input_manifest
        .canonicalize()
        .with_context(|| format!("Manifest not found: {}", input_manifest.display()))?;

    let (manifest_path, workspace_root) = match args.package.as_deref() {
        Some(pkg) => resolve_workspace_member(&input_manifest, pkg)?,
        None => (input_manifest, None),
    };

    let profile_name = args.profile.as_deref().unwrap_or("release");
    let profile = builder::Profile::from_name(profile_name);

    let bins = builder::get_bin_targets(&manifest_path)?;
    if bins.is_empty() {
        anyhow::bail!("No binary targets found in {}", manifest_path.display());
    }

    let output_dir = args
        .output_dir
        .unwrap_or_else(|| find_target_dir(&manifest_path, workspace_root.as_deref()));

    std::fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "Failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    builder::build_contract(
        &manifest_path,
        &output_dir,
        &profile,
        &bins,
        args.message_format.as_deref(),
        args.features.as_deref(),
        args.no_default_features,
    )?;

    Ok(())
}

fn find_target_dir(manifest_path: &Path, workspace_root: Option<&Path>) -> PathBuf {
    // Precedence mirrors cargo: env var > workspace root > manifest dir.
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(root) = workspace_root {
        return root.join("target");
    }
    manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("target")
}

/// Resolve `<package>` via `cargo metadata` against `manifest_path`. Works
/// whether the manifest is a workspace root (selecting a member by name) or
/// a single-crate manifest (where `<package>` must match the crate's own
/// name). Returns the resolved member's `Cargo.toml` and the workspace root
/// if cargo reports one.
fn resolve_workspace_member(
    manifest_path: &Path,
    package: &str,
) -> Result<(PathBuf, Option<PathBuf>)> {
    let mut cmd = Command::new("cargo");
    cmd.arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
        .arg("--manifest-path")
        .arg(manifest_path);
    if let Some(dir) = manifest_path.parent() {
        cmd.current_dir(dir);
    }
    let output = cmd.output().context("Failed to invoke `cargo metadata`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("`cargo metadata` failed:\n{stderr}");
    }

    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("Failed to parse `cargo metadata` output")?;

    let packages = metadata
        .get("packages")
        .and_then(|p| p.as_array())
        .context("`cargo metadata` output missing `packages`")?;

    let member = packages
        .iter()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(package))
        .with_context(|| {
            format!(
                "Package `{package}` not found in workspace at {}",
                manifest_path.display()
            )
        })?;

    let member_manifest = member
        .get("manifest_path")
        .and_then(|m| m.as_str())
        .map(PathBuf::from)
        .with_context(|| format!("Package `{package}` has no manifest_path"))?;

    let workspace_root = metadata
        .get("workspace_root")
        .and_then(|w| w.as_str())
        .map(PathBuf::from);

    Ok((member_manifest, workspace_root))
}

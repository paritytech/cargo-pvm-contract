use anyhow::{Context, Result};
use cargo_pvm_contract_builder as builder;
use clap::Args;
use std::path::{Path, PathBuf};

#[derive(Args, Debug)]
pub struct BuildArgs {
    /// Path to Cargo.toml
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    /// Build profile (default: release)
    #[arg(long)]
    profile: Option<String>,

    /// Output directory for .polkavm and .abi.json files
    #[arg(short = 'o', long = "output-dir")]
    output_dir: Option<PathBuf>,

    /// Cargo message format
    #[arg(long)]
    message_format: Option<String>,
}

pub fn build_contracts(args: BuildArgs) -> Result<()> {
    let manifest_path = match args.manifest_path {
        Some(path) => path,
        None => std::env::current_dir()
            .context("Failed to determine current working directory")?
            .join("Cargo.toml"),
    };

    let manifest_path = manifest_path
        .canonicalize()
        .with_context(|| format!("Manifest not found: {}", manifest_path.display()))?;

    let profile_name = args.profile.as_deref().unwrap_or("release");
    let profile = builder::Profile::from_name(profile_name);

    let bins = builder::get_bin_targets(&manifest_path)?;
    if bins.is_empty() {
        anyhow::bail!("No binary targets found in {}", manifest_path.display());
    }

    let output_dir = args
        .output_dir
        .unwrap_or_else(|| find_target_dir(&manifest_path));

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
    )?;

    Ok(())
}

fn find_target_dir(manifest_path: &Path) -> PathBuf {
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(dir);
    }
    manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("target")
}

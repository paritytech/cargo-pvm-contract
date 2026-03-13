use anyhow::{Context, Result};
use cargo_pvm_contract_builder as builder;
use clap::Parser;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
pub struct BuildArgs {
    /// Path to Cargo.toml
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    /// Packages to build
    #[arg(short = 'p', long = "package")]
    packages: Vec<String>,

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
    let manifest_path = args
        .manifest_path
        .unwrap_or_else(|| std::env::current_dir().unwrap().join("Cargo.toml"));

    let manifest_path = manifest_path
        .canonicalize()
        .with_context(|| format!("Manifest not found: {}", manifest_path.display()))?;

    let profile_name = args.profile.as_deref().unwrap_or("release");
    let profile = builder::Profile::from_name(profile_name);

    let packages = if args.packages.is_empty() {
        vec![builder::get_package_name(&manifest_path)?]
    } else {
        args.packages
    };

    let output_dir = args
        .output_dir
        .unwrap_or_else(|| find_target_dir(&manifest_path));

    std::fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "Failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    for _pkg in &packages {
        let bins = resolve_bins_for_package(&manifest_path)?;
        builder::build_contract(
            &manifest_path,
            &output_dir,
            &profile,
            &bins,
            args.message_format.as_deref(),
        )?;
    }

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

fn resolve_bins_for_package(manifest_path: &Path) -> Result<Vec<String>> {
    builder::get_bin_targets(manifest_path)
}

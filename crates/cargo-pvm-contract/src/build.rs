use anyhow::{Context, Result};
use cargo_pvm_contract_builder::{build_contract, get_bin_targets, get_package_name};
use std::{env, path::{Path, PathBuf}, process::Command};

#[derive(clap::Parser, Debug)]
pub struct BuildArgs {
    /// Path to Cargo.toml
    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    /// Package(s) to build (for workspaces)
    #[arg(short, long)]
    pub packages: Option<Vec<String>>,

    /// Build in release mode (default)
    #[arg(long, default_value_t = true)]
    pub release: bool,

    /// Build profile
    #[arg(long)]
    pub profile: Option<String>,

    /// Output directory for .polkavm and .abi.json files
    #[arg(short, long)]
    pub output_dir: Option<PathBuf>,

    /// Forward --message-format to inner cargo build (e.g. "json")
    #[arg(long)]
    pub message_format: Option<String>,
}

pub fn build_contracts(args: &BuildArgs) -> Result<()> {
    let manifest_path = args
        .manifest_path
        .clone()
        .unwrap_or_else(|| env::current_dir().unwrap().join("Cargo.toml"));

    if !manifest_path.exists() {
        anyhow::bail!("Cargo.toml not found at: {}", manifest_path.display());
    }

    let profile = if args.release || args.profile.is_none() {
        "release"
    } else {
        args.profile.as_deref().unwrap()
    };

    let packages = if let Some(ref pkgs) = args.packages {
        pkgs.clone()
    } else {
        vec![get_package_name(&manifest_path)?]
    };

    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| find_target_dir(&manifest_path));

    // If --message-format is set, emit total crate count upfront via cargo metadata
    if args.message_format.is_some() {
        if let Ok(total) = get_crate_count(&manifest_path) {
            println!("{{\"reason\":\"build-plan\",\"total\":{total}}}");
        }
    }

    for pkg_name in &packages {
        eprintln!("Building contract: {pkg_name}");

        let bins = resolve_bins_for_package(&manifest_path, pkg_name)?;
        build_contract(&manifest_path, &output_dir, profile, pkg_name, bins, args.message_format.as_deref())?;
    }

    Ok(())
}

/// Get total crate count from cargo metadata.
fn get_crate_count(manifest_path: &Path) -> Result<usize> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .arg("metadata")
        .arg("--format-version=1")
        .arg("--manifest-path")
        .arg(manifest_path)
        .output()
        .context("Failed to run cargo metadata")?;
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("Failed to parse cargo metadata")?;
    let count = meta["packages"].as_array().map(|a| a.len()).unwrap_or(0);
    Ok(count)
}

fn find_target_dir(manifest_path: &Path) -> PathBuf {
    if let Ok(dir) = env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(dir);
    }
    manifest_path.parent().unwrap_or(Path::new(".")).join("target")
}

fn resolve_bins_for_package(manifest_path: &Path, pkg_name: &str) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let doc: toml_edit::DocumentMut = content.parse().context("Failed to parse Cargo.toml")?;

    // If workspace, find the member's Cargo.toml
    if let Some(workspace) = doc.get("workspace") {
        if let Some(members) = workspace.get("members").and_then(|m| m.as_array()) {
            let base = manifest_path.parent().unwrap();
            for member in members {
                if let Some(member_path) = member.as_str() {
                    let paths = if member_path.contains('*') {
                        glob_member_paths(base, member_path)
                    } else {
                        vec![base.join(member_path)]
                    };
                    for path in paths {
                        let member_toml = path.join("Cargo.toml");
                        if member_toml.exists() {
                            if let Ok(name) = get_package_name(&member_toml) {
                                if name == pkg_name {
                                    return get_bin_targets(&member_toml);
                                }
                            }
                        }
                    }
                }
            }
        }
        anyhow::bail!("Package '{pkg_name}' not found in workspace");
    }

    get_bin_targets(manifest_path)
}

fn glob_member_paths(base: &Path, pattern: &str) -> Vec<PathBuf> {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() != 2 {
        return vec![base.join(pattern)];
    }
    let prefix = base.join(parts[0]);
    let suffix = parts[1];
    let Ok(entries) = std::fs::read_dir(&prefix) else {
        return vec![];
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let mut p = e.path();
            if !suffix.is_empty() {
                p = p.join(suffix.trim_start_matches('/'));
            }
            p
        })
        .collect()
}

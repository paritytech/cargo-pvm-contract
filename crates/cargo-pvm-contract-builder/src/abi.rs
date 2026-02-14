use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::Path, process::Command};
use toml_edit::DocumentMut;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct AbiJson(Vec<AbiItem>);

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AbiItem {
    Function {
        name: String,
        inputs: Vec<AbiParam>,
        outputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        #[serde(skip_serializing_if = "Option::is_none")]
        state_mutability: Option<String>,
    },
    Constructor {
        inputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        #[serde(skip_serializing_if = "Option::is_none")]
        state_mutability: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct AbiParam {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
}

pub fn generate_abi_for_bin(manifest_dir: &Path, bin_name: &str) -> Result<Option<AbiJson>> {
    generate_abi_via_feature(manifest_dir, bin_name)
}

fn generate_abi_via_feature(manifest_dir: &Path, bin_name: &str) -> Result<Option<AbiJson>> {
    let source_path = resolve_bin_source_path(manifest_dir, bin_name)?;
    if !source_path.exists() {
        return Ok(None);
    }

    let source_content = fs::read_to_string(&source_path)
        .with_context(|| format!("Failed to read {}", source_path.display()))?;

    if let Some(sol_path) = extract_sol_path_from_source(&source_content) {
        let sol_full_path = manifest_dir.join(sol_path);
        return generate_abi_from_sol(&sol_full_path);
    }

    let target_dir = super::get_target_root().join("abi-gen-target");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let manifest_path = manifest_dir.join("Cargo.toml");

    // The project's .cargo/config.toml targets RISC-V with build-std=core,alloc.
    // The abi-gen binary needs std and must run on the host, so we override both:
    // --target forces the host triple, build-std adds std to the sysroot rebuild.
    let host = env::var("HOST")
        .context("HOST env var not set — generate_abi_via_feature must run from build.rs")?;

    let output = Command::new(&cargo)
        .current_dir(manifest_dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env("CARGO_TARGET_DIR", &target_dir)
        .env(super::INTERNAL_BUILD_ENV, "1")
        .arg("run")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--target")
        .arg(&host)
        .arg("--config")
        .arg(r#"unstable.build-std=["std","core","alloc"]"#)
        .arg("--features")
        .arg("abi-gen")
        .arg("--bin")
        .arg(bin_name)
        .output()
        .context("Failed to execute abi-gen compile and run")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ABI generation via abi-gen feature failed:\n{stderr}");
    }

    let stdout_str =
        String::from_utf8(output.stdout).context("ABI generation output is not valid UTF-8")?;

    let abi: AbiJson = serde_json::from_str(&stdout_str)
        .context("Failed to parse ABI JSON from abi-gen output")?;

    Ok(Some(abi))
}

fn extract_sol_path_from_source(source: &str) -> Option<String> {
    if let Some(start) = source.find("contract(\"") {
        let after_quote = &source[start + 10..];
        if let Some(end) = after_quote.find('"') {
            let path = &after_quote[..end];
            if path.ends_with(".sol") {
                return Some(path.to_string());
            }
        }
    }
    None
}

fn resolve_bin_source_path(manifest_dir: &Path, bin_name: &str) -> Result<std::path::PathBuf> {
    let cargo_toml_path = manifest_dir.join("Cargo.toml");
    let cargo_toml = std::fs::read_to_string(&cargo_toml_path)
        .with_context(|| format!("Failed to read {}", cargo_toml_path.display()))?;
    let doc = cargo_toml
        .parse::<DocumentMut>()
        .context("Failed to parse Cargo.toml")?;

    if let Some(bin_array) = doc.get("bin").and_then(|b| b.as_array_of_tables()) {
        for bin in bin_array {
            if bin.get("name").and_then(|n| n.as_str()) == Some(bin_name) {
                if let Some(path) = bin.get("path").and_then(|p| p.as_str()) {
                    return Ok(manifest_dir.join(path));
                }
                return Ok(manifest_dir.join("src/bin").join(format!("{bin_name}.rs")));
            }
        }
    }

    if doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        == Some(bin_name)
    {
        return Ok(manifest_dir.join("src/main.rs"));
    }

    Ok(manifest_dir.join("src/bin").join(format!("{bin_name}.rs")))
}

fn generate_abi_from_sol(sol_path: &Path) -> Result<Option<AbiJson>> {
    let content = std::fs::read_to_string(sol_path)
        .with_context(|| format!("Failed to read sol file: {}", sol_path.display()))?;

    let mut items = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("function ")
            && let Some(func) = parse_sol_function_line(line)
        {
            items.push(func);
        }
    }

    if items.is_empty() {
        return Ok(None);
    }

    Ok(Some(AbiJson(items)))
}

fn parse_sol_function_line(line: &str) -> Option<AbiItem> {
    let line = line.strip_prefix("function ")?.trim();

    let paren_start = line.find('(')?;
    let name = line[..paren_start].trim().to_string();

    let paren_end = line.find(')')?;
    let params_str = &line[paren_start + 1..paren_end];
    let inputs = parse_sol_params(params_str);

    let outputs = if let Some(returns_idx) = line.find("returns") {
        let after_returns = &line[returns_idx + 7..];
        if let Some(start) = after_returns.find('(') {
            if let Some(end) = after_returns.find(')') {
                parse_sol_params(&after_returns[start + 1..end])
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let state_mutability = if line.contains(" view ") || line.contains(" view)") {
        "view"
    } else if line.contains(" pure ") || line.contains(" pure)") {
        "pure"
    } else if line.contains(" payable ") || line.contains(" payable)") {
        "payable"
    } else {
        "nonpayable"
    }
    .to_string();

    Some(AbiItem::Function {
        name,
        inputs,
        outputs,
        state_mutability: Some(state_mutability),
    })
}

fn parse_sol_params(params_str: &str) -> Vec<AbiParam> {
    if params_str.trim().is_empty() {
        return vec![];
    }

    params_str
        .split(',')
        .filter_map(|p| {
            let p = p.trim();
            let parts: Vec<&str> = p.split_whitespace().collect();
            if parts.is_empty() {
                return None;
            }
            let param_type = parts[0].to_string();
            let name = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
            Some(AbiParam { name, param_type })
        })
        .collect()
}

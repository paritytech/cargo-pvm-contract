use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;
use std::process::Command;

#[derive(Args, Debug)]
pub struct TestArgs {
    /// Path to Cargo.toml
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    /// Features to forward to `cargo test --features`
    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    /// Arguments forwarded to `cargo test` after `--`
    #[arg(trailing_var_arg = true)]
    extra: Vec<String>,
}

pub fn run_tests(args: TestArgs) -> Result<()> {
    let manifest_path = match args.manifest_path {
        Some(path) => path,
        None => std::env::current_dir()
            .context("Failed to determine current working directory")?
            .join("Cargo.toml"),
    };
    let manifest_path = manifest_path
        .canonicalize()
        .with_context(|| format!("Manifest not found: {}", manifest_path.display()))?;

    // Contracts ship a `.cargo/config.toml` that forces the polkavm target;
    // unit tests must run on the host. We override by passing `--target`
    // explicitly, plus unset `CARGO_BUILD_TARGET` in case an env picked it up.
    // `RUSTFLAGS` is left alone — users may need it for coverage, sanitizers,
    // `--cap-lints`, or similar. If a user sets `RUSTFLAGS=-Ctarget-feature=...`
    // targeting polkavm, passing `--target <host>` still takes precedence.
    let host_target = host_target_triple()?;

    let mut cmd = Command::new(env!("CARGO"));
    cmd.arg("test")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--target")
        .arg(&host_target)
        .env_remove("CARGO_BUILD_TARGET");

    if !args.features.is_empty() {
        cmd.arg("--features").arg(args.features.join(","));
    }

    if !args.extra.is_empty() {
        cmd.arg("--").args(&args.extra);
    }

    let status = cmd.status().with_context(|| {
        format!(
            "failed to spawn `cargo test` for {}",
            manifest_path.display()
        )
    })?;

    if !status.success() {
        anyhow::bail!("cargo test failed with status {status}");
    }
    Ok(())
}

fn host_target_triple() -> Result<String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("failed to invoke rustc to determine host target")?;
    let info =
        std::str::from_utf8(&output.stdout).context("rustc -vV produced non-UTF-8 output")?;
    for line in info.lines() {
        if let Some(rest) = line.strip_prefix("host: ") {
            return Ok(rest.trim().to_string());
        }
    }
    anyhow::bail!("could not parse host triple from `rustc -vV`")
}

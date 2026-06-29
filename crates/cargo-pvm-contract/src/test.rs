use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
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

    // Run the test build from the project directory so rustup resolves the
    // project's pinned toolchain (`rust-toolchain.toml` / inherited
    // `RUSTUP_TOOLCHAIN`). `manifest_path` is already canonical, so it stays
    // valid regardless of the working directory.
    let project_dir = manifest_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("manifest path has no parent directory: {manifest_path:?}"))?
        .to_path_buf();

    // Invoke `cargo` from PATH (the rustup proxy), not `env!("CARGO")`: the
    // latter bakes in the absolute path of whatever toolchain built this CLI
    // and bypasses the proxy, so the project's `rust-toolchain.toml` would be
    // ignored. The bare proxy honors it, matching the `build` subcommand.
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&project_dir)
        .arg("test")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--target")
        .arg(&host_target)
        .env_remove("CARGO_BUILD_TARGET");

    // Scaffolded `.cargo/config.toml` forces `-Zbuild-std=core,alloc` for the
    // polkavm target. Host unit tests link the real prebuilt `std` (the crate
    // is not `no_std` under `cfg(test)`), so a build-std `alloc` collides with
    // std's `alloc` (duplicate `exchange_malloc` lang item). Override it with
    // an empty `-Zbuild-std=` — but only on a nightly cargo: stable already
    // ignores the config's build-std, and rejects the `-Z` flag outright.
    if cargo_is_nightly(&project_dir) {
        cmd.arg("-Zbuild-std=");
    }

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

/// Whether the `cargo` resolved from `project_dir` is a nightly-channel cargo.
/// Detected by parsing `cargo --version` (nightly prints e.g.
/// `cargo 1.92.0-nightly (...)`). Defaults to `false` (don't pass `-Z`) if the
/// probe fails — the safe choice, since stable rejects `-Z` and ignores the
/// scaffold's build-std config anyway.
fn cargo_is_nightly(project_dir: &Path) -> bool {
    Command::new("cargo")
        .current_dir(project_dir)
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .is_some_and(|version| version.contains("nightly"))
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

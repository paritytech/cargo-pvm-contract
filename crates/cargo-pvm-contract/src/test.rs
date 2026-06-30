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
    // and bypasses the proxy. Run from `project_dir` so the proxy honors the
    // project's `rust-toolchain.toml` by default — but, unlike `build` (which
    // *requires* the pinned nightly for `-Zbuild-std=core,alloc`), we do NOT
    // force the pin: host unit tests run on the prebuilt host `std` and aren't
    // bound to the build toolchain, so an explicit `RUSTUP_TOOLCHAIN` override
    // is respected.
    //
    //   - `RUSTC_BOOTSTRAP=1` unlocks `-Z` on any channel, so this works
    //     whether the resolved toolchain is stable, beta, or nightly.
    //   - `-Zbuild-std=` (empty) overrides the scaffold's `.cargo/config.toml`
    //     `-Zbuild-std=core,alloc`. Host unit tests link the real prebuilt
    //     `std` (the crate is not `no_std` under `cfg(test)`), so a build-std
    //     `alloc` would collide with std's `alloc` (duplicate `exchange_malloc`
    //     lang item).
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&project_dir)
        .arg("test")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--target")
        .arg(&host_target)
        .arg("-Zbuild-std=")
        .env("RUSTC_BOOTSTRAP", "1")
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

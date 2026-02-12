use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    NoAlloc,
    WithAlloc,
    Alloy,
    BuilderDsl,
}

impl Variant {
    fn name(&self) -> &'static str {
        match self {
            Variant::NoAlloc => "no-alloc",
            Variant::WithAlloc => "with-alloc",
            Variant::Alloy => "alloy",
            Variant::BuilderDsl => "builder-dsl",
        }
    }

    fn cargo_toml(&self, contract: &str, base_path: &Path) -> String {
        match self {
            Variant::NoAlloc => cargo_toml_no_alloc(contract, base_path),
            Variant::WithAlloc => cargo_toml_with_alloc(contract, base_path),
            Variant::Alloy => cargo_toml_alloy(contract, base_path),
            Variant::BuilderDsl => cargo_toml_builder_dsl(contract, base_path),
        }
    }
}

fn cargo_toml_no_alloc(contract: &str, base_path: &Path) -> String {
    let macros_path = base_path.join("crates/pvm-contract-macros");
    let types_path = base_path.join("crates/pvm-contract-types");
    let builder_path = base_path.join("crates/cargo-pvm-contract-builder");

    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"
rust-version = "1.92"
build = "build.rs"

[[bin]]
name = "{}"
path = "src/{}.rs"

[dependencies]
pvm-contract-macros = {{ path = "{}" }}
pvm-contract-types = {{ path = "{}" }}
pallet-revive-uapi = {{ version = "0.10", default-features = false }}
polkavm-derive = {{ version = "0.31.0" }}
ruint = {{ version = "1.17", default-features = false }}

[build-dependencies]
cargo-pvm-contract-builder = {{ path = "{}" }}

[profile.dev]
panic = "abort"

[profile.release]
codegen-units = 1
lto = true
opt-level = "z"
panic = "abort"
overflow-checks = false
"#,
        contract,
        contract,
        contract,
        macros_path.display(),
        types_path.display(),
        builder_path.display()
    )
}

fn cargo_toml_with_alloc(contract: &str, base_path: &Path) -> String {
    let macros_path = base_path.join("crates/pvm-contract-macros");
    let types_path = base_path.join("crates/pvm-contract-types");
    let builder_path = base_path.join("crates/cargo-pvm-contract-builder");

    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"
rust-version = "1.92"
build = "build.rs"

[[bin]]
name = "{}"
path = "src/{}.rs"

[dependencies]
pvm-contract-macros = {{ path = "{}" }}
pvm-contract-types = {{ path = "{}" }}
pallet-revive-uapi = {{ version = "0.10", default-features = false }}
polkavm-derive = {{ version = "0.31.0" }}
ruint = {{ version = "1.17", default-features = false }}
picoalloc = {{ version = "5", default-features = false }}

[build-dependencies]
cargo-pvm-contract-builder = {{ path = "{}" }}

[profile.dev]
panic = "abort"

[profile.release]
codegen-units = 1
lto = true
opt-level = "z"
panic = "abort"
overflow-checks = false
"#,
        contract,
        contract,
        contract,
        macros_path.display(),
        types_path.display(),
        builder_path.display()
    )
}

fn cargo_toml_alloy(contract: &str, base_path: &Path) -> String {
    let builder_path = base_path.join("crates/cargo-pvm-contract-builder");

    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"
rust-version = "1.92"
build = "build.rs"

[[bin]]
name = "{}"
path = "src/{}.rs"

[dependencies]
alloy-core = {{ version = "1.5", default-features = false, features = ["sol-types"] }}
pallet-revive-uapi = {{ version = "0.10", default-features = false }}
polkavm-derive = {{ version = "0.31.0" }}
picoalloc = {{ version = "5", default-features = false }}

[build-dependencies]
cargo-pvm-contract-builder = {{ path = "{}" }}

[profile.dev]
panic = "abort"

[profile.release]
codegen-units = 1
lto = true
opt-level = "z"
panic = "abort"
overflow-checks = false
"#,
        contract,
        contract,
        contract,
        builder_path.display()
    )
}

fn cargo_toml_builder_dsl(contract: &str, base_path: &Path) -> String {
    let dsl_path = base_path.join("crates/pvm-contract-builder-dsl");
    let types_path = base_path.join("crates/pvm-contract-types");
    let builder_path = base_path.join("crates/cargo-pvm-contract-builder");

    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"
rust-version = "1.92"
build = "build.rs"

[[bin]]
name = "{}"
path = "src/{}.rs"

[dependencies]
pvm-contract-builder-dsl = {{ path = "{}" }}
pvm-contract-types = {{ path = "{}" }}
pallet-revive-uapi = {{ version = "0.10", default-features = false }}
polkavm-derive = {{ version = "0.31.0" }}
ruint = {{ version = "1.17", default-features = false }}

[build-dependencies]
cargo-pvm-contract-builder = {{ path = "{}" }}

[profile.dev]
panic = "abort"

[profile.release]
codegen-units = 1
lto = true
opt-level = "z"
panic = "abort"
overflow-checks = false
"#,
        contract,
        contract,
        contract,
        dsl_path.display(),
        types_path.display(),
        builder_path.display()
    )
}

fn get_source_file(contract: &str, variant: Variant, base_path: &Path) -> Result<String> {
    if variant == Variant::BuilderDsl {
        let examples_dir = base_path.join("crates/pvm-contract-builder-dsl/contracts");
        let source_file = format!("{}_builder.rs", contract);
        let source_path = examples_dir.join(&source_file);
        return fs::read_to_string(&source_path)
            .with_context(|| format!("Failed to read {}", source_path.display()));
    }

    let template_dir = base_path
        .join("crates/cargo-pvm-contract/templates/examples")
        .join(contract);

    let source_file = match variant {
        Variant::NoAlloc => format!("{}_no_alloc.rs", contract),
        Variant::WithAlloc => format!("{}_with_alloc.rs", contract),
        Variant::Alloy => format!("{}_alloy.rs", contract),
        Variant::BuilderDsl => unreachable!(),
    };

    let source_path = template_dir.join(&source_file);
    let mut content = fs::read_to_string(&source_path)
        .with_context(|| format!("Failed to read {}", source_path.display()))?;

    if variant == Variant::Alloy {
        content = content.replace("abi_decode(input, true)", "abi_decode(input)");
    }

    Ok(content)
}

fn build_variant(
    contract: &str,
    variant: Variant,
    profile: &str,
    artifacts_dir: &Path,
    base_path: &Path,
) -> Result<()> {
    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    let temp_path = temp_dir.path();

    let src_dir = temp_path.join("src");
    fs::create_dir(&src_dir).context("Failed to create src directory")?;

    let source_content = get_source_file(contract, variant, base_path)?;
    let dest_source = src_dir.join(format!("{}.rs", contract));
    fs::write(&dest_source, source_content).context("Failed to write contract source")?;

    let template_dir = base_path
        .join("crates/cargo-pvm-contract/templates/examples")
        .join(contract);

    for entry in fs::read_dir(&template_dir)
        .with_context(|| format!("Failed to read template dir {}", template_dir.display()))?
    {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "sol") {
            let file_name = path.file_name().unwrap();
            let dest = temp_path.join(file_name);
            fs::copy(&path, &dest).with_context(|| format!("Failed to copy {}", path.display()))?;
        }
    }

    let cargo_toml_content = variant.cargo_toml(contract, base_path);
    let cargo_toml_path = temp_path.join("Cargo.toml");
    fs::write(&cargo_toml_path, cargo_toml_content).context("Failed to write Cargo.toml")?;

    let build_rs = r#"fn main() {
    cargo_pvm_contract_builder::PvmBuilder::new().build();
}
"#;
    fs::write(temp_path.join("build.rs"), build_rs).context("Failed to write build.rs")?;

    let cargo_profile = if profile == "debug" { "dev" } else { profile };

    let mut args = polkavm_linker::TargetJsonArgs::default();
    args.is_64_bit = true;
    let target_json = polkavm_linker::target_json_path(args)
        .map_err(|e| anyhow::anyhow!("Failed to get target JSON: {e}"))?;

    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(temp_path)
        .arg("build")
        .arg("--profile")
        .arg(cargo_profile)
        .arg("--target")
        .arg(&target_json)
        .arg("-Zbuild-std=core,alloc");

    let output = cmd.output().context("Failed to execute cargo build")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Build failed for {} {} ({}): {}",
            contract,
            variant.name(),
            profile,
            stderr
        );
    }

    let polkavm_file = temp_path
        .join("target")
        .join(format!("{}.{}.polkavm", contract, profile));

    if !polkavm_file.exists() {
        let target_dir = temp_path.join("target");
        let files: Vec<_> = fs::read_dir(&target_dir)
            .ok()
            .and_then(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .find(|p| p.extension().is_some_and(|ext| ext == "polkavm"))
            })
            .into_iter()
            .collect();

        anyhow::bail!(
            "PolkaVM file not found at {}. Found files: {:?}",
            polkavm_file.display(),
            files
        );
    }

    let output_name = format!("{}_{}.{}.polkavm", contract, variant.name(), profile);
    let output_path = artifacts_dir.join(&output_name);

    fs::copy(&polkavm_file, &output_path).with_context(|| {
        format!(
            "Failed to copy {} to {}",
            polkavm_file.display(),
            output_path.display()
        )
    })?;

    println!(
        "✓ Built {} {} ({}): {}",
        contract,
        variant.name(),
        profile,
        output_path.display()
    );

    Ok(())
}

fn variants_for_contract(contract: &str) -> Vec<Variant> {
    match contract {
        "multi" => vec![Variant::NoAlloc, Variant::BuilderDsl],
        _ => vec![
            Variant::NoAlloc,
            Variant::WithAlloc,
            Variant::Alloy,
            Variant::BuilderDsl,
        ],
    }
}

fn main() -> Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let base_path = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let artifacts_dir = PathBuf::from("target/benchmark-artifacts");
    fs::create_dir_all(&artifacts_dir).context("Failed to create artifacts directory")?;

    let contracts = vec!["fibonacci", "mytoken", "multi"];
    let profiles = vec!["debug", "release"];

    let total: usize = contracts
        .iter()
        .map(|contract| variants_for_contract(contract).len() * profiles.len())
        .sum();
    let mut count = 0;

    for contract in &contracts {
        for variant in variants_for_contract(contract) {
            for profile in &profiles {
                count += 1;
                println!(
                    "\n[{}/{}] Building {} {} ({})",
                    count,
                    total,
                    contract,
                    variant.name(),
                    profile
                );
                build_variant(contract, variant, profile, &artifacts_dir, &base_path)?;
            }
        }
    }

    println!("\n✓ All builds completed successfully!");
    println!("Artifacts saved to: {}", artifacts_dir.display());

    let results_dir = PathBuf::from("target/benchmark-results");
    fs::create_dir_all(&results_dir).context("Failed to create results directory")?;

    let variants = pvm_contract_benchmarks::collect_variants(&artifacts_dir)
        .context("Failed to collect variants")?;

    if variants.is_empty() {
        anyhow::bail!("No variants found in artifacts directory");
    }

    let report = pvm_contract_benchmarks::generate_report(&variants);

    let report_path = results_dir.join("binary-sizes.md");
    fs::write(&report_path, &report)
        .with_context(|| format!("Failed to write report to {}", report_path.display()))?;

    println!("\n✓ Report generated: {}", report_path.display());
    println!("\n{}", report);

    Ok(())
}

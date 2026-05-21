use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    NoAlloc,
    WithAlloc,
    BuilderDsl,
    Storage,
}

impl Variant {
    fn name(&self) -> &'static str {
        match self {
            Variant::NoAlloc => "no-alloc",
            Variant::WithAlloc => "with-alloc",
            Variant::BuilderDsl => "builder-dsl",
            Variant::Storage => "storage",
        }
    }

    fn cargo_toml(&self, contract: &str, base_path: &Path) -> String {
        match self {
            Variant::NoAlloc | Variant::Storage => cargo_toml_no_alloc(contract, base_path),
            Variant::WithAlloc => cargo_toml_with_alloc(contract, base_path),
            Variant::BuilderDsl => cargo_toml_builder_dsl(contract, base_path),
        }
    }
}

fn cargo_toml_no_alloc(contract: &str, base_path: &Path) -> String {
    let sdk_path = base_path.join("crates/pvm-contract-sdk");

    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"
rust-version = "1.92"

[[bin]]
name = "{}"
path = "src/{}.rs"

[dependencies]
pvm-contract-sdk = {{ path = "{}" }}
polkavm-derive = {{ version = "0.31.0" }}

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
        sdk_path.display(),
    )
}

fn cargo_toml_with_alloc(contract: &str, base_path: &Path) -> String {
    let sdk_path = base_path.join("crates/pvm-contract-sdk");
    let bump_alloc_path = base_path.join("crates/pvm-bump-allocator");

    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"
rust-version = "1.92"

[[bin]]
name = "{}"
path = "src/{}.rs"

[dependencies]
pvm-contract-sdk = {{ path = "{}" }}
pvm-bump-allocator = {{ path = "{}" }}
polkavm-derive = {{ version = "0.31.0" }}

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
        sdk_path.display(),
        bump_alloc_path.display(),
    )
}

fn cargo_toml_builder_dsl(contract: &str, base_path: &Path) -> String {
    let dsl_path = base_path.join("crates/pvm-contract-builder-dsl");

    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"
rust-version = "1.92"

[[bin]]
name = "{}"
path = "src/{}.rs"

[dependencies]
pvm-contract-builder-dsl = {{ path = "{}" }}
polkavm-derive = {{ version = "0.31.0" }}

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
    )
}

fn get_source_file(contract: &str, variant: Variant, base_path: &Path) -> Result<String> {
    if variant == Variant::BuilderDsl {
        let examples_dir = base_path.join("crates/pvm-contract-builder-dsl/contracts");
        let source_file = format!("{contract}_builder.rs");
        let source_path = examples_dir.join(&source_file);
        return fs::read_to_string(&source_path)
            .with_context(|| format!("Failed to read {}", source_path.display()));
    }

    let template_dir = base_path
        .join("crates/cargo-pvm-contract/templates/examples")
        .join(contract);

    let source_file = match variant {
        Variant::NoAlloc => format!("{contract}_no_alloc.rs"),
        Variant::WithAlloc => format!("{contract}_with_alloc.rs"),
        Variant::Storage => format!("{contract}_storage.rs"),
        Variant::BuilderDsl => unreachable!(),
    };

    let source_path = template_dir.join(&source_file);
    fs::read_to_string(&source_path)
        .with_context(|| format!("Failed to read {}", source_path.display()))
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
    let dest_source = src_dir.join(format!("{contract}.rs"));
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

    let build_profile = cargo_pvm_contract_builder::Profile::from_name(profile);
    let output_dir = temp_path.join("target");
    let bins = vec![contract.to_string()];

    cargo_pvm_contract_builder::build_contract(
        &cargo_toml_path,
        &output_dir,
        &build_profile,
        &bins,
        None,
        None,
        false,
    )
    .with_context(|| {
        format!(
            "Build failed for {} {} ({})",
            contract,
            variant.name(),
            profile,
        )
    })?;

    let polkavm_file = output_dir.join(profile).join(format!("{contract}.polkavm"));

    if !polkavm_file.exists() {
        anyhow::bail!("PolkaVM file not found at {}", polkavm_file.display(),);
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
    let mut variants = vec![Variant::NoAlloc, Variant::WithAlloc, Variant::BuilderDsl];
    if contract == "mytoken" {
        variants.push(Variant::Storage);
    }
    variants
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
    println!("\n{report}");

    Ok(())
}

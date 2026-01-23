use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn scaffold_example_with_pvm_contract(
    temp_dir: &TempDir,
    name: &str,
    memory_model: &str,
    pvm_contract_path: &Path,
) -> PathBuf {
    let builder_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../cargo-pvm-contract-builder");
    let project_dir = temp_dir.path().join(name);
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_BUILDER_PATH", &builder_path)
        .env("CARGO_PVM_CONTRACT_PATH", pvm_contract_path)
        .arg("pvm-contract")
        .arg("--init-type")
        .arg("example")
        .arg("--example")
        .arg("MyToken")
        .arg("--memory-model")
        .arg(memory_model)
        .arg("--name")
        .arg(name)
        .assert()
        .success();

    project_dir
}

fn build_scaffolded_project(project_dir: &Path) {
    let status = std::process::Command::new("cargo")
        .current_dir(project_dir)
        // Remove env vars that override rust-toolchain.toml
        .env_remove("CARGO")
        .env_remove("RUSTUP_TOOLCHAIN")
        .arg("build")
        .status()
        .expect("run cargo build");

    assert!(status.success(), "cargo build failed");
}

#[test]
fn scaffold_mytoken_alloc() {
    let temp_dir = TempDir::new().expect("temp dir");
    let pvm_contract_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../pvm_contract");
    let project_dir = scaffold_example_with_pvm_contract(
        &temp_dir,
        "mytoken-alloc",
        "alloc-with-alloy",
        &pvm_contract_path,
    );

    let cargo_toml =
        std::fs::read_to_string(project_dir.join("Cargo.toml")).expect("Cargo.toml exists");

    assert!(cargo_toml.contains("pvm_contract"));
    assert!(cargo_toml.contains("polkavm-derive"));
    assert!(cargo_toml.contains("picoalloc"));

    build_scaffolded_project(&project_dir);
}

#[test]
fn scaffold_mytoken_no_alloc() {
    let temp_dir = TempDir::new().expect("temp dir");
    let pvm_contract_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../pvm_contract");
    let project_dir = scaffold_example_with_pvm_contract(
        &temp_dir,
        "mytoken-no-alloc",
        "no-alloc",
        &pvm_contract_path,
    );

    let cargo_toml =
        std::fs::read_to_string(project_dir.join("Cargo.toml")).expect("Cargo.toml exists");

    assert!(cargo_toml.contains("pvm_contract"));
    assert!(cargo_toml.contains("polkavm-derive"));
    assert!(cargo_toml.contains("picoalloc"));

    build_scaffolded_project(&project_dir);
}

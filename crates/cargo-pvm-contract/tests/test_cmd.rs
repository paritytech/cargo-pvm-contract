use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn workspace_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scaffold_example(temp_dir: &TempDir, name: &str, example: &str, memory_model: &str) -> PathBuf {
    let project_dir = temp_dir.path().join(name);
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_PATH", workspace_path())
        .arg("pvm-contract")
        .arg("--init-type")
        .arg("example")
        .arg("--example")
        .arg(example)
        .arg("--memory-model")
        .arg(memory_model)
        .arg("--name")
        .arg(name)
        .assert()
        .success();

    project_dir
}

fn scaffold_new_contract(temp_dir: &TempDir, name: &str, memory_model: &str) -> PathBuf {
    let project_dir = temp_dir.path().join(name);
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_PATH", workspace_path())
        .arg("pvm-contract")
        .arg("--init-type")
        .arg("new")
        .arg("--memory-model")
        .arg(memory_model)
        .arg("--name")
        .arg(name)
        .assert()
        .success();

    project_dir
}

fn build_project(project_dir: &Path, profile: &str) {
    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(project_dir)
        .env_remove("CARGO")
        .env_remove("RUSTUP_TOOLCHAIN")
        .arg("build");

    if profile == "release" {
        cmd.arg("--release");
    }

    let status = cmd.status().expect("run cargo build");
    assert!(status.success(), "cargo build ({}) failed", profile);
}

fn verify_build_artifacts(project_dir: &Path, binary_name: &str, profile: &str) {
    let target_dir = project_dir.join("target");

    let polkavm_file = target_dir.join(format!("{}.{}.polkavm", binary_name, profile));
    assert!(
        polkavm_file.exists(),
        "PolkaVM binary not found: {}",
        polkavm_file.display()
    );

    let abi_file = target_dir.join(format!("{}.{}.abi.json", binary_name, profile));
    assert!(
        abi_file.exists(),
        "ABI JSON not found: {}",
        abi_file.display()
    );

    let abi_content = std::fs::read_to_string(&abi_file).expect("read ABI file");
    let abi: serde_json::Value = serde_json::from_str(&abi_content).expect("parse ABI JSON");
    assert!(abi.is_array(), "ABI should be an array");
}

fn verify_cargo_toml(project_dir: &Path, use_alloc: bool) {
    let cargo_toml =
        std::fs::read_to_string(project_dir.join("Cargo.toml")).expect("Cargo.toml exists");

    assert!(cargo_toml.contains("pvm-contract-macros"));
    assert!(cargo_toml.contains("polkavm-derive"));
    assert!(cargo_toml.contains("pallet-revive-uapi"));
    assert!(cargo_toml.contains("ruint"));

    if use_alloc {
        assert!(cargo_toml.contains("picoalloc"));
    } else {
        assert!(!cargo_toml.contains("picoalloc"));
    }
}

#[test]
fn mytoken_alloc_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(
        &temp_dir,
        "mytoken-alloc-debug",
        "MyToken",
        "alloc-with-alloy",
    );

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "mytoken-alloc-debug", "debug");
}

#[test]
fn mytoken_alloc_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(
        &temp_dir,
        "mytoken-alloc-release",
        "MyToken",
        "alloc-with-alloy",
    );

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "mytoken-alloc-release", "release");
}

#[test]
fn mytoken_no_alloc_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "mytoken-no-alloc-debug", "MyToken", "no-alloc");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "mytoken-no-alloc-debug", "debug");
}

#[test]
fn mytoken_no_alloc_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir =
        scaffold_example(&temp_dir, "mytoken-no-alloc-release", "MyToken", "no-alloc");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "mytoken-no-alloc-release", "release");
}

#[test]
fn fibonacci_alloc_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(
        &temp_dir,
        "fibonacci-alloc-debug",
        "Fibonacci",
        "alloc-with-alloy",
    );

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "fibonacci-alloc-debug", "debug");
}

#[test]
fn fibonacci_alloc_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(
        &temp_dir,
        "fibonacci-alloc-release",
        "Fibonacci",
        "alloc-with-alloy",
    );

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "fibonacci-alloc-release", "release");
}

#[test]
fn fibonacci_no_alloc_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(
        &temp_dir,
        "fibonacci-no-alloc-debug",
        "Fibonacci",
        "no-alloc",
    );

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "fibonacci-no-alloc-debug", "debug");
}

#[test]
fn fibonacci_no_alloc_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(
        &temp_dir,
        "fibonacci-no-alloc-release",
        "Fibonacci",
        "no-alloc",
    );

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "fibonacci-no-alloc-release", "release");
}

#[test]
fn new_contract_alloc_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-alloc-debug", "alloc-with-alloy");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "new-alloc-debug", "debug");
}

#[test]
fn new_contract_alloc_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-alloc-release", "alloc-with-alloy");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "new-alloc-release", "release");
}

#[test]
fn new_contract_no_alloc_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-no-alloc-debug", "no-alloc");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "new-no-alloc-debug", "debug");
}

#[test]
fn new_contract_no_alloc_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-no-alloc-release", "no-alloc");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "new-no-alloc-release", "release");
}

#[test]
fn abi_json_has_correct_structure() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "abi-test", "MyToken", "no-alloc");

    build_project(&project_dir, "debug");

    let abi_file = project_dir.join("target").join("abi-test.debug.abi.json");
    let abi_content = std::fs::read_to_string(&abi_file).expect("read ABI file");
    let abi: Vec<serde_json::Value> = serde_json::from_str(&abi_content).expect("parse ABI JSON");

    let function_names: Vec<&str> = abi
        .iter()
        .filter(|entry| entry.get("type").and_then(|t| t.as_str()) == Some("function"))
        .filter_map(|entry| entry.get("name").and_then(|n| n.as_str()))
        .collect();

    assert!(
        function_names.contains(&"totalSupply"),
        "ABI should contain totalSupply"
    );
    assert!(
        function_names.contains(&"balanceOf"),
        "ABI should contain balanceOf"
    );
    assert!(
        function_names.contains(&"transfer"),
        "ABI should contain transfer"
    );
    assert!(function_names.contains(&"mint"), "ABI should contain mint");
}

#[test]
fn polkavm_binary_is_valid() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "polkavm-test", "Fibonacci", "no-alloc");

    build_project(&project_dir, "release");

    let polkavm_file = project_dir
        .join("target")
        .join("polkavm-test.release.polkavm");
    let binary = std::fs::read(&polkavm_file).expect("read polkavm file");

    assert!(!binary.is_empty(), "PolkaVM binary should not be empty");
    assert!(
        binary.len() < 100_000,
        "Release binary should be reasonably small (got {} bytes)",
        binary.len()
    );
}

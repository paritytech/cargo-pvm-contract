use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn workspace_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scaffold_example(temp_dir: &TempDir, name: &str, example: &str, api_style: &str) -> PathBuf {
    let project_dir = temp_dir.path().join(name);
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_PATH", workspace_path())
        .arg("pvm-contract")
        .arg("init")
        .arg("--init-type")
        .arg("example")
        .arg("--example")
        .arg(example)
        .arg("--api-style")
        .arg(api_style)
        .arg("--name")
        .arg(name)
        .assert()
        .success();

    project_dir
}

fn scaffold_new_contract(
    temp_dir: &TempDir,
    name: &str,
    api_style: &str,
    allocator: Option<&str>,
) -> PathBuf {
    let project_dir = temp_dir.path().join(name);
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_PATH", workspace_path())
        .arg("pvm-contract")
        .arg("init")
        .arg("--init-type")
        .arg("new")
        .arg("--api-style")
        .arg(api_style)
        .arg("--name")
        .arg(name);

    if let Some(alloc) = allocator {
        cmd.arg("--allocator").arg(alloc);
    }

    cmd.assert().success();

    project_dir
}

fn build_project(project_dir: &Path, profile: &str) {
    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(project_dir)
        .arg("pvm-contract")
        .arg("build");

    if profile == "debug" {
        cmd.arg("--profile").arg("dev");
    }

    let status = cmd.status().expect("run cargo pvm-contract build");
    assert!(
        status.success(),
        "cargo pvm-contract build ({profile}) failed"
    );
}

fn verify_build_artifacts(project_dir: &Path, binary_name: &str, profile: &str) {
    verify_polkavm_binary(project_dir, binary_name, profile);
    verify_abi_json(project_dir, binary_name, profile);
}

fn verify_polkavm_binary(project_dir: &Path, binary_name: &str, profile: &str) {
    let target_dir = project_dir.join("target").join(profile);
    let polkavm_file = target_dir.join(format!("{binary_name}.polkavm"));
    assert!(
        polkavm_file.exists(),
        "PolkaVM binary not found: {}",
        polkavm_file.display()
    );
}

fn verify_abi_json(project_dir: &Path, binary_name: &str, profile: &str) {
    let target_dir = project_dir.join("target").join(profile);
    let abi_file = target_dir.join(format!("{binary_name}.abi.json"));
    assert!(
        abi_file.exists(),
        "ABI JSON not found: {}",
        abi_file.display()
    );

    let abi_content = std::fs::read_to_string(&abi_file).expect("read ABI file");
    let abi: serde_json::Value = serde_json::from_str(&abi_content).expect("parse ABI JSON");
    assert!(abi.is_array(), "ABI should be an array");
}

fn verify_cargo_toml(project_dir: &Path, use_dsl: bool) {
    let cargo_toml =
        std::fs::read_to_string(project_dir.join("Cargo.toml")).expect("Cargo.toml exists");

    if use_dsl {
        assert!(cargo_toml.contains("pvm-contract-builder-dsl"));
    } else {
        assert!(cargo_toml.contains("pvm-contract-macros"));
    }
    assert!(cargo_toml.contains("pvm-contract-types"));
    assert!(cargo_toml.contains("polkavm-derive"));
    assert!(cargo_toml.contains("ruint"));
    assert!(
        !cargo_toml.contains("[build-dependencies]"),
        "Cargo.toml should not contain [build-dependencies]"
    );
}

#[test]
fn mytoken_macro_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "mytoken-macro-debug", "MyToken", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "mytoken-macro-debug", "debug");
}

#[test]
fn mytoken_macro_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "mytoken-macro-release", "MyToken", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "mytoken-macro-release", "release");
}

#[test]
fn mytoken_dsl_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "mytoken-dsl-debug", "MyToken", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "mytoken-dsl-debug", "debug");
}

#[test]
fn mytoken_dsl_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "mytoken-dsl-release", "MyToken", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "release");
    verify_polkavm_binary(&project_dir, "mytoken-dsl-release", "release");
}

#[test]
fn fibonacci_macro_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "fibonacci-macro-debug", "Fibonacci", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "fibonacci-macro-debug", "debug");
}

#[test]
fn fibonacci_macro_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "fibonacci-macro-release", "Fibonacci", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "fibonacci-macro-release", "release");
}

#[test]
fn fibonacci_dsl_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "fibonacci-dsl-debug", "Fibonacci", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "fibonacci-dsl-debug", "debug");
}

#[test]
fn fibonacci_dsl_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "fibonacci-dsl-release", "Fibonacci", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "release");
    verify_polkavm_binary(&project_dir, "fibonacci-dsl-release", "release");
}

#[test]
fn new_contract_macro_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-macro-debug", "macro", None);

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "new-macro-debug", "debug");
}

#[test]
fn new_contract_macro_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-macro-release", "macro", None);

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "new-macro-release", "release");
}

#[test]
fn new_contract_dsl_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-dsl-debug", "dsl", Some("no-alloc"));

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "new-dsl-debug", "debug");
}

#[test]
fn new_contract_dsl_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-dsl-release", "dsl", Some("no-alloc"));

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "release");
    verify_polkavm_binary(&project_dir, "new-dsl-release", "release");
}

#[test]
fn abi_json_has_correct_structure() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "abi-test", "MyToken", "macro");

    build_project(&project_dir, "debug");

    let abi_file = project_dir
        .join("target")
        .join("debug")
        .join("abi-test.abi.json");
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
fn multi_macro_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "multi-macro-debug", "Multi", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "multi-macro-debug", "debug");
}

#[test]
fn multi_dsl_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "multi-dsl-debug", "Multi", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "multi-dsl-debug", "debug");
}

#[test]
fn new_contract_dsl_bump_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-dsl-bump-debug", "dsl", Some("bump"));

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "new-dsl-bump-debug", "debug");
}

#[test]
fn new_contract_macro_bump_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir =
        scaffold_new_contract(&temp_dir, "new-macro-bump-debug", "macro", Some("bump"));

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "new-macro-bump-debug", "debug");
}

#[test]
fn polkavm_binary_is_valid() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "polkavm-test", "Fibonacci", "macro");

    build_project(&project_dir, "release");

    let polkavm_file = project_dir
        .join("target")
        .join("release")
        .join("polkavm-test.polkavm");
    let binary = std::fs::read(&polkavm_file).expect("read polkavm file");

    assert!(!binary.is_empty(), "PolkaVM binary should not be empty");
    assert!(
        binary.len() < 100_000,
        "Release binary should be reasonably small (got {} bytes)",
        binary.len()
    );
}

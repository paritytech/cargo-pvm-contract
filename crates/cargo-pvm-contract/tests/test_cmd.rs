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

fn run_cli_test(project_dir: &Path) {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(project_dir)
        .arg("pvm-contract")
        .arg("test")
        .arg("--")
        .arg("--nocapture")
        .assert()
        .success();
}

fn run_cli_test_with_manifest(manifest_path: &Path, cwd: &Path) {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(cwd)
        .arg("pvm-contract")
        .arg("test")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--")
        .arg("--nocapture")
        .assert()
        .success();
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
        assert!(cargo_toml.contains("pvm-contract-sdk"));
    }
    assert!(cargo_toml.contains("polkavm-derive"));
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

#[test]
fn cli_test_is_end_to_end_for_scaffolded_macro_project() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "cli-test-macro", "macro", None);

    run_cli_test(&project_dir);
}

#[test]
fn cli_test_supports_manifest_path_from_outside_project_dir() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "cli-test-manifest", "macro", None);
    let manifest_path = project_dir.join("Cargo.toml");

    run_cli_test_with_manifest(&manifest_path, temp_dir.path());
}

/// Remove the standalone `[workspace]` table that the scaffold template adds
/// to each member's `Cargo.toml`, so the crate can be included in a parent
/// workspace. Parsing with `toml_edit` keeps this robust against template
/// whitespace/ordering changes.
fn strip_workspace_table(manifest: &Path) {
    let content = std::fs::read_to_string(manifest).expect("read member Cargo.toml");
    let mut doc: toml_edit::DocumentMut = content.parse().expect("parse member Cargo.toml");
    doc.remove("workspace");
    std::fs::write(manifest, doc.to_string()).expect("write member Cargo.toml");
}

#[test]
fn build_selects_workspace_member_via_package_flag() {
    let temp_dir = TempDir::new().expect("temp dir");

    let pkg_a = scaffold_new_contract(&temp_dir, "ws-pkg-a", "macro", None);
    let pkg_b = scaffold_new_contract(&temp_dir, "ws-pkg-b", "macro", None);

    strip_workspace_table(&pkg_a.join("Cargo.toml"));
    strip_workspace_table(&pkg_b.join("Cargo.toml"));

    let workspace_manifest = temp_dir.path().join("Cargo.toml");
    std::fs::write(
        &workspace_manifest,
        "[workspace]\nresolver = \"2\"\nmembers = [\"ws-pkg-a\", \"ws-pkg-b\"]\n",
    )
    .expect("write workspace Cargo.toml");

    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .arg("pvm-contract")
        .arg("build")
        .arg("--manifest-path")
        .arg(&workspace_manifest)
        .arg("-p")
        .arg("ws-pkg-a");

    let status = cmd.status().expect("run cargo pvm-contract build -p");
    assert!(status.success(), "build -p ws-pkg-a failed");

    let target_release = temp_dir.path().join("target").join("release");
    assert!(
        target_release.join("ws-pkg-a.polkavm").exists(),
        "ws-pkg-a.polkavm should exist at workspace target/release"
    );
    assert!(
        target_release.join("ws-pkg-a.abi.json").exists(),
        "ws-pkg-a.abi.json should exist at workspace target/release"
    );
    assert!(
        !target_release.join("ws-pkg-b.polkavm").exists(),
        "ws-pkg-b.polkavm should NOT exist — only ws-pkg-a was selected"
    );
}

#[test]
fn build_forwards_features_with_package_flag() {
    let temp_dir = TempDir::new().expect("temp dir");

    let pkg = scaffold_new_contract(&temp_dir, "ws-feat-pkg", "macro", None);
    strip_workspace_table(&pkg.join("Cargo.toml"));

    // Add a `gated-build` feature to the member.
    let member_manifest = pkg.join("Cargo.toml");
    let content = std::fs::read_to_string(&member_manifest).expect("read member Cargo.toml");
    let mut doc: toml_edit::DocumentMut = content.parse().expect("parse member Cargo.toml");
    let features = doc
        .entry("features")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .expect("[features] table");
    features.insert("gated-build", toml_edit::value(toml_edit::Array::new()));
    std::fs::write(&member_manifest, doc.to_string()).expect("write member Cargo.toml");

    // Make the contract refuse to compile unless `gated-build` is on. This
    // proves the feature was actually forwarded — without it, the build
    // would hit `compile_error!`. Appended after the scaffold's inner
    // attributes so it doesn't push them past an item.
    let src = pkg.join("src/ws-feat-pkg.rs");
    let mut source = std::fs::read_to_string(&src).expect("read contract source");
    source.push_str(
        "\n#[cfg(not(feature = \"gated-build\"))]\ncompile_error!(\"gated-build feature is required\");\n",
    );
    std::fs::write(&src, source).expect("write contract source");

    let workspace_manifest = temp_dir.path().join("Cargo.toml");
    std::fs::write(
        &workspace_manifest,
        "[workspace]\nresolver = \"2\"\nmembers = [\"ws-feat-pkg\"]\n",
    )
    .expect("write workspace Cargo.toml");

    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .arg("pvm-contract")
        .arg("build")
        .arg("--manifest-path")
        .arg(&workspace_manifest)
        .arg("-p")
        .arg("ws-feat-pkg")
        .arg("--features")
        .arg("gated-build");

    let status = cmd
        .status()
        .expect("run cargo pvm-contract build -p ... --features ...");
    assert!(
        status.success(),
        "build -p ws-feat-pkg --features gated-build failed"
    );

    let target_release = temp_dir.path().join("target").join("release");
    assert!(
        target_release.join("ws-feat-pkg.polkavm").exists(),
        "ws-feat-pkg.polkavm should exist at workspace target/release"
    );
    assert!(
        target_release.join("ws-feat-pkg.abi.json").exists(),
        "ws-feat-pkg.abi.json should exist — proves --features reached the abi-gen invocation too"
    );
}

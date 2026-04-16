use std::path::PathBuf;
use std::process::Command;

fn test_abi_contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("test_abi_contract")
}

fn cargo_run_abi(bin_name: &str) -> serde_json::Value {
    let dir = test_abi_contract_dir();

    let output = Command::new(env!("CARGO"))
        .current_dir(&dir)
        .arg("run")
        .arg("--features")
        .arg("abi-gen")
        .arg("--bin")
        .arg(bin_name)
        .output()
        .expect("failed to run cargo");

    assert!(
        output.status.success(),
        "cargo run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("invalid utf8 in stdout");
    serde_json::from_str(&stdout).expect("failed to parse ABI JSON")
}

fn expected_abi(name: &str) -> serde_json::Value {
    let path = test_abi_contract_dir().join(format!("abi_{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

#[test]
fn constructor_with_params_produces_valid_abi() {
    assert_eq!(
        cargo_run_abi("constructor-with-params"),
        expected_abi("constructor_with_params"),
    );
}

#[test]
fn constructor_no_params_produces_valid_abi() {
    assert_eq!(
        cargo_run_abi("constructor-no-params"),
        expected_abi("constructor_no_params"),
    );
}

#[test]
fn custom_type_method_produces_valid_abi() {
    assert_eq!(
        cargo_run_abi("custom-type-method"),
        expected_abi("custom_type_method"),
    );
}

#[test]
fn multi_method_produces_valid_abi() {
    assert_eq!(cargo_run_abi("multi-method"), expected_abi("multi_method"),);
}

#[test]
fn nested_custom_type_produces_valid_abi() {
    assert_eq!(
        cargo_run_abi("nested-custom-type"),
        expected_abi("nested_custom_type"),
    );
}

#[test]
fn dynamic_custom_return_produces_valid_abi() {
    assert_eq!(
        cargo_run_abi("dynamic-custom-return"),
        expected_abi("dynamic_custom_return"),
    );
}

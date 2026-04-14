use std::path::PathBuf;
use std::process::Command;

fn test_abi_contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("test_abi_contract")
}

/// Build a test contract with `--features abi-gen` on the host and capture ABI JSON from stdout.
fn build_and_extract_abi(bin_name: &str) -> serde_json::Value {
    let dir = test_abi_contract_dir();

    // Detect host triple
    let rustc_output = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("failed to run rustc -vV");
    let stdout = String::from_utf8_lossy(&rustc_output.stdout);
    let host = stdout
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .expect("could not detect host triple");

    let output = Command::new(env!("CARGO"))
        .current_dir(&dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env("RUSTC_BOOTSTRAP", "1")
        .arg("run")
        .arg("--target")
        .arg(host)
        .arg("--config")
        .arg(r#"unstable.build-std=["std","core","alloc"]"#)
        .arg("--features")
        .arg("abi-gen")
        .arg("--bin")
        .arg(bin_name)
        .output()
        .expect("failed to run abi-gen");

    assert!(
        output.status.success(),
        "abi-gen failed for {bin_name}:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = String::from_utf8(output.stdout).expect("invalid UTF-8");
    serde_json::from_str(json.trim()).expect("invalid JSON")
}

fn expected_abi(name: &str) -> serde_json::Value {
    let path = test_abi_contract_dir().join(format!("abi_{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

#[test]
fn constructor_no_params_produces_valid_abi() {
    assert_eq!(
        build_and_extract_abi("constructor-no-params"),
        expected_abi("constructor_no_params"),
    );
}

#[test]
fn constructor_with_params_produces_valid_abi() {
    assert_eq!(
        build_and_extract_abi("constructor-with-params"),
        expected_abi("constructor_with_params"),
    );
}

#[test]
fn custom_type_method_produces_valid_abi() {
    assert_eq!(
        build_and_extract_abi("custom-type-method"),
        expected_abi("custom_type_method"),
    );
}

#[test]
fn multi_method_produces_valid_abi() {
    assert_eq!(
        build_and_extract_abi("multi-method"),
        expected_abi("multi_method"),
    );
}

#[test]
fn nested_custom_type_produces_valid_abi() {
    assert_eq!(
        build_and_extract_abi("nested-custom-type"),
        expected_abi("nested_custom_type"),
    );
}

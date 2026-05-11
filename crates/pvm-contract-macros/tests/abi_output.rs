use std::path::PathBuf;
use std::process::Command;

fn test_abi_contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("test_abi_contract")
}

fn cargo_run_abi(bin_name: &str) -> String {
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
    serde_json::to_string_pretty(
        &serde_json::from_str::<serde_json::Value>(&stdout).expect("failed to parse ABI JSON"),
    )
    .expect("failed to serialzie json abi")
}

#[test]
fn constructor_with_params_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_constructor_with_params.json")
        .assert_eq(&cargo_run_abi("constructor-with-params"));
}

#[test]
fn constructor_no_params_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_constructor_no_params.json")
        .assert_eq(&cargo_run_abi("constructor-no-params"));
}

#[test]
fn constructor_payable_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_constructor_payable.json")
        .assert_eq(&cargo_run_abi("constructor-payable"));
}

#[test]
fn custom_type_method_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_custom_type_method.json")
        .assert_eq(&cargo_run_abi("custom-type-method"));
}

#[test]
fn multi_method_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_multi_method.json")
        .assert_eq(&cargo_run_abi("multi-method"));
}

#[test]
fn nested_custom_type_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_nested_custom_type.json")
        .assert_eq(&cargo_run_abi("nested-custom-type"));
}

#[test]
fn dynamic_custom_return_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_dynamic_custom_return.json")
        .assert_eq(&cargo_run_abi("dynamic-custom-return"));
}

/// Contract with real host API calls (get_storage, set_storage, caller).
/// Verifies that abi-gen cfg-gating correctly excludes function bodies
/// that reference HostFnImpl methods unavailable on the host target.
#[test]
fn host_api_calls_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_host_api_calls.json")
        .assert_eq(&cargo_run_abi("host-api-calls"))
}

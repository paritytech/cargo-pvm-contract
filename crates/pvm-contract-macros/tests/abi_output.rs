use std::path::PathBuf;
use std::process::Command;

fn test_abi_contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("test_abi_contract")
}

/// Build a test contract for RISC-V and extract ABI JSON from the ELF.
fn build_and_extract_abi(bin_name: &str) -> serde_json::Value {
    let dir = test_abi_contract_dir();

    let mut args = polkavm_linker::TargetJsonArgs::default();
    args.is_64_bit = true;
    let target_json = polkavm_linker::target_json_path(args)
        .expect("failed to get target JSON path");

    let rustflags = "-Zunstable-options -Cpanic=immediate-abort -Clink-arg=--undefined=__PVM_ABI";

    let output = Command::new(env!("CARGO"))
        .current_dir(&dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env("RUSTFLAGS", rustflags)
        .env("RUSTC_BOOTSTRAP", "1")
        .arg("build")
        .arg("--target")
        .arg(&target_json)
        .arg("-Zbuild-std=core,alloc")
        .arg("--bin")
        .arg(bin_name)
        .output()
        .expect("failed to run cargo build");

    assert!(
        output.status.success(),
        "cargo build failed for {bin_name}:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Find the ELF binary
    let elf_path = dir
        .join("target")
        .join("riscv64emac-unknown-none-polkavm")
        .join("debug")
        .join(bin_name);

    assert!(
        elf_path.exists(),
        "ELF not found at: {}",
        elf_path.display()
    );

    let elf_bytes = std::fs::read(&elf_path)
        .unwrap_or_else(|e| panic!("failed to read ELF {}: {e}", elf_path.display()));

    let abi_json = cargo_pvm_contract_builder::extract_abi_from_elf(&elf_bytes)
        .expect("failed to extract ABI from ELF")
        .expect("no __PVM_ABI symbol found in ELF");

    serde_json::from_str(&abi_json).expect("failed to parse extracted ABI JSON")
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

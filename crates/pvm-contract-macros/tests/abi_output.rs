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

/// Contract with a mix of static and dynamic storage value types. Verifies
/// that the storageLayout JSON resolves type names for `Lazy<T>` and
/// `Mapping<K, V>` syntactically via `SolEncode::SOL_NAME`, including dynamic
/// `V` (`String`, `Bytes`) and nested mappings — i.e. the case the old
/// `StorageLayoutType` trait blocked under coherence rules.
#[test]
fn storage_layout_mixed_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_storage_layout_mixed.json")
        .assert_eq(&cargo_run_abi("storage-layout-mixed"))
}

/// Contract that embeds two `#[storage]` sub-structs (`Erc20State`,
/// `MetadataState`) alongside a flat `Lazy<bool>` field. Verifies that
/// composed storage renders solc-style under abi-gen: each sub-struct is a
/// single entry (`erc20`, `metadata`) whose `type` keys into the `types`
/// table (`t_struct(Erc20State)0_storage`, …) describing the struct's
/// members with struct-relative slots, via `StorageLayoutEmit::emit_entries`
/// and `StorageTypeName::emit_members`, with top-level slot ranges chained
/// through the sub-structs' `StorageType::SLOTS`.
#[test]
fn storage_layout_composed_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_storage_layout_composed.json")
        .assert_eq(&cargo_run_abi("storage-layout-composed"))
}

/// Contract with `StorageVec` storage fields. Verifies that the storageLayout
/// JSON compiles and resolves `StorageVec<T>` to Solidity's `T[]` type name,
/// including nested (`uint256[][]`) and mapping-valued
/// (`mapping(address => uint256[])`) shapes — i.e. the case that previously
/// failed to compile under `--features abi-gen` because `StorageVec` lacked a
/// layout-emit path.
#[test]
fn storage_layout_vec_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_storage_layout_vec.json")
        .assert_eq(&cargo_run_abi("storage-layout-vec"))
}

/// Contract with auto-numbered storage only — no `#[slot]` attribute anywhere.
/// Verifies that the storageLayout JSON is emitted for the default
/// auto-numbering mode and that sub-word fields pack solc-style: `count`
/// (`uint32`, offset 0) and `owner` (`address`, offset 4) share slot 0.
#[test]
fn storage_layout_auto_produces_valid_abi() {
    expect_test::expect_file!("./test_abi_contract/abi_storage_layout_auto.json")
        .assert_eq(&cargo_run_abi("storage-layout-auto"))
}

/// The auto-numbered form and its `#[slot(0)]`-anchored `#[storage]`
/// sub-struct twin must describe the same packing — same slots, offsets, and
/// types. The anchored form renders solc-style: a single `state` entry whose
/// `type` keys into the `types` table, and the two fields are that entry's
/// `members`. Member slots are struct-relative, which coincide with the
/// auto-numbered absolute slots because the sub-struct is anchored at slot 0.
#[test]
fn auto_and_anchored_layouts_agree() {
    fn storage_layout(bin_name: &str) -> serde_json::Value {
        let abi: serde_json::Value =
            serde_json::from_str(&cargo_run_abi(bin_name)).expect("failed to parse ABI JSON");
        abi["storageLayout"].clone()
    }

    let auto = storage_layout("storage-layout-auto");
    let anchored = storage_layout("storage-layout-anchored");

    let entries = anchored["storage"]
        .as_array()
        .expect("storage must be an array");
    assert_eq!(
        entries.len(),
        1,
        "anchored sub-struct must render as a single struct-typed entry: {anchored}"
    );
    let state = &entries[0];
    assert_eq!(state["label"], "state");
    assert_eq!(state["slot"], "0");
    assert_eq!(state["offset"], 0);
    let key = state["type"].as_str().expect("type must be a string");
    let state_type = &anchored["types"][key];
    assert_eq!(
        state_type["label"], "struct AnchoredCounter.CounterState",
        "sub-struct type must be registered under its key {key}: {anchored}"
    );

    assert_eq!(
        auto["storage"], state_type["members"],
        "auto-numbered fields and #[slot(0)]-anchored sub-struct members must agree"
    );
}

/// Contract mixing a numeric `#[slot(N)]` field with a raw external
/// (`#[slot(raw = KEY)]`) field. Verifies the contract compiles under
/// `--features abi-gen` and that the raw external slot is OMITTED from
/// `storageLayout` (like solc, which doesn't emit EIP-1967-style slots),
/// while the sequential field is still present.
#[test]
fn storage_layout_raw_slot_omits_external_slot() {
    expect_test::expect_file!("./test_abi_contract/abi_proxy_raw_slot.json")
        .assert_eq(&cargo_run_abi("proxy-raw-slot"))
}

/// Contract that has enum params
#[test]
fn enum_abi() {
    expect_test::expect_file!("./test_abi_contract/enum_abi.json")
        .assert_eq(&cargo_run_abi("enum-abi"))
}

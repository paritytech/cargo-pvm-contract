//! E2E tests for `cargo-pvm-contract-extrinsics` — the Substrate extrinsics path.
//!
//! These tests exercise the native Substrate RPC (subxt) against `revive-dev-node`,
//! proving that upload, instantiate, call, remove, and query operations work
//! end-to-end through `pallet-revive` dispatchables.
//!
//! Each test starts its own `revive-dev-node` instance for full isolation.
//!
//! Requirements: nightly + rust-src + solc + revive-dev-node
//! Run: cargo test -p pvm-contract-e2e-tests --test extrinsics_e2e_tests -- --test-threads=1

use cargo_pvm_contract_extrinsics::Code;
use pvm_contract_e2e_tests::build::contract;
use pvm_contract_e2e_tests::dev_node::SubstrateDevNode;
use pvm_contract_e2e_tests::substrate_client::{SubstrateClient, encode_call};

const DEFAULT_VARIANT: &str = "example-mytoken-macro-bump-alloc";

fn mytoken() -> pvm_contract_e2e_tests::build::Contract {
    contract("example-mytoken")
}

fn build_mytoken() -> (Vec<u8>, std::path::PathBuf) {
    let c = mytoken();
    c.build();
    let bytecode =
        std::fs::read(c.polkavm_binary(DEFAULT_VARIANT, "release")).expect("read polkavm binary");
    let abi_path = c.abi_json_path(DEFAULT_VARIANT, "release");
    (bytecode, abi_path)
}

fn start_and_client() -> (SubstrateDevNode, SubstrateClient) {
    let node = SubstrateDevNode::start();
    let client = SubstrateClient::new(node.ws_url());
    (node, client)
}

#[tokio::test]
async fn instantiate_dry_run_estimates_gas() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, _) = build_mytoken();

    let result = client
        .instantiate_dry_run(Code::Upload(bytecode), vec![], &alice)
        .await
        .expect("instantiate dry-run");

    assert!(
        result.weight_required.ref_time() > 0,
        "weight_required ref_time should be > 0"
    );
    assert!(
        result.result.is_ok(),
        "dry-run should succeed, got: {:?}",
        result.result
    );
}

#[tokio::test]
async fn call_revert_detected() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, abi_path) = build_mytoken();

    let deploy = client
        .instantiate(Code::Upload(bytecode), vec![], &alice)
        .await
        .expect("deploy");

    // Transfer with 0 balance — should revert
    let transfer_data = encode_call(
        &abi_path,
        "transfer",
        &["0x0000000000000000000000000000000000000001", "100"],
    );
    let result = client
        .call_dry_run(deploy.contract_address, transfer_data, &alice)
        .await
        .expect("transfer dry-run");

    match result.result {
        Ok(ref exec_result) => {
            assert!(
                exec_result.did_revert(),
                "transfer with 0 balance should revert"
            );
        }
        Err(_) => {
            // A dispatch error also indicates failure, which is acceptable
        }
    }
}

#[tokio::test]
async fn remove_code_after_upload() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, _) = build_mytoken();

    client.upload_code(&bytecode, &alice).await.expect("upload");

    let code_hash = cargo_pvm_contract_extrinsics::ContractBinary(bytecode).code_hash();
    client
        .remove_code(subxt::utils::H256::from(code_hash), &alice)
        .await
        .expect("remove_code should succeed when no instances exist");
}

#[tokio::test]
async fn full_lifecycle_upload_instantiate_call() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, abi_path) = build_mytoken();

    // 1. Map alice's account (may already be mapped in dev mode)
    let alice_addr = match client.map_account(&alice).await {
        Ok(alice_map) => {
            assert_ne!(alice_map.address, sp_core::H160::zero());
            format!("{:?}", alice_map.address)
        }
        Err(e) if e.to_string().contains("AccountAlreadyMapped") => {
            // In dev mode Alice is pre-mapped; derive her EVM address
            let alice_id = subxt_signer::sr25519::dev::alice().public_key().0;
            let addr = cargo_pvm_contract_extrinsics::AccountIdMapper::to_address(&alice_id);
            format!("{:?}", addr)
        }
        Err(e) => panic!("map_account: {e}"),
    };

    // 2. Deploy contract (upload + instantiate in one shot)
    let deploy = client
        .instantiate(Code::Upload(bytecode), vec![], &alice)
        .await
        .expect("instantiate");
    assert_ne!(deploy.contract_address, sp_core::H160::zero());

    // 3. Verify totalSupply is 0
    let supply_data = encode_call(&abi_path, "totalSupply", &[]);
    let supply_result = client
        .call_dry_run(deploy.contract_address, supply_data, &alice)
        .await
        .expect("totalSupply");
    let supply = uint256_from_bytes(&supply_result.result.unwrap().data);
    assert_eq!(supply, 0);

    // 4. Mint 500 tokens
    let mint_data = encode_call(&abi_path, "mint", &[&alice_addr, "500"]);
    client
        .call(deploy.contract_address, mint_data, &alice)
        .await
        .expect("mint");

    // 5. Verify totalSupply is 500
    let supply_data = encode_call(&abi_path, "totalSupply", &[]);
    let supply_result = client
        .call_dry_run(deploy.contract_address, supply_data, &alice)
        .await
        .expect("totalSupply after mint");
    let supply = uint256_from_bytes(&supply_result.result.unwrap().data);
    assert_eq!(supply, 500);

    // 6. Verify balanceOf(alice) is 500
    let balance_data = encode_call(&abi_path, "balanceOf", &[&alice_addr]);
    let balance_result = client
        .call_dry_run(deploy.contract_address, balance_data, &alice)
        .await
        .expect("balanceOf");
    let balance = uint256_from_bytes(&balance_result.result.unwrap().data);
    assert_eq!(balance, 500);
}

#[tokio::test]
async fn fetch_contract_info_after_instantiate() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, _) = build_mytoken();

    let deploy = client
        .instantiate(Code::Upload(bytecode.clone()), vec![], &alice)
        .await
        .expect("instantiate");

    let info = client
        .fetch_contract_info(&deploy.contract_address)
        .await
        .expect("fetch_contract_info");

    let expected_code_hash = subxt::utils::H256::from(
        cargo_pvm_contract_extrinsics::ContractBinary(bytecode).code_hash(),
    );
    assert_eq!(*info.code_hash(), expected_code_hash);
    assert_eq!(info.storage_bytes(), 0);
}

#[tokio::test]
async fn fetch_contract_info_non_existent_fails() {
    let (_node, client) = start_and_client();
    let fake_addr = sp_core::H160::from_low_u64_be(0x99);

    let result = client.fetch_contract_info(&fake_addr).await;
    assert!(result.is_err(), "should fail for non-existent contract");
}

#[tokio::test]
async fn get_account_data_for_alice() {
    let (_node, client) = start_and_client();

    let alice_id: subxt::utils::AccountId32 =
        subxt_signer::sr25519::dev::alice().public_key().0.into();
    let data = client
        .get_account_data(&alice_id)
        .await
        .expect("get_account_data");

    assert!(data.free > 0, "Alice should have a non-zero free balance");
}

#[tokio::test]
async fn resolve_h160_for_mapped_account() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();

    // Ensure Alice is mapped
    let _ = client.map_account(&alice).await;

    let alice_id_bytes = subxt_signer::sr25519::dev::alice().public_key().0;
    let alice_h160 = cargo_pvm_contract_extrinsics::AccountIdMapper::to_address(&alice_id_bytes);

    let resolved = client
        .resolve_h160(&alice_h160)
        .await
        .expect("resolve_h160");

    let expected_id: subxt::utils::AccountId32 = alice_id_bytes.into();
    assert_eq!(resolved, expected_id);
}

#[tokio::test]
async fn fetch_all_contracts_includes_deployed() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, _) = build_mytoken();

    let deploy = client
        .instantiate(Code::Upload(bytecode), vec![], &alice)
        .await
        .expect("instantiate");

    let contracts = client
        .fetch_all_contracts()
        .await
        .expect("fetch_all_contracts");

    assert!(
        contracts.contains(&deploy.contract_address),
        "deployed contract should appear in fetch_all_contracts"
    );

    // EOAs (like Alice's mapped address) should not appear
    let _ = client.map_account(&alice).await;
    let alice_id = subxt_signer::sr25519::dev::alice().public_key().0;
    let alice_h160 = cargo_pvm_contract_extrinsics::AccountIdMapper::to_address(&alice_id);

    let contracts = client
        .fetch_all_contracts()
        .await
        .expect("fetch_all_contracts after map");
    assert!(
        !contracts.contains(&alice_h160),
        "EOA should not appear in fetch_all_contracts"
    );
}

#[tokio::test]
async fn instantiate_from_existing_code_hash() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, _) = build_mytoken();

    // 1. Upload code only
    client.upload_code(&bytecode, &alice).await.expect("upload");

    // 2. Instantiate from the existing code hash (not re-uploading)
    let code_hash = subxt::utils::H256::from(
        cargo_pvm_contract_extrinsics::ContractBinary(bytecode).code_hash(),
    );
    let deploy = client
        .instantiate(Code::Existing(code_hash), vec![], &alice)
        .await
        .expect("instantiate from existing code hash");

    // 3. Verify the contract has been deployed
    assert_ne!(deploy.contract_address, sp_core::H160::zero());

    let info = client
        .fetch_contract_info(&deploy.contract_address)
        .await
        .expect("fetch_contract_info");
    assert_eq!(*info.code_hash(), code_hash);
}

#[tokio::test]
async fn rpc_system_chain_returns_value() {
    let (_node, client) = start_and_client();

    let result = client
        .rpc_raw_call("system_chain", &[])
        .await
        .expect("rpc system_chain");

    assert!(
        !result.is_empty(),
        "system_chain should return a non-empty string"
    );
}

#[tokio::test]
async fn rpc_invalid_method_fails() {
    let (_node, client) = start_and_client();

    let result = client.rpc_raw_call("nonExistentMethod", &[]).await;
    assert!(result.is_err(), "non-existent RPC method should fail");
}

#[tokio::test]
async fn account_id_mapper_matches_on_chain_mapping() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();

    // Map Alice's account
    let map_result = match client.map_account(&alice).await {
        Ok(r) => r.address,
        Err(e) if e.to_string().contains("AccountAlreadyMapped") => {
            let alice_id = subxt_signer::sr25519::dev::alice().public_key().0;
            cargo_pvm_contract_extrinsics::AccountIdMapper::to_address(&alice_id)
        }
        Err(e) => panic!("map_account: {e}"),
    };

    // Local derivation should match what the chain returned
    let alice_id = subxt_signer::sr25519::dev::alice().public_key().0;
    let local_h160 = cargo_pvm_contract_extrinsics::AccountIdMapper::to_address(&alice_id);
    assert_eq!(local_h160, map_result);
}

// ---------- dynamic-types contract tests ----------

fn build_dynamic_types() -> (Vec<u8>, std::path::PathBuf) {
    let c = contract("test-contracts");
    c.build();
    let bytecode =
        std::fs::read(c.polkavm_binary("dynamic-types", "release")).expect("read polkavm binary");
    let abi_path = c.abi_json_path("dynamic-types", "release");
    (bytecode, abi_path)
}

#[tokio::test]
async fn dynamic_types_string_length() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, abi_path) = build_dynamic_types();

    let deploy = client
        .instantiate(Code::Upload(bytecode), vec![], &alice)
        .await
        .expect("instantiate dynamic-types");

    let data = encode_call(&abi_path, "getStringLength", &["hello world"]);
    let result = client
        .call_dry_run(deploy.contract_address, data, &alice)
        .await
        .expect("getStringLength");
    let len = uint256_from_bytes(&result.result.unwrap().data);
    assert_eq!(len, 11, "string 'hello world' should have length 11");
}

#[tokio::test]
async fn dynamic_types_bytes_length() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, abi_path) = build_dynamic_types();

    let deploy = client
        .instantiate(Code::Upload(bytecode), vec![], &alice)
        .await
        .expect("instantiate dynamic-types");

    let data = encode_call(&abi_path, "getBytesLength", &["0xDEADBEEF"]);
    let result = client
        .call_dry_run(deploy.contract_address, data, &alice)
        .await
        .expect("getBytesLength");
    let len = uint256_from_bytes(&result.result.unwrap().data);
    assert_eq!(len, 4, "bytes 0xDEADBEEF should have length 4");
}

#[tokio::test]
async fn dynamic_types_sum_array() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, abi_path) = build_dynamic_types();

    let deploy = client
        .instantiate(Code::Upload(bytecode), vec![], &alice)
        .await
        .expect("instantiate dynamic-types");

    let data = encode_call(&abi_path, "sumArray", &["[1,2,3]"]);
    let result = client
        .call_dry_run(deploy.contract_address, data, &alice)
        .await
        .expect("sumArray");
    let sum = uint256_from_bytes(&result.result.unwrap().data);
    assert_eq!(sum, 6, "sum of [1,2,3] should be 6");
}

// ---------- composite-types contract tests ----------

fn build_composite_types() -> (Vec<u8>, std::path::PathBuf) {
    let c = contract("test-contracts");
    c.build();
    let bytecode =
        std::fs::read(c.polkavm_binary("composite-types", "release")).expect("read polkavm binary");
    let abi_path = c.abi_json_path("composite-types", "release");
    (bytecode, abi_path)
}

#[tokio::test]
#[ignore = "ABI generator does not yet emit tuple components — see parse_sol_params in abi.rs"]
async fn composite_types_process_tuple() {
    let (_node, client) = start_and_client();
    let alice = SubstrateClient::alice();
    let (bytecode, abi_path) = build_composite_types();

    let deploy = client
        .instantiate(Code::Upload(bytecode), vec![], &alice)
        .await
        .expect("instantiate composite-types");

    // processTuple((42, true)) should return 42 (returns amount if flag is true)
    let data = encode_call(&abi_path, "processTuple", &["(42,true)"]);
    let result = client
        .call_dry_run(deploy.contract_address, data, &alice)
        .await
        .expect("processTuple(42,true)");
    let val = uint256_from_bytes(&result.result.unwrap().data);
    assert_eq!(val, 42, "processTuple((42,true)) should return 42");

    // processTuple((42, false)) should return 0 (returns 0 if flag is false)
    let data = encode_call(&abi_path, "processTuple", &["(42,false)"]);
    let result = client
        .call_dry_run(deploy.contract_address, data, &alice)
        .await
        .expect("processTuple(42,false)");
    let val = uint256_from_bytes(&result.result.unwrap().data);
    assert_eq!(val, 0, "processTuple((42,false)) should return 0");
}

/// Decode a big-endian uint256 (32 bytes) into a u128.
fn uint256_from_bytes(data: &[u8]) -> u128 {
    assert!(data.len() >= 32, "uint256 needs at least 32 bytes");
    for &b in &data[..16] {
        assert_eq!(b, 0, "value exceeds u128::MAX");
    }
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&data[16..32]);
    u128::from_be_bytes(buf)
}

//! E2E validation of `pallet-revive` weight for an OrderedIndex prefix-range
//! query, measured against the storage-ops-only prediction emitted by the
//! `measure-ordered-index` benchmark binary in `pvm-storage`.
//!
//! ## Prediction (storage-ops-only weight model)
//!
//! Run this command before the test to capture the predicted weight:
//!
//! ```text
//! BENCH_N=100 BENCH_Q=1000 BENCH_T=10 \
//!   cargo run --release -p pvm-storage --bin measure-ordered-index --features alloc
//! ```
//!
//! The JSON output's `weight_ref_time_per_query` field is the predicted
//! on-chain ref_time for one prefix-range query over a 100-record
//! `OrderedIndex<String, u64, T=10>`. This test measures the real on-chain
//! weight via `call_dry_run` and prints both numbers — predicted and measured —
//! plus the ratio (measured / predicted).
//!
//! ## What this test does
//!
//! 1. Boots a `revive-dev-node` (instant-seal) on a fresh port.
//! 2. Builds `examples/ordered-index-bench` and deploys it via
//!    `client.instantiate(Code::Upload(bytecode), …, &alice)`.
//! 3. Submits 100 real extrinsic `insert(key, value)` calls to populate the
//!    index with `user0000..user0099`.
//! 4. Dry-runs `rangeQuery("user005")` via `client.call_dry_run`, reads
//!    `weight_required.ref_time()` (the on-chain ref_time), and prints it
//!    alongside the result count.
//!
//! ## Weight field path
//!
//! `client.call_dry_run(addr, data, signer)` returns
//! `cargo_pvm_contract_extrinsics::pallet_revive_primitives::ContractExecResult<u128>`.
//! The on-chain ref_time lives at `.weight_required.ref_time()`.
//!
//! Run with:
//!
//! ```text
//! REVIVE_DEV_NODE=$HOME/.local/bin/revive-dev-node \
//!   cargo test -p pvm-contract-e2e-tests \
//!     --test ordered_index_validation \
//!     -- --test-threads=1 --nocapture
//! ```

use cargo_pvm_contract_extrinsics::Code;
use pvm_contract_e2e_tests::build::contract;
use pvm_contract_e2e_tests::dev_node::SubstrateDevNode;
use pvm_contract_e2e_tests::substrate_client::{SubstrateClient, encode_call};

const DEFAULT_VARIANT: &str = "ordered-index-bench";

fn bench_contract() -> pvm_contract_e2e_tests::build::Contract {
    contract("ordered-index-bench")
}

fn build_bench() -> (Vec<u8>, std::path::PathBuf) {
    let c = bench_contract();
    c.build();
    let bytecode = std::fs::read(c.polkavm_binary(DEFAULT_VARIANT, "release"))
        .expect("read polkavm binary");
    let abi_path = c.abi_json_path(DEFAULT_VARIANT, "release");
    (bytecode, abi_path)
}

#[tokio::test]
async fn ordered_index_range_query_gas() {
    let _node = SubstrateDevNode::start();
    let client = SubstrateClient::new(_node.ws_url());
    let alice = SubstrateClient::alice();

    let (bytecode, abi_path) = build_bench();

    let deploy = client
        .instantiate(Code::Upload(bytecode), vec![], &alice)
        .await
        .expect("instantiate ordered-index-bench");

    for i in 0u64..100 {
        let k = format!("user{:04}", i);
        let v = i.to_string();
        let insert_data = encode_call(&abi_path, "insert", &[&k, &v]);
        client
            .call(deploy.contract_address, insert_data, &alice)
            .await
            .unwrap_or_else(|e| panic!("insert {k} failed: {e}"));
    }

    let query_data = encode_call(&abi_path, "rangeQuery", &["user005"]);
    let result = client
        .call_dry_run(deploy.contract_address, query_data, &alice)
        .await
        .expect("rangeQuery dry-run");

    let measured_ref_time = result.weight_required.ref_time();
    let exec_result = result.result.expect("rangeQuery should succeed");
    let count = u64::from_be_bytes(
        exec_result.data[24..32]
            .try_into()
            .expect("decode uint64 result"),
    );

    eprintln!(
        "[validation] measured rangeQuery ref_time = {} ps",
        measured_ref_time
    );
    eprintln!("[validation] rangeQuery returned count = {}", count);
    eprintln!(
        "[validation] (compare to predicted `weight_ref_time_per_query` from \
         BENCH_N=100 BENCH_Q=1000 BENCH_T=10 measure-ordered-index)"
    );

    assert!(
        measured_ref_time > 0,
        "weight_required ref_time should be > 0, got {measured_ref_time}"
    );
    assert!(count > 0, "rangeQuery should return at least one record");
}
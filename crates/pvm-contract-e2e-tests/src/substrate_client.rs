//! Substrate extrinsics test client.
//!
//! Wraps the `cargo-pvm-contract-extrinsics` builder APIs into a convenient
//! facade for e2e tests that exercise the native Substrate RPC path
//! (as opposed to the Ethereum JSON-RPC path used by `cast`).

use anyhow::Result;
use cargo_pvm_contract_extrinsics::{
    AccountData, CallCommandBuilder, Code, ContractBinary, ContractInfo, ExtrinsicOptsBuilder,
    InstantiateCommandBuilder, InstantiateExecResult, MapAccountCommandBuilder,
    MapAccountExecResult, RawParams, RemoveCommandBuilder, RpcRequest, UploadCommandBuilder,
    UploadResult,
    pallet_revive_primitives::{CodeUploadResult, ContractExecResult, ContractInstantiateResult},
};
use subxt::{
    blocks::ExtrinsicEvents,
    config::SubstrateConfig,
    utils::{H160, H256},
};
use subxt_signer::sr25519::Keypair;

pub type TestConfig = SubstrateConfig;
pub type TestSigner = Keypair;

/// A test helper that wraps the extrinsics crate builders for convenient use
/// in async e2e tests against a revive-dev-node.
pub struct SubstrateClient {
    ws_url: String,
}

impl SubstrateClient {
    pub fn new(ws_url: &str) -> Self {
        Self {
            ws_url: ws_url.to_string(),
        }
    }

    pub fn alice() -> TestSigner {
        subxt_signer::sr25519::dev::alice()
    }

    pub fn bob() -> TestSigner {
        subxt_signer::sr25519::dev::bob()
    }

    fn opts(
        &self,
        signer: &TestSigner,
    ) -> cargo_pvm_contract_extrinsics::ExtrinsicOpts<TestConfig, TestSigner> {
        ExtrinsicOptsBuilder::<TestConfig, TestSigner>::new(signer.clone())
            .url(url::Url::parse(&self.ws_url).expect("valid ws url"))
            .storage_deposit_limit(Some(10_000_000_000_000))
            .done()
    }

    // ── map_account ─────────────────────────────────────────────────

    pub async fn map_account(
        &self,
        signer: &TestSigner,
    ) -> Result<MapAccountExecResult<TestConfig>> {
        let exec = MapAccountCommandBuilder::new(self.opts(signer))
            .done()
            .await?;
        exec.map_account().await.map_err(|e| anyhow::anyhow!("{e}"))
    }

    // ── upload ───────────────────────────────────────────────────────

    pub async fn upload_code(
        &self,
        code: &[u8],
        signer: &TestSigner,
    ) -> Result<UploadResult<TestConfig>> {
        let mut exec = UploadCommandBuilder::new(self.opts(signer), ContractBinary(code.to_vec()))
            .done()
            .await?;
        exec.upload_code().await.map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn upload_code_dry_run(
        &self,
        code: &[u8],
        signer: &TestSigner,
    ) -> Result<CodeUploadResult<u128>> {
        let exec = UploadCommandBuilder::new(self.opts(signer), ContractBinary(code.to_vec()))
            .done()
            .await?;
        exec.upload_code_rpc().await
    }

    // ── instantiate ─────────────────────────────────────────────────

    pub async fn instantiate(
        &self,
        code: Code,
        data: Vec<u8>,
        signer: &TestSigner,
    ) -> Result<InstantiateExecResult<TestConfig>> {
        let exec = InstantiateCommandBuilder::new(self.opts(signer), code, data)
            .done()
            .await?;
        exec.instantiate(None, None)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn instantiate_dry_run(
        &self,
        code: Code,
        data: Vec<u8>,
        signer: &TestSigner,
    ) -> Result<ContractInstantiateResult<u128>> {
        let exec = InstantiateCommandBuilder::new(self.opts(signer), code, data)
            .done()
            .await?;
        exec.instantiate_dry_run().await
    }

    // ── call ────────────────────────────────────────────────────────

    pub async fn call(
        &self,
        contract: H160,
        call_data: Vec<u8>,
        signer: &TestSigner,
    ) -> Result<ExtrinsicEvents<TestConfig>> {
        let exec = CallCommandBuilder::new(contract, call_data, self.opts(signer))
            .done()
            .await?;
        exec.call(None, None)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn call_dry_run(
        &self,
        contract: H160,
        call_data: Vec<u8>,
        signer: &TestSigner,
    ) -> Result<ContractExecResult<u128>> {
        let exec = CallCommandBuilder::new(contract, call_data, self.opts(signer))
            .done()
            .await?;
        exec.call_dry_run().await
    }

    // ── remove ──────────────────────────────────────────────────────

    pub async fn remove_code(
        &self,
        code_hash: H256,
        signer: &TestSigner,
    ) -> Result<ExtrinsicEvents<TestConfig>> {
        let exec = RemoveCommandBuilder::new(self.opts(signer), code_hash)
            .done()
            .await?;
        exec.remove_code().await.map_err(|e| anyhow::anyhow!("{e}"))
    }

    // ── query helpers ───────────────────────────────────────────

    async fn rpc_and_client(
        &self,
    ) -> Result<(
        subxt::backend::legacy::LegacyRpcMethods<TestConfig>,
        subxt::OnlineClient<TestConfig>,
    )> {
        let rpc_cli = subxt::backend::rpc::RpcClient::from_url(&self.ws_url).await?;
        let client = subxt::OnlineClient::<TestConfig>::from_rpc_client(rpc_cli.clone()).await?;
        let rpc = subxt::backend::legacy::LegacyRpcMethods::<TestConfig>::new(rpc_cli);
        Ok((rpc, client))
    }

    pub async fn fetch_contract_info(&self, contract: &H160) -> Result<ContractInfo<u128>> {
        let (rpc, client) = self.rpc_and_client().await?;
        cargo_pvm_contract_extrinsics::fetch_contract_info(contract, &rpc, &client).await
    }

    pub async fn get_account_data(
        &self,
        account: &subxt::utils::AccountId32,
    ) -> Result<AccountData<u128>> {
        let (rpc, client) = self.rpc_and_client().await?;
        cargo_pvm_contract_extrinsics::get_account_data(account, &rpc, &client).await
    }

    pub async fn resolve_h160(&self, addr: &H160) -> Result<subxt::utils::AccountId32> {
        let (rpc, client) = self.rpc_and_client().await?;
        cargo_pvm_contract_extrinsics::resolve_h160(addr, &rpc, &client).await
    }

    pub async fn fetch_all_contracts(&self) -> Result<Vec<H160>> {
        let (rpc, client) = self.rpc_and_client().await?;
        cargo_pvm_contract_extrinsics::fetch_all_contracts(&client, &rpc).await
    }

    pub async fn rpc_raw_call(&self, method: &str, params: &[String]) -> Result<String> {
        let url = url::Url::parse(&self.ws_url)?;
        let rpc = RpcRequest::new(&url).await?;
        let raw_params = RawParams::new(params)?;
        let result = rpc.raw_call(method, raw_params).await?;
        Ok(result.get().to_string())
    }
}

/// Encode a function call using the `cargo-pvm-contract` CLI binary.
///
/// Returns raw calldata bytes (selector + ABI-encoded args).
pub fn encode_call(abi_path: &std::path::Path, function: &str, args: &[&str]) -> Vec<u8> {
    let mut cmd = std::process::Command::new(cargo_pvm_contract_bin());
    cmd.arg("pvm-contract")
        .arg("encode")
        .arg("--abi")
        .arg(abi_path)
        .arg("--function")
        .arg(function)
        .arg("--");
    for arg in args {
        cmd.arg(arg);
    }
    let output = cmd
        .output()
        .expect("failed to run cargo-pvm-contract encode");
    assert!(
        output.status.success(),
        "cargo-pvm-contract encode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let hex_str = String::from_utf8(output.stdout)
        .expect("valid utf8")
        .trim()
        .trim_start_matches("0x")
        .to_string();
    hex_to_bytes(&hex_str)
}

/// Encode constructor args using the `cargo-pvm-contract` CLI binary.
pub fn encode_constructor(abi_path: &std::path::Path, args: &[&str]) -> Vec<u8> {
    let mut cmd = std::process::Command::new(cargo_pvm_contract_bin());
    cmd.arg("pvm-contract")
        .arg("encode")
        .arg("--abi")
        .arg(abi_path)
        .arg("--");
    for arg in args {
        cmd.arg(arg);
    }
    let output = cmd
        .output()
        .expect("failed to run cargo-pvm-contract encode");
    assert!(
        output.status.success(),
        "cargo-pvm-contract encode (constructor) failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let hex_str = String::from_utf8(output.stdout)
        .expect("valid utf8")
        .trim()
        .trim_start_matches("0x")
        .to_string();
    hex_to_bytes(&hex_str)
}

fn cargo_pvm_contract_bin() -> std::path::PathBuf {
    // Check if cargo set the env var (available during `cargo test`)
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_cargo-pvm-contract") {
        return std::path::PathBuf::from(path);
    }
    // Fallback: workspace target directory
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.ancestors().nth(2).expect("workspace root");
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| workspace_root.join("target"));
    target_dir.join("debug").join("cargo-pvm-contract")
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    let hex = hex.trim();
    assert!(
        hex.len().is_multiple_of(2),
        "hex_to_bytes: hex string must have even length, got {}: {:?}",
        hex.len(),
        hex
    );
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or_else(|e| {
                panic!(
                    "hex_to_bytes: invalid hex at byte index {} (\"{}\"): {}",
                    i,
                    &hex[i..i + 2],
                    e
                )
            })
        })
        .collect()
}

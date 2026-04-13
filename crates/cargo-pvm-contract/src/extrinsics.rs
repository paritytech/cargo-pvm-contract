use anyhow::{Context, Result};
use cargo_pvm_contract_extrinsics::{
    CallCommandBuilder, Code, ContractBinary, ExtrinsicOptsBuilder, InstantiateCommandBuilder,
    MapAccountCommandBuilder, RemoveCommandBuilder, UploadCommandBuilder,
};
use sp_core::H160;
use subxt_signer::sr25519::Keypair;

use crate::{
    AccountArgs, CallArgs, CliInstantiateArgs, ExtrinsicArgs, InfoArgs, MapAccountArgs, RemoveArgs,
    RpcArgs, UploadArgs,
};

type Conf = subxt::config::SubstrateConfig;
type Signer = Keypair;

fn parse_signer(suri: &str) -> Result<Signer> {
    use std::str::FromStr;
    let uri = subxt_signer::SecretUri::from_str(suri)
        .map_err(|e| anyhow::anyhow!("Invalid SURI '{suri}': {e}"))?;
    Keypair::from_uri(&uri)
        .map_err(|e| anyhow::anyhow!("Failed to create keypair from '{suri}': {e}"))
}

fn build_opts(
    args: &ExtrinsicArgs,
) -> Result<cargo_pvm_contract_extrinsics::ExtrinsicOpts<Conf, Signer>> {
    let signer = parse_signer(&args.suri)?;
    let url = url::Url::parse(&args.url).context("Invalid node URL")?;
    let opts = ExtrinsicOptsBuilder::<Conf, Signer>::new(signer)
        .url(url)
        .storage_deposit_limit(args.storage_deposit_limit)
        .done();
    Ok(opts)
}

fn hex_to_bytes(hex_str: &str) -> Result<Vec<u8>> {
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    hex::decode(hex_str).context("Invalid hex string")
}

fn build_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("Failed to create tokio runtime")
}

pub fn upload_command(args: UploadArgs) -> Result<()> {
    let code = std::fs::read(&args.code)
        .with_context(|| format!("Failed to read contract binary: {}", args.code.display()))?;
    let opts = build_opts(&args.extrinsic)?;
    let rt = build_runtime();

    rt.block_on(async {
        let mut exec = UploadCommandBuilder::new(opts, ContractBinary(code.clone()))
            .done()
            .await?;

        if args.dry_run {
            let _result = exec.upload_code_rpc().await?;
            let code_hash = ContractBinary(code).code_hash();
            println!("Upload dry-run succeeded");
            println!("  Code hash: 0x{}", hex::encode(code_hash));
        } else {
            let _result = exec
                .upload_code()
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let code_hash = ContractBinary(code).code_hash();
            println!("Code uploaded successfully");
            println!("  Code hash: 0x{}", hex::encode(code_hash));
        }
        Ok(())
    })
}

pub fn instantiate_command(args: CliInstantiateArgs) -> Result<()> {
    let code_bytes = std::fs::read(&args.code)
        .with_context(|| format!("Failed to read contract binary: {}", args.code.display()))?;
    let data = match &args.data {
        Some(hex) => hex_to_bytes(hex)?,
        None => vec![],
    };
    let opts = build_opts(&args.extrinsic)?;
    let rt = build_runtime();

    rt.block_on(async {
        let mut builder =
            InstantiateCommandBuilder::new(opts, Code::Upload(code_bytes), data).value(args.value);

        if let Some(ref salt_hex) = args.salt {
            builder = builder.salt(Some(hex_to_bytes(salt_hex)?));
        }

        let exec = builder.done().await?;

        if args.dry_run {
            let result = exec.instantiate_dry_run().await?;
            let weight = result.weight_required;
            println!("Instantiate dry-run succeeded");
            println!(
                "  Result: {}",
                if result.result.is_ok() {
                    "success"
                } else {
                    "failed"
                }
            );
            println!(
                "  Gas required: ref_time={}, proof_size={}",
                weight.ref_time(),
                weight.proof_size()
            );
        } else {
            let result = exec
                .instantiate(None, None)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("Contract instantiated successfully");
            println!("  Contract address: {:?}", result.contract_address);
        }
        Ok(())
    })
}

pub fn call_command(args: CallArgs) -> Result<()> {
    let contract = AddrKind::from(&args.contract)?.to_h160();
    let call_data = hex_to_bytes(&args.data)?;
    let opts = build_opts(&args.extrinsic)?;
    let rt = build_runtime();

    rt.block_on(async {
        let exec = CallCommandBuilder::new(contract, call_data, opts)
            .value(args.value)
            .done()
            .await?;

        if args.dry_run {
            let result = exec.call_dry_run().await?;
            match result.result {
                Ok(ref exec_result) => {
                    println!("Call dry-run succeeded");
                    println!("  Reverted: {}", exec_result.did_revert());
                    println!("  Output: 0x{}", hex::encode(&exec_result.data));
                    println!(
                        "  Gas required: ref_time={}, proof_size={}",
                        result.weight_required.ref_time(),
                        result.weight_required.proof_size()
                    );
                }
                Err(ref err) => {
                    println!("Call dry-run failed: {err:?}");
                }
            }
        } else {
            let _events = exec
                .call(None, None)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("Call executed successfully");
        }
        Ok(())
    })
}

pub fn remove_command(args: RemoveArgs) -> Result<()> {
    let hash_bytes = hex_to_bytes(&args.code_hash)?;
    if hash_bytes.len() != 32 {
        anyhow::bail!("Code hash must be 32 bytes, got {}", hash_bytes.len());
    }
    let code_hash = sp_core::H256::from_slice(&hash_bytes);
    let opts = build_opts(&args.extrinsic)?;
    let rt = build_runtime();

    rt.block_on(async {
        let exec = RemoveCommandBuilder::new(opts, code_hash).done().await?;
        exec.remove_code()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("Code removed successfully");
        println!("  Code hash: {}", args.code_hash);
        Ok(())
    })
}

pub fn map_account_command(args: MapAccountArgs) -> Result<()> {
    let opts = build_opts(&args.extrinsic)?;
    let rt = build_runtime();

    rt.block_on(async {
        let exec = MapAccountCommandBuilder::new(opts).done().await?;

        if args.dry_run {
            let fee = exec.map_account_dry_run().await?;
            println!("Map account dry-run succeeded");
            println!("  Estimated fee: {fee}");
        } else {
            match exec.map_account().await {
                Ok(result) => {
                    println!("Account mapped successfully");
                    println!("  EVM address: {:?}", result.address);
                }
                Err(e) => {
                    let err_str = format!("{e}");
                    if err_str.contains("AccountAlreadyMapped") {
                        println!("Account already mapped");
                    } else {
                        return Err(anyhow::anyhow!("{e}"));
                    }
                }
            }
        }
        Ok(())
    })
}

pub fn info_command(args: InfoArgs) -> Result<()> {
    let contract = AddrKind::from(&args.contract)?.to_h160();
    let url = url::Url::parse(&args.url).context("Invalid node URL")?;
    let rt = build_runtime();

    rt.block_on(async {
        let rpc_cli = subxt::backend::rpc::RpcClient::from_url(
            cargo_pvm_contract_extrinsics::url_to_string(&url),
        )
        .await?;
        let client = subxt::OnlineClient::<Conf>::from_rpc_client(rpc_cli.clone()).await?;
        let rpc = subxt::backend::legacy::LegacyRpcMethods::<Conf>::new(rpc_cli);

        let info =
            cargo_pvm_contract_extrinsics::fetch_contract_info(&contract, &rpc, &client).await?;
        println!("{}", info.to_json()?);
        Ok(())
    })
}

pub fn rpc_command(args: RpcArgs) -> Result<()> {
    let url = url::Url::parse(&args.url).context("Invalid node URL")?;
    let rt = build_runtime();

    rt.block_on(async {
        let rpc = cargo_pvm_contract_extrinsics::RpcRequest::new(&url).await?;
        let params = cargo_pvm_contract_extrinsics::RawParams::new(&args.params)?;
        let result = rpc.raw_call(&args.method, params).await?;
        println!("{}", result.get());
        Ok(())
    })
}

/// Determine whether the address is an H160 (0x-prefixed, 20 bytes) or an SS58 Substrate
/// account.
enum AddrKind {
    H160(sp_core::H160),
    Substrate(subxt::utils::AccountId32),
}

impl AddrKind {
    fn from(addr: &str) -> Result<Self> {
        if addr.starts_with("0x") || addr.starts_with("0X") {
            let bytes = hex_to_bytes(addr)?;
            if bytes.len() == 20 {
                Ok(AddrKind::H160(sp_core::H160::from_slice(&bytes)))
            } else {
                anyhow::bail!("H160 address must be 20 bytes, got {}", bytes.len());
            }
        } else {
            let account_id: subxt::utils::AccountId32 = addr.parse().map_err(|_| {
                anyhow::anyhow!("Invalid address: not a valid H160 or SS58 address")
            })?;
            Ok(AddrKind::Substrate(account_id))
        }
    }

    fn to_h160(&self) -> H160 {
        match self {
            AddrKind::H160(h) => *h,
            AddrKind::Substrate(id) => {
                cargo_pvm_contract_extrinsics::AccountIdMapper::to_address(&id.0)
            }
        }
    }

    async fn to_account_id(
        &self,
        rpc: &subxt::backend::legacy::LegacyRpcMethods<Conf>,
        client: &subxt::OnlineClient<Conf>,
    ) -> Result<subxt::utils::AccountId32> {
        match self {
            AddrKind::H160(h160) => {
                cargo_pvm_contract_extrinsics::resolve_h160(h160, rpc, client).await
            }
            AddrKind::Substrate(id) => Ok(id.clone()),
        }
    }
}

pub fn account_command(args: AccountArgs) -> Result<()> {
    let addr_kind = AddrKind::from(&args.addr)?;
    let url = url::Url::parse(&args.url).context("Invalid node URL")?;
    let rt = build_runtime();

    rt.block_on(async {
        let rpc_cli = subxt::backend::rpc::RpcClient::from_url(
            cargo_pvm_contract_extrinsics::url_to_string(&url),
        )
        .await?;
        let client = subxt::OnlineClient::<Conf>::from_rpc_client(rpc_cli.clone()).await?;
        let rpc = subxt::backend::legacy::LegacyRpcMethods::<Conf>::new(rpc_cli);

        let account_id = addr_kind.to_account_id(&rpc, &client).await?;

        let account_data =
            cargo_pvm_contract_extrinsics::get_account_data(&account_id, &rpc, &client).await?;

        if args.output_json {
            let output = serde_json::json!({
                "account_id": format!("{account_id}"),
                "free": account_data.free.to_string(),
                "reserved": account_data.reserved.to_string(),
                "frozen": account_data.frozen.to_string(),
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("Account Id: {account_id}");
            println!("Free Balance: {}", account_data.free);
            println!("Reserved: {}", account_data.reserved);
            println!("Frozen: {}", account_data.frozen);
        }
        Ok(())
    })
}

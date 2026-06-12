mod balance;
mod call;
mod contract_info;
mod error;
pub mod events;
pub mod extrinsic_calls;
mod extrinsic_opts;
mod instantiate;
mod map_account;
pub mod pallet_revive_primitives;
mod remove;
mod rpc;
pub mod upload;

use anyhow::Result;
pub use balance::{BalanceVariant, TokenMetadata};
pub use call::{CallCommandBuilder, CallExec};
pub use contract_info::{
    AccountData, CodeInfo, ContractInfo, TrieId, fetch_all_contracts, fetch_code_info,
    fetch_contract_binary, fetch_contract_info, get_account_data, resolve_h160,
};
pub use error::{ErrorVariant, GenericError};
pub use events::DisplayEvents;
pub use extrinsic_calls::{Call, Instantiate, InstantiateWithCode, UploadCode};
pub use extrinsic_opts::{ExtrinsicOpts, ExtrinsicOptsBuilder};
pub use instantiate::{
    Code, InstantiateArgs, InstantiateCommandBuilder, InstantiateDryRunResult, InstantiateExec,
    InstantiateExecResult,
};
pub use map_account::{MapAccountCommandBuilder, MapAccountExec, MapAccountExecResult};
pub use remove::{RemoveCommandBuilder, RemoveExec};
pub use rpc::{RawParams, RpcRequest};
pub use sp_runtime::DispatchError;
pub use upload::{UploadCommandBuilder, UploadExec, UploadResult};

use scale::{Decode, Encode};
use sp_core::{H160, keccak_256};
use subxt::{
    Config, OnlineClient,
    backend::legacy::LegacyRpcMethods,
    blocks,
    config::{DefaultExtrinsicParams, DefaultExtrinsicParamsBuilder, ExtrinsicParams, HashFor},
    ext::subxt_rpcs::methods::legacy::DryRunResultBytes,
    tx,
};

/// The binary of a contract (compiled for PolkaVM).
#[derive(Debug, Clone)]
pub struct ContractBinary(pub Vec<u8>);

impl ContractBinary {
    /// The hash of the contract code: uniquely identifies the contract code on-chain.
    pub fn code_hash(&self) -> [u8; 32] {
        use tiny_keccak::{Hasher, Keccak};
        let mut hasher = Keccak::v256();
        hasher.update(&self.0);
        let mut output = [0u8; 32];
        hasher.finalize(&mut output);
        output
    }
}

/// Wait for the transaction to be included successfully into a block.
///
/// Currently reports success once the transaction is included in a block (not finalized).
async fn submit_extrinsic<C, Call, Signer>(
    client: &OnlineClient<C>,
    rpc: &LegacyRpcMethods<C>,
    call: &Call,
    signer: &Signer,
) -> core::result::Result<blocks::ExtrinsicEvents<C>, subxt::Error>
where
    C: Config,
    Call: tx::Payload,
    Signer: tx::Signer<C>,
    <C::ExtrinsicParams as ExtrinsicParams<C>>::Params:
        From<<DefaultExtrinsicParams<C> as ExtrinsicParams<C>>::Params>,
{
    let account_id = Signer::account_id(signer);
    let account_nonce = get_account_nonce(client, rpc, &account_id).await?;

    let params = DefaultExtrinsicParamsBuilder::new()
        .nonce(account_nonce)
        .build();
    let mut tx = client
        .tx()
        .create_partial_offline(call, params.into())?
        .sign(signer)
        .submit_and_watch()
        .await?;

    use subxt::error::{RpcError, TransactionError};
    use tx::TxStatus;

    while let Some(status) = tx.next().await {
        match status? {
            TxStatus::InBestBlock(tx_in_block) | TxStatus::InFinalizedBlock(tx_in_block) => {
                let events = tx_in_block.wait_for_success().await?;
                return Ok(events);
            }
            TxStatus::Error { message } => return Err(TransactionError::Error(message).into()),
            TxStatus::Invalid { message } => return Err(TransactionError::Invalid(message).into()),
            TxStatus::Dropped { message } => return Err(TransactionError::Dropped(message).into()),
            _ => continue,
        }
    }
    Err(RpcError::SubscriptionDropped.into())
}

/// Dry-run an extrinsic, returning the result bytes and estimated fee.
async fn dry_run_extrinsic<C, Call, Signer>(
    client: &OnlineClient<C>,
    rpc: &LegacyRpcMethods<C>,
    call: &Call,
    signer: &Signer,
) -> core::result::Result<(DryRunResultBytes, u128), subxt::Error>
where
    C: Config,
    Call: tx::Payload,
    Signer: tx::Signer<C>,
    <C::ExtrinsicParams as ExtrinsicParams<C>>::Params:
        From<<DefaultExtrinsicParams<C> as ExtrinsicParams<C>>::Params>,
{
    let account_id = Signer::account_id(signer);
    let account_nonce = get_account_nonce(client, rpc, &account_id).await?;

    let params = DefaultExtrinsicParamsBuilder::new()
        .nonce(account_nonce)
        .build();
    let extrinsic = client
        .tx()
        .create_partial_offline(call, params.into())?
        .sign(signer);
    let result = rpc.dry_run(extrinsic.encoded(), None).await?;
    let partial_fee_estimate = extrinsic.partial_fee_estimate().await?;
    Ok((result, partial_fee_estimate))
}

/// Return the account nonce at the *best* block for an account ID.
///
/// NOTE: This reads the nonce at a point in time and is not protected against
/// concurrent submissions with the same signer. If two extrinsics are submitted
/// concurrently using the same account, they may read the same nonce and one
/// will be rejected as stale. Callers must ensure sequential submission per signer.
async fn get_account_nonce<C>(
    client: &OnlineClient<C>,
    rpc: &LegacyRpcMethods<C>,
    account_id: &C::AccountId,
) -> core::result::Result<u64, subxt::Error>
where
    C: Config,
{
    let best_block = rpc
        .chain_get_block_hash(None)
        .await?
        .ok_or(subxt::Error::Other("Best block not found".into()))?;
    let account_nonce = client
        .blocks()
        .at(best_block)
        .await?
        .account_nonce(account_id)
        .await?;
    Ok(account_nonce)
}

async fn state_call<C, A: Encode, R: Decode>(
    rpc: &LegacyRpcMethods<C>,
    func: &str,
    args: A,
) -> Result<R>
where
    C: Config,
{
    let params = args.encode();
    let bytes = rpc.state_call(func, Some(&params), None).await?;
    Ok(R::decode(&mut bytes.as_ref())?)
}

/// Fetch the hash of the *best* block (included but not guaranteed to be finalized).
async fn get_best_block<C>(
    rpc: &LegacyRpcMethods<C>,
) -> core::result::Result<HashFor<C>, subxt::Error>
where
    C: Config,
{
    rpc.chain_get_block_hash(None)
        .await?
        .ok_or(subxt::Error::Other("Best block not found".into()))
}

/// Converts a Url into a String representation without excluding the default port.
pub fn url_to_string(url: &url::Url) -> String {
    match (url.port(), url.port_or_known_default()) {
        (None, Some(port)) => {
            format!(
                "{}:{port}{}",
                &url[..url::Position::AfterHost],
                &url[url::Position::BeforePath..]
            )
        }
        _ => url.to_string(),
    }
}

pub struct AccountIdMapper;

impl AccountIdMapper {
    pub fn to_address(account_id: &[u8; 32]) -> H160 {
        if Self::is_eth_derived(account_id) {
            H160::from_slice(&account_id[..20])
        } else {
            let account_hash = keccak_256(account_id);
            H160::from_slice(&account_hash[12..])
        }
    }

    fn is_eth_derived(account_bytes: &[u8; 32]) -> bool {
        account_bytes[20..] == [0xEE; 12]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_to_string_works() {
        let url = url::Url::parse("ws://127.0.0.1:9944").unwrap();
        assert_eq!(url_to_string(&url), "ws://127.0.0.1:9944/");

        let url = url::Url::parse("wss://127.0.0.1:443").unwrap();
        assert_eq!(url_to_string(&url), "wss://127.0.0.1:443/");

        let url = url::Url::parse("wss://test.io/test/1").unwrap();
        assert_eq!(url_to_string(&url), "wss://test.io:443/test/1");
    }
}

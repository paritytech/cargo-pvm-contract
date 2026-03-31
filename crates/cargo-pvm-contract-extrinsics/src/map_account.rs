use super::{AccountIdMapper, ErrorVariant, dry_run_extrinsic, submit_extrinsic};
use crate::{extrinsic_calls::MapAccount, extrinsic_opts::ExtrinsicOpts};
use anyhow::Result;
use scale::Encode;
use subxt::{
    Config, OnlineClient,
    backend::{
        legacy::{LegacyRpcMethods, rpc_methods::DryRunResult},
        rpc::RpcClient,
    },
    blocks::ExtrinsicEvents,
    config::{DefaultExtrinsicParams, ExtrinsicParams},
    ext::subxt_rpcs::methods::legacy::DryRunDecodeError,
    tx,
    utils::H160,
};

/// A builder for the map_account command.
pub struct MapAccountCommandBuilder<C: Config, Signer: Clone> {
    extrinsic_opts: ExtrinsicOpts<C, Signer>,
}

impl<C: Config, Signer> MapAccountCommandBuilder<C, Signer>
where
    Signer: tx::Signer<C> + Clone,
{
    pub fn new(extrinsic_opts: ExtrinsicOpts<C, Signer>) -> MapAccountCommandBuilder<C, Signer> {
        MapAccountCommandBuilder { extrinsic_opts }
    }

    /// Connects to the node and prepares for map_account.
    pub async fn done(self) -> Result<MapAccountExec<C, Signer>> {
        let url = self.extrinsic_opts.url();
        let rpc_cli = RpcClient::from_url(&url).await?;
        let client = OnlineClient::from_rpc_client(rpc_cli.clone()).await?;
        let rpc = LegacyRpcMethods::new(rpc_cli);

        Ok(MapAccountExec {
            opts: self.extrinsic_opts,
            rpc,
            client,
        })
    }
}

pub struct MapAccountExec<C: Config, Signer: Clone> {
    opts: ExtrinsicOpts<C, Signer>,
    rpc: LegacyRpcMethods<C>,
    client: OnlineClient<C>,
}

impl<C: Config, Signer> MapAccountExec<C, Signer>
where
    <C::ExtrinsicParams as ExtrinsicParams<C>>::Params:
        From<<DefaultExtrinsicParams<C> as ExtrinsicParams<C>>::Params>,
    Signer: tx::Signer<C> + Clone,
{
    /// Dry-run the map_account call, returning the estimated fee.
    pub async fn map_account_dry_run(&self) -> Result<u128> {
        let call = MapAccount::new().build();
        let (bytes, partial_fee_estimation) =
            dry_run_extrinsic(&self.client, &self.rpc, &call, self.opts.signer()).await?;
        let res = bytes.into_dry_run_result();
        match res {
            Ok(DryRunResult::Success) => Ok(partial_fee_estimation),
            Ok(DryRunResult::DispatchError(err)) => {
                Err(anyhow::format_err!("dispatch error: {err:?}"))
            }
            Ok(DryRunResult::TransactionValidityError) => {
                Err(anyhow::anyhow!("transaction validity error"))
            }
            Err(err) => match err {
                DryRunDecodeError::WrongNumberOfBytes => Err(anyhow::anyhow!(
                    "decode error: dry run result was less than 2 bytes"
                )),
                DryRunDecodeError::InvalidBytes => {
                    Err(anyhow::anyhow!("decode error: dry run bytes are not valid"))
                }
            },
        }
    }

    /// Submit the map_account extrinsic.
    pub async fn map_account(&self) -> Result<MapAccountExecResult<C>, ErrorVariant> {
        let call = MapAccount::new().build();
        let events = submit_extrinsic(&self.client, &self.rpc, &call, self.opts.signer()).await?;
        let account_id = self.opts.signer().account_id();
        Ok(MapAccountExecResult {
            events,
            address: AccountIdMapper::to_address(
                &account_id.encode()[..]
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("AccountId must be 32 bytes"))?,
            ),
        })
    }

    pub fn opts(&self) -> &ExtrinsicOpts<C, Signer> {
        &self.opts
    }

    pub fn client(&self) -> &OnlineClient<C> {
        &self.client
    }
}

/// Result of a map_account execution.
pub struct MapAccountExecResult<C: Config> {
    pub events: ExtrinsicEvents<C>,
    pub address: H160,
}

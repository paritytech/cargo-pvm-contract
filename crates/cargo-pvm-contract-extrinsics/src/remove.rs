use super::{ErrorVariant, submit_extrinsic};
use crate::{extrinsic_calls::RemoveCode, extrinsic_opts::ExtrinsicOpts};

use anyhow::Result;
use subxt::{
    Config, OnlineClient,
    backend::{legacy::LegacyRpcMethods, rpc::RpcClient},
    blocks::ExtrinsicEvents,
    config::{DefaultExtrinsicParams, ExtrinsicParams},
    tx,
    utils::H256,
};

/// A builder for the remove command.
pub struct RemoveCommandBuilder<C: Config, Signer: Clone> {
    code_hash: H256,
    extrinsic_opts: ExtrinsicOpts<C, Signer>,
}

impl<C: Config, Signer> RemoveCommandBuilder<C, Signer>
where
    Signer: tx::Signer<C> + Clone,
{
    /// Returns a clean builder for [`RemoveExec`].
    pub fn new(
        extrinsic_opts: ExtrinsicOpts<C, Signer>,
        code_hash: H256,
    ) -> RemoveCommandBuilder<C, Signer> {
        RemoveCommandBuilder {
            code_hash,
            extrinsic_opts,
        }
    }

    /// Connects to the node and prepares for removal.
    pub async fn done(self) -> Result<RemoveExec<C, Signer>> {
        let url = self.extrinsic_opts.url();
        let rpc_cli = RpcClient::from_url(&url).await?;
        let client = OnlineClient::<C>::from_rpc_client(rpc_cli.clone()).await?;
        let rpc = LegacyRpcMethods::<C>::new(rpc_cli);

        Ok(RemoveExec {
            final_code_hash: self.code_hash,
            opts: self.extrinsic_opts,
            rpc,
            client,
        })
    }
}

pub struct RemoveExec<C: Config, Signer: Clone> {
    final_code_hash: H256,
    opts: ExtrinsicOpts<C, Signer>,
    rpc: LegacyRpcMethods<C>,
    client: OnlineClient<C>,
}

impl<C: Config, Signer> RemoveExec<C, Signer>
where
    <C::ExtrinsicParams as ExtrinsicParams<C>>::Params:
        From<<DefaultExtrinsicParams<C> as ExtrinsicParams<C>>::Params>,
    Signer: tx::Signer<C> + Clone,
{
    /// Removes a contract code from the blockchain.
    pub async fn remove_code(&self) -> Result<ExtrinsicEvents<C>, ErrorVariant> {
        let code_hash = self.final_code_hash;
        let call = RemoveCode::new(code_hash).build();
        let events = submit_extrinsic(&self.client, &self.rpc, &call, self.opts.signer()).await?;
        Ok(events)
    }

    pub fn final_code_hash(&self) -> H256 {
        self.final_code_hash
    }

    pub fn opts(&self) -> &ExtrinsicOpts<C, Signer> {
        &self.opts
    }

    pub fn client(&self) -> &OnlineClient<C> {
        &self.client
    }
}

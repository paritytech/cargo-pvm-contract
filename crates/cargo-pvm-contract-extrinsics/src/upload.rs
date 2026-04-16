use super::{
    ContractBinary, ErrorVariant, pallet_revive_primitives::CodeUploadResult, state_call,
    submit_extrinsic,
};
use crate::{extrinsic_calls::UploadCode, extrinsic_opts::ExtrinsicOpts};
use anyhow::Result;
use scale::Encode;
use subxt::{
    Config, OnlineClient,
    backend::{legacy::LegacyRpcMethods, rpc::RpcClient},
    blocks::ExtrinsicEvents,
    config::{DefaultExtrinsicParams, ExtrinsicParams},
    tx,
};

/// A builder for the upload command.
pub struct UploadCommandBuilder<C: Config, Signer: Clone> {
    extrinsic_opts: ExtrinsicOpts<C, Signer>,
    code: ContractBinary,
}

impl<C: Config, Signer> UploadCommandBuilder<C, Signer>
where
    Signer: tx::Signer<C> + Clone,
{
    /// Returns a clean builder for [`UploadExec`].
    pub fn new(
        extrinsic_opts: ExtrinsicOpts<C, Signer>,
        code: ContractBinary,
    ) -> UploadCommandBuilder<C, Signer> {
        UploadCommandBuilder {
            extrinsic_opts,
            code,
        }
    }

    /// Connects to the node and prepares for upload.
    pub async fn done(self) -> Result<UploadExec<C, Signer>> {
        let url = self.extrinsic_opts.url();
        let rpc_cli = RpcClient::from_url(&url).await?;
        let client = OnlineClient::from_rpc_client(rpc_cli.clone()).await?;
        let rpc = LegacyRpcMethods::new(rpc_cli);

        Ok(UploadExec {
            opts: self.extrinsic_opts,
            rpc,
            client,
            code: self.code,
        })
    }
}

pub struct UploadExec<C: Config, Signer: Clone> {
    opts: ExtrinsicOpts<C, Signer>,
    rpc: LegacyRpcMethods<C>,
    client: OnlineClient<C>,
    code: ContractBinary,
}

impl<C: Config, Signer> UploadExec<C, Signer>
where
    <C::ExtrinsicParams as ExtrinsicParams<C>>::Params:
        From<<DefaultExtrinsicParams<C> as ExtrinsicParams<C>>::Params>,
    Signer: tx::Signer<C> + Clone,
{
    /// Uploads contract code via a `ReviveApi_upload_code` RPC state call (dry-run).
    pub async fn upload_code_rpc(&self) -> Result<CodeUploadResult<u128>> {
        let storage_deposit_limit = self.opts.storage_deposit_limit();
        let call_request = CodeUploadRequest {
            origin: self.opts.signer().account_id(),
            code: self.code.0.clone(),
            storage_deposit_limit,
        };
        state_call(&self.rpc, "ReviveApi_upload_code", call_request).await
    }

    /// Uploads contract code to the blockchain via an extrinsic.
    ///
    /// If no storage deposit limit was provided, a dry-run is performed first
    /// to estimate the required deposit.
    pub async fn upload_code(&mut self) -> Result<UploadResult<C>, ErrorVariant> {
        if self.opts.storage_deposit_limit().is_none() {
            let dry_run_result = self
                .upload_code_rpc()
                .await
                .map_err(|e| {
                    ErrorVariant::from(anyhow::anyhow!(
                        "Dry-run to estimate storage deposit limit failed: {e}"
                    ))
                })?
                .map_err(|e| {
                    ErrorVariant::from(anyhow::anyhow!(
                        "Upload dry-run returned dispatch error: {e:?}"
                    ))
                })?;
            self.set_storage_deposit_limit(Some(dry_run_result.deposit));
        }
        let storage_deposit_limit = self.opts.storage_deposit_limit().expect("set above");

        let call = UploadCode::new(self.code.clone(), storage_deposit_limit).build();

        let events = submit_extrinsic(&self.client, &self.rpc, &call, self.opts.signer()).await?;
        tracing::debug!("events: {:?}", events);

        Ok(UploadResult { events })
    }

    /// Returns the extrinsic options.
    pub fn opts(&self) -> &ExtrinsicOpts<C, Signer> {
        &self.opts
    }

    /// Returns the client.
    pub fn client(&self) -> &OnlineClient<C> {
        &self.client
    }

    /// Returns the code.
    pub fn code(&self) -> &ContractBinary {
        &self.code
    }

    /// Sets a new storage deposit limit.
    pub fn set_storage_deposit_limit(&mut self, limit: Option<u128>) {
        self.opts.set_storage_deposit_limit(limit);
    }
}

/// A struct that encodes RPC parameters required for a call to upload a new code.
#[derive(Encode)]
struct CodeUploadRequest<AccountId> {
    origin: AccountId,
    code: Vec<u8>,
    storage_deposit_limit: Option<u128>,
}

/// A struct representing the result of an upload command execution.
pub struct UploadResult<C: Config> {
    pub events: ExtrinsicEvents<C>,
}

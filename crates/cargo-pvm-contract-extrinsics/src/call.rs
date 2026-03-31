use super::{
    ErrorVariant,
    pallet_revive_primitives::{ContractExecResult, StorageDeposit},
    state_call, submit_extrinsic,
};
use crate::{extrinsic_calls::Call, extrinsic_opts::ExtrinsicOpts};

use anyhow::{Result, anyhow};
use scale::Encode;
use sp_weights::Weight;

use subxt::{
    Config, OnlineClient,
    backend::{legacy::LegacyRpcMethods, rpc::RpcClient},
    blocks::ExtrinsicEvents,
    config::{DefaultExtrinsicParams, ExtrinsicParams},
    tx,
    utils::H160,
};

/// A builder for the call command.
pub struct CallCommandBuilder<C: Config, Signer: Clone> {
    contract: H160,
    call_data: Vec<u8>,
    extrinsic_opts: ExtrinsicOpts<C, Signer>,
    gas_limit: Option<u64>,
    proof_size: Option<u64>,
    value: u128,
}

impl<C: Config, Signer> CallCommandBuilder<C, Signer>
where
    Signer: tx::Signer<C> + Clone,
{
    /// Returns a clean builder for [`CallExec`].
    ///
    /// `call_data` should be the ABI-encoded function selector + arguments.
    pub fn new(
        contract: H160,
        call_data: Vec<u8>,
        extrinsic_opts: ExtrinsicOpts<C, Signer>,
    ) -> CallCommandBuilder<C, Signer> {
        CallCommandBuilder {
            contract,
            call_data,
            extrinsic_opts,
            gas_limit: None,
            proof_size: None,
            value: 0,
        }
    }

    /// Sets the maximum amount of gas to be used for this command.
    pub fn gas_limit(mut self, gas_limit: Option<u64>) -> Self {
        self.gas_limit = gas_limit;
        self
    }

    /// Sets the maximum proof size for this call.
    pub fn proof_size(mut self, proof_size: Option<u64>) -> Self {
        self.proof_size = proof_size;
        self
    }

    /// Sets the value to be transferred as part of the call.
    pub fn value(mut self, value: u128) -> Self {
        self.value = value;
        self
    }

    /// Connects to the node and prepares for the call.
    pub async fn done(self) -> Result<CallExec<C, Signer>> {
        let url = self.extrinsic_opts.url();
        let rpc = RpcClient::from_url(&url).await?;
        let client = OnlineClient::from_rpc_client(rpc.clone()).await?;
        let rpc = LegacyRpcMethods::new(rpc);

        Ok(CallExec {
            contract: self.contract,
            opts: self.extrinsic_opts,
            gas_limit: self.gas_limit,
            proof_size: self.proof_size,
            value: self.value,
            rpc,
            client,
            call_data: self.call_data,
        })
    }
}

pub struct CallExec<C: Config, Signer: Clone> {
    contract: H160,
    opts: ExtrinsicOpts<C, Signer>,
    gas_limit: Option<u64>,
    proof_size: Option<u64>,
    value: u128,
    rpc: LegacyRpcMethods<C>,
    client: OnlineClient<C>,
    call_data: Vec<u8>,
}

impl<C: Config, Signer> CallExec<C, Signer>
where
    <C::ExtrinsicParams as ExtrinsicParams<C>>::Params:
        From<<DefaultExtrinsicParams<C> as ExtrinsicParams<C>>::Params>,
    Signer: tx::Signer<C> + Clone,
{
    /// Simulates a contract call without modifying the blockchain.
    pub async fn call_dry_run(&self) -> Result<ContractExecResult<u128>> {
        let storage_deposit_limit = self.opts.storage_deposit_limit();
        let call_request = CallRequest {
            origin: self.opts.signer().account_id(),
            dest: self.contract,
            value: self.value,
            gas_limit: None,
            storage_deposit_limit,
            input_data: self.call_data.clone(),
        };
        state_call(&self.rpc, "ReviveApi_call", call_request).await
    }

    /// Calls a contract on the blockchain with a specified gas limit.
    pub async fn call(
        &self,
        gas_limit: Option<Weight>,
        storage_deposit_limit: Option<u128>,
    ) -> Result<ExtrinsicEvents<C>, ErrorVariant> {
        let estimate = self.estimate_gas().await?;
        let gas_limit = gas_limit.unwrap_or(estimate.0);
        let storage_deposit_limit = storage_deposit_limit.unwrap_or(estimate.1);

        tracing::debug!("calling contract {:?}", self.contract);

        let call = Call::new(
            self.contract,
            self.value,
            gas_limit,
            storage_deposit_limit,
            self.call_data.clone(),
        )
        .build();

        let result = submit_extrinsic(&self.client, &self.rpc, &call, self.opts.signer()).await?;

        Ok(result)
    }

    /// Estimates the gas required for a contract call.
    pub async fn estimate_gas(&self) -> Result<(Weight, u128)> {
        match (
            self.gas_limit,
            self.proof_size,
            self.opts.storage_deposit_limit(),
        ) {
            (Some(ref_time), Some(proof_size), Some(deposit_limit)) => {
                Ok((Weight::from_parts(ref_time, proof_size), deposit_limit))
            }
            _ => {
                let call_result = self.call_dry_run().await?;
                match call_result.result {
                    Ok(_) => {
                        let ref_time = self
                            .gas_limit
                            .unwrap_or_else(|| call_result.weight_required.ref_time());
                        let proof_size = self
                            .proof_size
                            .unwrap_or_else(|| call_result.weight_required.proof_size());
                        let storage_deposit_limit = self.opts.storage_deposit_limit().unwrap_or(
                            match call_result.storage_deposit {
                                StorageDeposit::Refund(_) => 0,
                                StorageDeposit::Charge(charge) => charge,
                            },
                        );
                        Ok((
                            Weight::from_parts(ref_time, proof_size),
                            storage_deposit_limit,
                        ))
                    }
                    Err(ref err) => {
                        let object =
                            ErrorVariant::from_dispatch_error(err, &self.client.metadata())?;
                        Err(anyhow!("Pre-submission dry-run failed. Error: {object}"))
                    }
                }
            }
        }
    }

    pub fn contract(&self) -> &H160 {
        &self.contract
    }

    pub fn opts(&self) -> &ExtrinsicOpts<C, Signer> {
        &self.opts
    }

    pub fn gas_limit(&self) -> Option<u64> {
        self.gas_limit
    }

    pub fn proof_size(&self) -> Option<u64> {
        self.proof_size
    }

    pub fn value(&self) -> u128 {
        self.value
    }

    pub fn client(&self) -> &OnlineClient<C> {
        &self.client
    }

    pub fn rpc(&self) -> &LegacyRpcMethods<C> {
        &self.rpc
    }

    pub fn call_data(&self) -> &[u8] {
        &self.call_data
    }
}

/// A struct that encodes RPC parameters required for a call to a smart contract.
#[derive(Encode)]
struct CallRequest<AccountId> {
    origin: AccountId,
    dest: H160,
    value: u128,
    gas_limit: Option<Weight>,
    storage_deposit_limit: Option<u128>,
    input_data: Vec<u8>,
}

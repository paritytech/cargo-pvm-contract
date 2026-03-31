use super::{
    ErrorVariant,
    events::ContractInstantiated,
    pallet_revive_primitives::{ContractInstantiateResult, StorageDeposit},
    state_call, submit_extrinsic,
};
use crate::{
    extrinsic_calls::{Instantiate, InstantiateWithCode},
    extrinsic_opts::ExtrinsicOpts,
};
use anyhow::{Result, anyhow};
use scale::Encode;
use sp_weights::Weight;
use subxt::{
    Config, OnlineClient,
    backend::{legacy::LegacyRpcMethods, rpc::RpcClient},
    blocks::ExtrinsicEvents,
    config::{DefaultExtrinsicParams, ExtrinsicParams, HashFor},
    ext::scale_decode::IntoVisitor,
    tx,
    utils::{H160, H256},
};

/// A builder for the instantiate command.
pub struct InstantiateCommandBuilder<C: Config, Signer: Clone> {
    extrinsic_opts: ExtrinsicOpts<C, Signer>,
    value: u128,
    gas_limit: Option<u64>,
    proof_size: Option<u64>,
    salt: Option<Vec<u8>>,
    code: Code,
    data: Vec<u8>,
}

impl<C: Config, Signer> InstantiateCommandBuilder<C, Signer>
where
    Signer: tx::Signer<C> + Clone,
    HashFor<C>: From<[u8; 32]>,
{
    /// Returns a clean builder for [`InstantiateExec`].
    ///
    /// `code` is either the raw bytecode to upload or a reference to an existing code hash.
    /// `data` is the ABI-encoded constructor arguments.
    pub fn new(
        extrinsic_opts: ExtrinsicOpts<C, Signer>,
        code: Code,
        data: Vec<u8>,
    ) -> InstantiateCommandBuilder<C, Signer> {
        InstantiateCommandBuilder {
            extrinsic_opts,
            value: 0,
            gas_limit: None,
            proof_size: None,
            salt: None,
            code,
            data,
        }
    }

    /// Sets the initial balance to transfer to the instantiated contract.
    pub fn value(mut self, value: u128) -> Self {
        self.value = value;
        self
    }

    /// Sets the maximum amount of gas to be used for this command.
    pub fn gas_limit(mut self, gas_limit: Option<u64>) -> Self {
        self.gas_limit = gas_limit;
        self
    }

    /// Sets the maximum proof size for this instantiation.
    pub fn proof_size(mut self, proof_size: Option<u64>) -> Self {
        self.proof_size = proof_size;
        self
    }

    /// Sets the salt used in the address derivation of the new contract.
    pub fn salt(mut self, salt: Option<Vec<u8>>) -> Self {
        self.salt = salt;
        self
    }

    /// Connects to the node and prepares for instantiation.
    pub async fn done(self) -> Result<InstantiateExec<C, Signer>> {
        let url = self.extrinsic_opts.url();

        let salt = self
            .salt
            .clone()
            .map(|s| {
                anyhow::ensure!(s.len() <= 32, "salt has to be <= 32 bytes, got {}", s.len());
                let mut salt = [0u8; 32];
                salt[..s.len()].copy_from_slice(&s);
                Ok(salt)
            })
            .transpose()?;

        let rpc_cli = RpcClient::from_url(&url).await?;
        let client = OnlineClient::from_rpc_client(rpc_cli.clone()).await?;
        let rpc = LegacyRpcMethods::new(rpc_cli);

        let args = InstantiateArgs {
            value: self.value,
            gas_limit: self.gas_limit,
            proof_size: self.proof_size,
            storage_deposit_limit: self.extrinsic_opts.storage_deposit_limit(),
            code: self.code,
            data: self.data,
            salt,
        };

        Ok(InstantiateExec {
            args,
            opts: self.extrinsic_opts,
            rpc,
            client,
        })
    }
}

pub struct InstantiateArgs {
    value: u128,
    gas_limit: Option<u64>,
    proof_size: Option<u64>,
    storage_deposit_limit: Option<u128>,
    code: Code,
    data: Vec<u8>,
    salt: Option<[u8; 32]>,
}

impl InstantiateArgs {
    pub fn value(&self) -> u128 {
        self.value
    }

    pub fn gas_limit(&self) -> Option<u64> {
        self.gas_limit
    }

    pub fn proof_size(&self) -> Option<u64> {
        self.proof_size
    }

    pub fn storage_deposit_limit(&self) -> Option<u128> {
        self.storage_deposit_limit
    }

    pub fn code(&self) -> &Code {
        &self.code
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn salt(&self) -> Option<&[u8; 32]> {
        self.salt.as_ref()
    }
}

pub struct InstantiateExec<C: Config, Signer: Clone> {
    opts: ExtrinsicOpts<C, Signer>,
    args: InstantiateArgs,
    rpc: LegacyRpcMethods<C>,
    client: OnlineClient<C>,
}

impl<C: Config, Signer> InstantiateExec<C, Signer>
where
    C::AccountId: IntoVisitor,
    <C::ExtrinsicParams as ExtrinsicParams<C>>::Params:
        From<<DefaultExtrinsicParams<C> as ExtrinsicParams<C>>::Params>,
    Signer: tx::Signer<C> + Clone,
{
    /// Simulates a contract instantiation via `ReviveApi_instantiate` RPC state call.
    pub async fn instantiate_dry_run(&self) -> Result<ContractInstantiateResult<u128>> {
        let call_request = InstantiateRequest::<C> {
            origin: self.opts.signer().account_id(),
            value: self.args.value,
            gas_limit: None,
            storage_deposit_limit: self.args.storage_deposit_limit,
            code: self.args.code.clone(),
            data: self.args.data.clone(),
            salt: self.args.salt,
        };
        state_call(&self.rpc, "ReviveApi_instantiate", &call_request).await
    }

    async fn instantiate_with_code(
        &self,
        code: Vec<u8>,
        gas_limit: Weight,
        storage_deposit_limit: u128,
    ) -> Result<InstantiateExecResult<C>, ErrorVariant> {
        let call = InstantiateWithCode::new(
            self.args.value,
            gas_limit,
            storage_deposit_limit,
            code,
            self.args.data.clone(),
            self.args.salt,
        )
        .build();

        let events = submit_extrinsic(&self.client, &self.rpc, &call, self.opts.signer()).await?;

        let instantiated = events
            .find_last::<ContractInstantiated>()?
            .ok_or_else(|| anyhow!("Failed to find Instantiated event"))?;

        Ok(InstantiateExecResult {
            events,
            code_hash: None,
            contract_address: instantiated.contract,
        })
    }

    async fn instantiate_with_code_hash(
        &self,
        code_hash: H256,
        gas_limit: Weight,
        storage_deposit_limit: u128,
    ) -> Result<InstantiateExecResult<C>, ErrorVariant> {
        let call = Instantiate::new(
            self.args.value,
            gas_limit,
            storage_deposit_limit,
            code_hash,
            self.args.data.clone(),
            self.args.salt,
        )
        .build();

        let events = submit_extrinsic(&self.client, &self.rpc, &call, self.opts.signer()).await?;

        let instantiated = events
            .find_last::<ContractInstantiated>()?
            .ok_or_else(|| anyhow!("Failed to find Instantiated event"))?;

        Ok(InstantiateExecResult {
            events,
            code_hash: None,
            contract_address: instantiated.contract,
        })
    }

    /// Initiates the deployment of a smart contract on the blockchain.
    pub async fn instantiate(
        &self,
        gas_limit: Option<Weight>,
        storage_deposit_limit: Option<u128>,
    ) -> Result<InstantiateExecResult<C>, ErrorVariant> {
        let (use_gas_limit, use_storage_deposit_limit) = match (gas_limit, storage_deposit_limit) {
            (Some(gas), Some(deposit)) => (gas, deposit),
            (gas, deposit) => {
                let (estimated_gas, estimated_deposit) = self.estimate_limits().await?;
                (
                    gas.unwrap_or(estimated_gas),
                    deposit.unwrap_or(estimated_deposit),
                )
            }
        };

        match self.args.code.clone() {
            Code::Upload(code) => {
                self.instantiate_with_code(code, use_gas_limit, use_storage_deposit_limit)
                    .await
            }
            Code::Existing(code_hash) => {
                self.instantiate_with_code_hash(code_hash, use_gas_limit, use_storage_deposit_limit)
                    .await
            }
        }
    }

    /// Estimates the gas required for the contract instantiation.
    pub async fn estimate_limits(&self) -> Result<(Weight, u128)> {
        let instantiate_result = self.instantiate_dry_run().await?;
        match instantiate_result.result {
            Ok(_) => {
                let ref_time = self
                    .args
                    .gas_limit
                    .unwrap_or_else(|| instantiate_result.weight_required.ref_time());
                let proof_size = self
                    .args
                    .proof_size
                    .unwrap_or_else(|| instantiate_result.weight_required.proof_size());
                let deposit_limit = self.args.storage_deposit_limit.unwrap_or(
                    match instantiate_result.storage_deposit {
                        StorageDeposit::Refund(_) => 0,
                        StorageDeposit::Charge(value) => value,
                    },
                );
                Ok((Weight::from_parts(ref_time, proof_size), deposit_limit))
            }
            Err(ref err) => {
                let object = ErrorVariant::from_dispatch_error(err, &self.client.metadata())?;
                tracing::info!("Pre-submission dry-run failed. Error: {}", object);
                Err(anyhow!("Pre-submission dry-run failed. Error: {object}"))
            }
        }
    }

    pub fn opts(&self) -> &ExtrinsicOpts<C, Signer> {
        &self.opts
    }

    pub fn args(&self) -> &InstantiateArgs {
        &self.args
    }

    pub fn client(&self) -> &OnlineClient<C> {
        &self.client
    }

    pub fn rpc(&self) -> &LegacyRpcMethods<C> {
        &self.rpc
    }
}

/// A struct representing the result of an instantiate command execution.
pub struct InstantiateExecResult<C: Config> {
    pub events: ExtrinsicEvents<C>,
    pub code_hash: Option<H256>,
    pub contract_address: H160,
}

/// Result of a dry-run instantiation.
#[derive(serde::Serialize)]
pub struct InstantiateDryRunResult {
    /// Contract address
    pub contract: H160,
    /// Was the operation reverted
    pub reverted: bool,
    pub gas_consumed: Weight,
    pub gas_required: Weight,
    /// Storage deposit after the operation
    pub storage_deposit: StorageDeposit<u128>,
    /// The raw return data
    pub data: Vec<u8>,
}

impl InstantiateDryRunResult {
    /// Returns a result in json format
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

/// A struct that encodes RPC parameters required to instantiate a new smart contract.
#[derive(Encode)]
struct InstantiateRequest<C: Config> {
    origin: C::AccountId,
    value: u128,
    gas_limit: Option<Weight>,
    storage_deposit_limit: Option<u128>,
    code: Code,
    data: Vec<u8>,
    salt: Option<[u8; 32]>,
}

/// Reference to an existing code hash or new contract binary.
#[derive(Clone, Encode)]
pub enum Code {
    /// A contract binary as raw bytes.
    Upload(Vec<u8>),
    /// The code hash of an on-chain contract binary blob.
    Existing(H256),
}

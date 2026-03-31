use pallet_revive_uapi::ReturnFlags;
use scale::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_runtime::{DispatchError, RuntimeDebug};
use sp_weights::Weight;
use subxt::utils::{H160, H256};

/// Result type of a `bare_call` or `bare_instantiate` call as well as
/// `ContractsApi::call` and `ContractsApi::instantiate`.
///
/// Copied from `pallet_revive`.
#[derive(Clone, Eq, PartialEq, Encode, Decode, RuntimeDebug, TypeInfo)]
pub struct ContractResult<R, Balance> {
    /// How much weight was consumed during execution.
    pub weight_consumed: Weight,
    /// How much weight is required as weight limit in order to execute this call.
    pub weight_required: Weight,
    /// How much balance was paid by the origin into the contract's deposit account
    /// in order to pay for storage.
    pub storage_deposit: StorageDeposit<Balance>,
    /// The maximal storage deposit amount that occurred at any point during execution.
    pub max_storage_deposit: StorageDeposit<Balance>,
    /// The amount of Ethereum gas that was consumed during execution.
    pub gas_consumed: Balance,
    /// The execution result of the code.
    pub result: Result<R, DispatchError>,
}

/// Result type of a `bare_call` call, as well as `ContractsApi::call`.
pub type ContractExecResult<Balance> = ContractResult<ExecReturnValue, Balance>;

/// Result type of a `bare_instantiate` call, as well as `ContractsApi::instantiate`.
pub type ContractInstantiateResult<Balance> = ContractResult<InstantiateReturnValue, Balance>;

/// Result type of a `bare_code_upload` call.
pub type CodeUploadResult<Balance> = Result<CodeUploadReturnValue<Balance>, DispatchError>;

/// Result type of a `get_storage` call.
pub type GetStorageResult = Result<Option<Vec<u8>>, ContractAccessError>;

/// The possible errors that can happen querying the storage of a contract.
#[derive(Copy, Clone, Eq, PartialEq, Encode, Decode, MaxEncodedLen, RuntimeDebug, TypeInfo)]
pub enum ContractAccessError {
    /// The given address doesn't point to a contract.
    DoesntExist,
    /// Storage key cannot be decoded from the provided input data.
    KeyDecodingFailed,
    /// Storage is migrating. Try again later.
    MigrationInProgress,
}

/// Output of a contract call or instantiation which ran to completion.
#[derive(Clone, PartialEq, Eq, Encode, Decode, RuntimeDebug, TypeInfo)]
pub struct ExecReturnValue {
    /// Flags passed along by `seal_return`. Empty when `seal_return` was never called.
    pub flags: ReturnFlags,
    /// Buffer passed along by `seal_return`. Empty when `seal_return` was never called.
    pub data: Vec<u8>,
}

impl ExecReturnValue {
    /// The contract did revert all storage changes.
    pub fn did_revert(&self) -> bool {
        self.flags.contains(ReturnFlags::REVERT)
    }
}

/// The result of a successful contract instantiation.
#[derive(Clone, PartialEq, Eq, Encode, Decode, RuntimeDebug, TypeInfo)]
pub struct InstantiateReturnValue {
    /// The output of the called constructor.
    pub result: ExecReturnValue,
    /// The address of the new contract.
    pub addr: H160,
}

/// The result of successfully uploading a contract.
#[derive(Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen, RuntimeDebug, TypeInfo)]
pub struct CodeUploadReturnValue<Balance> {
    /// The key under which the new code is stored.
    pub code_hash: H256,
    /// The deposit that was reserved at the caller. Is zero when the code already existed.
    pub deposit: Balance,
}

/// Reference to an existing code hash or a new contract binary.
#[derive(Clone, Eq, PartialEq, Encode, Decode, RuntimeDebug, TypeInfo)]
pub enum Code<Hash> {
    /// Bytecode of a contract.
    Upload(Vec<u8>),
    /// The code hash of an on-chain contract binary.
    Existing(Hash),
}

/// The amount of balance that was either charged or refunded in order to pay for storage.
#[derive(
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Encode,
    Decode,
    MaxEncodedLen,
    RuntimeDebug,
    TypeInfo,
    serde::Serialize,
)]
pub enum StorageDeposit<Balance> {
    /// The transaction reduced storage consumption.
    Refund(Balance),
    /// The transaction increased storage consumption.
    Charge(Balance),
}

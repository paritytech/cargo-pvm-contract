use super::get_best_block;
use anyhow::{Result, anyhow};
use std::fmt::{Debug, Display, Formatter};

use scale::Decode;
use subxt::{
    Config, OnlineClient,
    backend::legacy::LegacyRpcMethods,
    config::HashFor,
    ext::{
        scale_decode::{DecodeAsType, IntoVisitor},
        scale_value::Value,
    },
    storage::dynamic,
    utils::{H160, H256},
};

/// Return the account data for an account ID.
pub async fn get_account_data<C: Config>(
    account: &C::AccountId,
    rpc: &LegacyRpcMethods<C>,
    client: &OnlineClient<C>,
) -> Result<AccountData<u128>>
where
    C::AccountId: AsRef<[u8]>,
{
    let storage_query =
        subxt::dynamic::storage("System", "Account", vec![Value::from_bytes(account)]);
    let best_block = get_best_block(rpc).await?;

    let account = client
        .storage()
        .at(best_block)
        .fetch(&storage_query)
        .await?
        .ok_or_else(|| anyhow!("Failed to fetch account data"))?;

    let data = account.as_type::<AccountInfo<u128>>()?.data;
    Ok(data)
}

/// Returns the `AccountId32` for an `H160`.
///
/// If a mapping for `addr` is not found on the node, a fallback account will be returned.
pub async fn resolve_h160<C: Config>(
    addr: &H160,
    rpc: &LegacyRpcMethods<C>,
    client: &OnlineClient<C>,
) -> Result<C::AccountId>
where
    C::AccountId: AsRef<[u8]> + Display + IntoVisitor + Decode,
    HashFor<C>: IntoVisitor,
{
    let best_block = get_best_block(rpc).await?;
    let contract_info_address = dynamic("Revive", "OriginalAccount", vec![Value::from_bytes(addr)]);
    let raw_value = client
        .storage()
        .at(best_block)
        .fetch(&contract_info_address)
        .await?;
    match raw_value {
        None => {
            fn to_fallback_account_id(address: &H160) -> [u8; 32] {
                let mut account_id = [0xEE; 32];
                account_id[..20].copy_from_slice(address.as_bytes());
                account_id
            }
            let fallback = to_fallback_account_id(addr);
            tracing::debug!(
                "No address suffix found for H160 {:?}, using fallback {:?}",
                addr,
                fallback
            );
            let account_id = <C as Config>::AccountId::decode(&mut &fallback[..])
                .map_err(|err| anyhow!("AccountId from fallback deserialization error: {err}"))?;
            Ok(account_id)
        }
        Some(raw_value) => {
            let raw_account_id = raw_value.as_type::<[u8; 32]>()?;
            let account: C::AccountId = Decode::decode(&mut &raw_account_id[..])
                .map_err(|err| anyhow!("AccountId from `[u8; 32]` deserialization error: {err}"))?;
            Ok(account)
        }
    }
}

/// Fetch the code info from the storage using the provided client.
pub async fn fetch_code_info<C: Config>(
    code_hash: &H256,
    rpc: &LegacyRpcMethods<C>,
    client: &OnlineClient<C>,
) -> Result<CodeInfo<C::AccountId, u128>>
where
    HashFor<C>: IntoVisitor,
    C::AccountId: AsRef<[u8]> + Display + IntoVisitor + Decode + Debug,
{
    let best_block = get_best_block(rpc).await?;

    let code_info_address = dynamic("Revive", "CodeInfoOf", vec![Value::from_bytes(code_hash)]);
    let code_info_value = client
        .storage()
        .at(best_block)
        .fetch(&code_info_address)
        .await?
        .ok_or_else(|| anyhow!("No code info was found for hash {code_hash:?}"))?;
    let code_info = code_info_value.as_type::<CodeInfo<C::AccountId, u128>>()?;
    Ok(code_info)
}

/// Fetch the contract info from the storage using the provided client.
pub async fn fetch_contract_info<C: Config>(
    contract: &H160,
    rpc: &LegacyRpcMethods<C>,
    client: &OnlineClient<C>,
) -> Result<ContractInfo<u128>>
where
    HashFor<C>: IntoVisitor,
    C::AccountId: AsRef<[u8]> + Display + IntoVisitor + Decode + Debug,
{
    let best_block = get_best_block(rpc).await?;

    let account_info_address =
        dynamic("Revive", "AccountInfoOf", vec![Value::from_bytes(contract)]);
    let account_info_value = client
        .storage()
        .at(best_block)
        .fetch(&account_info_address)
        .await?
        .ok_or_else(|| anyhow!("No contract was found for address {contract:?}"))?;
    let account_info = account_info_value.as_type::<PrAccountInfo<u128>>()?;

    let contract_info = match account_info.account_type {
        PrAccountType::Contract(ci) => ci,
        PrAccountType::Eoa => anyhow::bail!("Address is an EOA, not a contract"),
    };
    Ok(ContractInfo {
        trie_id: contract_info.trie_id.0.into(),
        code_hash: contract_info.code_hash,
        storage_bytes: contract_info.storage_bytes,
        storage_items: contract_info.storage_items,
        storage_byte_deposit: contract_info.storage_byte_deposit,
        storage_item_deposit: contract_info.storage_item_deposit,
        storage_base_deposit: contract_info.storage_base_deposit,
        immutable_data_len: contract_info.immutable_data_len,
    })
}

/// Copied from `pallet-revive`.
#[derive(DecodeAsType)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
pub struct PrAccountInfo<Balance: Debug + DecodeAsType> {
    pub account_type: PrAccountType<Balance>,
    #[allow(dead_code)]
    pub dust: u32,
}

/// Copied from `pallet-revive`.
#[derive(DecodeAsType)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
pub enum PrAccountType<Balance: Debug + DecodeAsType> {
    Contract(PrContractInfo<Balance>),
    Eoa,
}

/// Copied from `pallet-revive`.
#[derive(DecodeAsType, Debug, PartialEq, serde::Serialize)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
pub struct PrContractInfo<Balance: Debug + IntoVisitor> {
    trie_id: TrieId,
    code_hash: H256,
    storage_bytes: u32,
    storage_items: u32,
    storage_byte_deposit: Balance,
    storage_item_deposit: Balance,
    storage_base_deposit: Balance,
    immutable_data_len: u32,
}

/// Contract info with public fields.
#[derive(DecodeAsType, Debug, PartialEq, serde::Serialize)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
pub struct ContractInfo<Balance: Debug + IntoVisitor> {
    trie_id: TrieId,
    code_hash: H256,
    storage_bytes: u32,
    storage_items: u32,
    storage_byte_deposit: Balance,
    storage_item_deposit: Balance,
    storage_base_deposit: Balance,
    immutable_data_len: u32,
}

impl<Balance> ContractInfo<Balance>
where
    Balance: serde::Serialize + Copy + IntoVisitor + Debug,
{
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn trie_id(&self) -> &TrieId {
        &self.trie_id
    }

    pub fn code_hash(&self) -> &H256 {
        &self.code_hash
    }

    pub fn storage_bytes(&self) -> u32 {
        self.storage_bytes
    }

    pub fn storage_items(&self) -> u32 {
        self.storage_items
    }
}

/// Contract code related data.
#[derive(DecodeAsType, Eq, PartialEq, Clone, Debug, serde::Serialize)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
pub struct CodeInfo<
    AccountId: Debug + DecodeAsType + IntoVisitor,
    Balance: Debug + DecodeAsType + IntoVisitor,
> {
    pub owner: AccountId,
    #[codec(compact)]
    pub deposit: Balance,
    #[codec(compact)]
    pub refcount: u64,
    pub code_len: u32,
    pub code_type: BytecodeType,
    pub behaviour_version: u32,
}

/// Copied from `pallet-revive`.
#[derive(PartialEq, Eq, Debug, Copy, Clone, DecodeAsType, serde::Serialize)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
pub enum BytecodeType {
    Pvm,
    Evm,
}

/// A contract's child trie id.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, DecodeAsType)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
pub struct TrieId(Vec<u8>);

impl TrieId {
    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(&self.0))
    }
}

impl From<Vec<u8>> for TrieId {
    fn from(raw: Vec<u8>) -> Self {
        Self(raw)
    }
}

impl AsRef<[u8]> for TrieId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Display for TrieId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Fetch the contract binary from storage using the provided client and code hash.
pub async fn fetch_contract_binary<C: Config>(
    client: &OnlineClient<C>,
    rpc: &LegacyRpcMethods<C>,
    hash: &H256,
) -> Result<Vec<u8>> {
    let best_block = get_best_block(rpc).await?;

    let pristine_code_address = dynamic("Revive", "PristineCode", vec![Value::from_bytes(hash)]);
    let pristine_code = client
        .storage()
        .at(best_block)
        .fetch(&pristine_code_address)
        .await?
        .ok_or_else(|| anyhow!("No contract binary was found for code hash {hash}"))?;
    pristine_code
        .as_type::<Vec<u8>>()
        .map_err(|e| anyhow!("Contract binary could not be parsed: {e}"))
}

/// Parse a contract address from a storage key.
fn parse_contract_address(
    storage_contract_account_key: &[u8],
    storage_contract_root_key_len: usize,
) -> Result<H160> {
    let mut account = storage_contract_account_key
        .get(storage_contract_root_key_len..)
        .ok_or(anyhow!("Unexpected storage key size"))?;
    Decode::decode(&mut account).map_err(|err| anyhow!("H160 deserialization error: {err}"))
}

/// Fetch all contract addresses from storage, excluding EOAs.
pub async fn fetch_all_contracts<C: Config>(
    client: &OnlineClient<C>,
    rpc: &LegacyRpcMethods<C>,
) -> Result<Vec<H160>> {
    let best_block = get_best_block(rpc).await?;
    let storage = client.storage().at(best_block);
    let root_key = subxt::dynamic::storage("Revive", "AccountInfoOf", ()).to_root_bytes();
    let mut keys = storage.fetch_raw_keys(root_key.clone()).await?;

    let mut contract_accounts = Vec::new();
    while let Some(result) = keys.next().await {
        let key = result?;
        let addr = parse_contract_address(&key, root_key.len())?;
        let account_info_address =
            dynamic("Revive", "AccountInfoOf", vec![Value::from_bytes(addr)]);
        if let Some(value) = storage.fetch(&account_info_address).await? {
            let info = value.as_type::<PrAccountInfo<u128>>()?;
            if matches!(info.account_type, PrAccountType::Contract(_)) {
                contract_accounts.push(addr);
            }
        }
    }

    Ok(contract_accounts)
}

/// A struct used in the storage reads to access account info.
#[derive(DecodeAsType, Debug)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
struct AccountInfo<Balance> {
    data: AccountData<Balance>,
}

/// A struct used in the storage reads to access account data.
#[derive(Clone, Debug, DecodeAsType)]
#[decode_as_type(crate_path = "subxt::ext::scale_decode")]
pub struct AccountData<Balance> {
    pub free: Balance,
    pub reserved: Balance,
    pub frozen: Balance,
}

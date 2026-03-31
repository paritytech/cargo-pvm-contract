use crate::ContractBinary;
use subxt::{
    ext::scale_encode::EncodeAsType,
    utils::{H160, H256},
};

/// Copied from `sp_weight` to additionally implement `scale_encode::EncodeAsType`.
#[derive(Debug, EncodeAsType)]
#[encode_as_type(crate_path = "subxt::ext::scale_encode")]
pub(crate) struct Weight {
    #[codec(compact)]
    ref_time: u64,
    #[codec(compact)]
    proof_size: u64,
}

impl From<sp_weights::Weight> for Weight {
    fn from(weight: sp_weights::Weight) -> Self {
        Self {
            ref_time: weight.ref_time(),
            proof_size: weight.proof_size(),
        }
    }
}

impl core::fmt::Display for Weight {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Weight(ref_time: {}, proof_size: {})",
            self.ref_time, self.proof_size
        )
    }
}

/// A raw call to `pallet-revive`'s `remove_code`.
#[derive(EncodeAsType)]
#[encode_as_type(crate_path = "subxt::ext::scale_encode")]
pub(crate) struct RemoveCode<Hash> {
    code_hash: Hash,
}

impl<Hash> RemoveCode<Hash> {
    pub fn new(code_hash: Hash) -> Self {
        Self { code_hash }
    }

    pub fn build(self) -> subxt::tx::DefaultPayload<Self> {
        subxt::tx::DefaultPayload::new("Revive", "remove_code", self)
    }
}

/// A raw call to `pallet-revive`'s `upload_code`.
#[derive(Debug, EncodeAsType)]
#[encode_as_type(crate_path = "subxt::ext::scale_encode")]
pub struct UploadCode {
    code: Vec<u8>,
    storage_deposit_limit: u128,
}

impl UploadCode {
    pub fn new(code: ContractBinary, storage_deposit_limit: u128) -> Self {
        Self {
            code: code.0,
            storage_deposit_limit,
        }
    }

    pub fn build(self) -> subxt::tx::DefaultPayload<Self> {
        subxt::tx::DefaultPayload::new("Revive", "upload_code", self)
    }
}

/// A raw call to `pallet-revive`'s `instantiate_with_code`.
#[derive(Debug, EncodeAsType)]
#[encode_as_type(crate_path = "subxt::ext::scale_encode")]
pub struct InstantiateWithCode {
    #[codec(compact)]
    value: u128,
    weight_limit: Weight,
    #[codec(compact)]
    storage_deposit_limit: u128,
    code: Vec<u8>,
    data: Vec<u8>,
    salt: Option<[u8; 32]>,
}

impl InstantiateWithCode {
    pub fn new(
        value: u128,
        weight_limit: sp_weights::Weight,
        storage_deposit_limit: u128,
        code: Vec<u8>,
        data: Vec<u8>,
        salt: Option<[u8; 32]>,
    ) -> Self {
        Self {
            value,
            weight_limit: weight_limit.into(),
            storage_deposit_limit,
            code,
            data,
            salt,
        }
    }

    pub fn build(self) -> subxt::tx::DefaultPayload<Self> {
        subxt::tx::DefaultPayload::new("Revive", "instantiate_with_code", self)
    }
}

/// A raw call to `pallet-revive`'s `instantiate` (from existing code hash).
#[derive(Debug, EncodeAsType)]
#[encode_as_type(crate_path = "subxt::ext::scale_encode")]
pub struct Instantiate {
    #[codec(compact)]
    value: u128,
    weight_limit: Weight,
    #[codec(compact)]
    storage_deposit_limit: u128,
    code_hash: H256,
    data: Vec<u8>,
    salt: Option<[u8; 32]>,
}

impl Instantiate {
    pub fn new(
        value: u128,
        weight_limit: sp_weights::Weight,
        storage_deposit_limit: u128,
        code_hash: H256,
        data: Vec<u8>,
        salt: Option<[u8; 32]>,
    ) -> Self {
        Self {
            value,
            weight_limit: weight_limit.into(),
            storage_deposit_limit,
            code_hash,
            data,
            salt,
        }
    }

    pub fn build(self) -> subxt::tx::DefaultPayload<Self> {
        subxt::tx::DefaultPayload::new("Revive", "instantiate", self)
    }
}

/// A raw call to `pallet-revive`'s `call`.
#[derive(EncodeAsType)]
#[encode_as_type(crate_path = "subxt::ext::scale_encode")]
pub struct Call {
    dest: H160,
    #[codec(compact)]
    value: u128,
    weight_limit: Weight,
    storage_deposit_limit: u128,
    data: Vec<u8>,
}

impl Call {
    pub fn new(
        dest: H160,
        value: u128,
        weight_limit: sp_weights::Weight,
        storage_deposit_limit: u128,
        data: Vec<u8>,
    ) -> Self {
        Self {
            dest,
            value,
            weight_limit: weight_limit.into(),
            storage_deposit_limit,
            data,
        }
    }

    pub fn build(self) -> subxt::tx::DefaultPayload<Self> {
        subxt::tx::DefaultPayload::new("Revive", "call", self)
    }
}

/// A raw call to `pallet-revive`'s `map_account`.
#[derive(Debug, EncodeAsType)]
#[encode_as_type(crate_path = "subxt::ext::scale_encode")]
pub(crate) struct MapAccount {}

impl MapAccount {
    pub fn new() -> Self {
        Self {}
    }

    pub fn build(self) -> subxt::tx::DefaultPayload<Self> {
        subxt::tx::DefaultPayload::new("Revive", "map_account", self)
    }
}

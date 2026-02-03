#![no_std]

#[cfg(target_arch = "riscv64")]
pub mod storage;

pub use pvm_contract_macros::{constructor, contract, fallback, method, storage};

pub use primitive_types::H160 as Address;
pub use alloy_primitives::{FixedBytes, I256, U256};

pub use pallet_revive_uapi::{HostFn, HostFnImpl as api, ReturnFlags, StorageFlags};

pub use parity_scale_codec::{Encode, Decode};

pub use polkavm_derive;

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#[inline]
pub fn caller() -> Address {
    let mut addr = [0u8; 20];
    api::caller(&mut addr);
    Address::from(addr)
}

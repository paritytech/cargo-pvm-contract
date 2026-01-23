#![no_std]

pub use pvm_contract_macros::{constructor, contract, fallback, method};

pub use ruint::{
    aliases::{U128, U256, U32, U64},
    Uint,
};

pub use pallet_revive_uapi::{HostFn, HostFnImpl as api, ReturnFlags, StorageFlags};

pub use polkavm_derive;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct Address([u8; 20]);

impl Address {
    pub const ZERO: Self = Self([0u8; 20]);

    #[inline]
    pub const fn new(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    #[inline]
    pub const fn as_slice(&self) -> &[u8] {
        &self.0
    }

    #[inline]
    pub const fn into_array(self) -> [u8; 20] {
        self.0
    }
}

impl From<[u8; 20]> for Address {
    #[inline]
    fn from(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }
}

impl From<Address> for [u8; 20] {
    #[inline]
    fn from(addr: Address) -> Self {
        addr.0
    }
}

impl AsRef<[u8]> for Address {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#[inline]
pub fn caller() -> Address {
    let mut addr = [0u8; 20];
    api::caller(&mut addr);
    Address::from(addr)
}

#![no_std]

extern crate alloc;

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub mod storage;

pub mod abi;

pub mod call;

pub use pvm_contract_macros::{abi_import, constructor, contract, fallback, method, storage, SolAbi};

pub use ethereum_types::Address;
pub use alloy_primitives::{FixedBytes, I256, U256};

pub use pallet_revive_uapi::{CallFlags, HostFn, HostFnImpl as api, ReturnFlags, StorageFlags};

pub use parity_scale_codec::{Encode, Decode};

pub use abi::{Bytes, SolAbi, compute_selector};

pub use polkavm_derive;
pub use const_format;

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("unimp");
        core::hint::unreachable_unchecked()
    }
}

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#[inline]
pub fn caller() -> Address {
    let mut addr = [0u8; 20];
    api::caller(&mut addr);
    Address::from(addr)
}

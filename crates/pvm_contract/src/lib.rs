#![no_std]

extern crate alloc;

// The storage layer can also be compiled for the host so its logic (most
// importantly the `OrderedIndex` B-tree) is testable with plain `cargo test`.
// On the host it is backed by a thread-local map instead of host functions;
// see the `backend` module in `storage.rs`. Target builds are unaffected.
#[cfg(all(
    any(test, feature = "host-test"),
    not(any(target_arch = "riscv32", target_arch = "riscv64"))
))]
extern crate std;

#[cfg(any(
    target_arch = "riscv32",
    target_arch = "riscv64",
    test,
    feature = "host-test"
))]
pub mod storage;

pub mod abi;

pub mod call;

// CDM cross-contract compat surface (see `call.rs`): re-exported at the crate
// root because `pvm_cdm::reference!` references these as `pvm_contract_sdk::*`.
pub use call::{HostApi, PolkaVmHost, Pure};

pub use pvm_contract_macros::{
    SolAbi, abi_import, constructor, contract, fallback, method, storage,
};

pub use alloy_primitives::{FixedBytes, I256, U256};
pub use ethereum_types::Address;

pub use pallet_revive_uapi::{CallFlags, HostFn, HostFnImpl as api, ReturnFlags, StorageFlags};

pub use parity_scale_codec::{Decode, Encode};

pub use abi::{Bytes, SolAbi, compute_selector};

pub use const_format;
pub use polkavm_derive;

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

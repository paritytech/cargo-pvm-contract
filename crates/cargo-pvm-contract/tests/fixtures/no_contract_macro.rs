// Test fixture for `rebuild_without_contract_macro_clears_stale_abi_json`.
//
// A minimal polkavm-only contract source that compiles against the standard
// macro-scaffolded Cargo.toml (`pvm-contract-sdk` + `polkavm-derive`) but
// does not invoke the contract attribute macro, so the builder's
// `has_contract_macro` check returns false and `generate_abi_file` hits
// the `Ok(None)` arm without writing.

#![cfg_attr(not(test), no_main, no_std)]

use pvm_contract_sdk::U256;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("unimp");
        core::hint::unreachable_unchecked()
    }
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn deploy() {}

#[cfg(not(test))]
#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn call() {
    let _u = U256::ZERO;
}

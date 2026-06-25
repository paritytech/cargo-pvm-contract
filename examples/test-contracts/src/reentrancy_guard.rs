#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

//! E2E fixture for the `#[non_reentrant]` modifier.

#[pvm_contract_sdk::contract(allocator = "pico")]
mod reentrancy_guard {
    use pvm_contract_sdk::*;

    #[derive(SolError, Debug)]
    pub enum Error {
        Panic(Panic),
        Revert(RevertString),
    }

    pub struct ReentrancyGuard;

    impl ReentrancyGuard {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::non_reentrant]
        pub fn protected(&mut self) -> Result<(), Error> {
            Ok(())
        }

        /// Re-enters `protected()` with `ALLOW_REENTRY` so the SDK guard, not
        /// pallet-revive's default reject, must reject it, then forwards the
        /// revert for the test to assert. Raw `call_evm` because `abi_import!`
        /// doesn't expose `ALLOW_REENTRY`.
        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::non_reentrant]
        pub fn attempt_reentry(&mut self) -> Result<(), Error> {
            let mut own = [0u8; 20];
            self.host().address(&mut own);

            let selector = const_selector("protected()");
            let mut out_storage = [0u8; 32];
            let mut out: &mut [u8] = &mut out_storage;
            let res = self.host().call_evm(
                CallFlags::ALLOW_REENTRY,
                &own,
                u64::MAX,
                &[0u8; 32],
                &selector,
                Some(&mut out),
            );

            if res.is_err() {
                // Forward the callee's error bytes so the test sees the real error.
                let size = self.host().return_data_size() as usize;
                let mut buf = alloc::vec![0u8; size];
                self.host().return_data_copy(&mut buf.as_mut_slice(), 0);
                self.host().return_value(ReturnFlags::REVERT, &buf);
            }

            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
}

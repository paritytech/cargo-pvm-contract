#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

//! E2E fixture: a borrower contract that calls back into its lender, the
//! legitimate reentrancy pattern (flash loans, ERC-777 hooks).

#[pvm_contract_sdk::contract(allocator = "pico")]
mod reentrancy_callback {
    use pvm_contract_sdk::*;

    #[derive(SolError, Debug)]
    pub enum Error {
        Panic(Panic),
        Revert(RevertString),
    }

     #[derive(SolError, Debug)]
    pub struct CallbackNotExecuted;

    pub struct ReentrancyCallback;

    impl ReentrancyCallback {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            Ok(())
        }

        /// Calls `lender.onCallback()` and checks the returned `bool`. Whether
        /// the call is permitted at all depends on the flag the lender set on
        /// its own outbound call.
        #[pvm_contract_sdk::method]
        pub fn call_back(&mut self, lender: Address) -> Result<(), Error> {
            let selector = const_selector("onCallback()");
            let mut out_storage = [0u8; 32];
            let mut out: &mut [u8] = &mut out_storage;
            let res = self.host().call_evm(
                CallFlags::empty(),
                &lender.0,
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
                self.host().revert(&buf);
            }
            // `bool` is ABI-encoded right-aligned in a 32-byte word, so `true`
            // is a 1 in the last byte. A mismatch means the callback did not
            // reach `onCallback()`'s body, which a bare tx success would hide.
            if out_storage[31] != 1 {
                self.host()
                    .revert(&<CallbackNotExecuted as SolError>::SELECTOR);
            }
            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
}
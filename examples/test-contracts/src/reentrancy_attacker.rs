#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

//! E2E fixture: a separate contract that re-enters a `#[non_reentrant]` target,
//! for the cross-contract reentrancy test.

#[pvm_contract_sdk::contract(allocator = "pico")]
mod reentrancy_attacker {
    use pvm_contract_sdk::*;

    #[derive(SolError, Debug)]
    pub enum Error {
        Panic(Panic),
        Revert(RevertString),
    }

    pub struct ReentrancyAttacker;

    impl ReentrancyAttacker {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            Ok(())
        }

        /// Call `target.protected()` with `ALLOW_REENTRY` set, so the target's
        /// SDK guard (not pallet-revive's default reject) must catch the
        /// re-entry. Forwards the target's revert for the test to assert.
        #[pvm_contract_sdk::method]
        pub fn reenter(&mut self, target: Address) -> Result<(), Error> {
            let selector = const_selector("protected()");
            let res = self.host().call_evm(
                CallFlags::ALLOW_REENTRY,
                &target.0,
                u64::MAX,
                &[0u8; 32],
                &selector,
                None,
            );
            if res.is_err() {
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

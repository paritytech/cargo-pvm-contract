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

    pub struct ReentrancyGuard {
        /// Bumped by each guarded call that runs its body to completion, so a
        /// test can confirm the calls actually executed (not just that the tx
        /// didn't revert).
        count: Lazy<U256>,
    }

    impl ReentrancyGuard {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::non_reentrant]
        pub fn protected(&mut self) -> Result<(), Error> {
            self.count.set(&(self.count.get() + U256::from(1u64)));
            Ok(())
        }

        /// Read the number of completed guarded calls.
        #[pvm_contract_sdk::method]
        pub fn count(&self) -> U256 {
            self.count.get()
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

        /// A guarded method that exits via a raw diverging `return_value`
        /// (success), so the codegen's post-body unlock is skipped. The lock
        /// must still be released by the `return_value` choke point.
        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::non_reentrant]
        pub fn protected_diverging(&mut self) -> Result<(), Error> {
            self.count.set(&(self.count.get() + U256::from(1u64)));
            self.host().return_value(ReturnFlags::empty(), &[]);
            #[allow(unreachable_code)]
            Ok(())
        }

        /// Regression test for the divergence hole: call the diverging guarded
        /// method, then call a guarded method again in the *same* transaction.
        /// The second call must succeed; if the first call's divergent exit left
        /// the lock set, it would revert with `ReentrancyGuardReentrantCall`.
        /// Both are `ALLOW_REENTRY` self-calls (separate frames, shared transient
        /// storage). Not itself guarded, so it holds no lock.
        #[pvm_contract_sdk::method]
        pub fn sequential_guarded_calls(&mut self) -> Result<(), Error> {
            let mut own = [0u8; 20];
            self.host().address(&mut own);

            // 1) Diverging guarded call: sets the lock, exits via raw return_value.
            let sel1 = const_selector("protectedDiverging()");
            let _ = self.host().call_evm(
                CallFlags::ALLOW_REENTRY,
                &own,
                u64::MAX,
                &[0u8; 32],
                &sel1,
                None,
            );

            // 2) A second guarded call in the same tx must not see a stale lock.
            let sel2 = const_selector("protected()");
            let res = self.host().call_evm(
                CallFlags::ALLOW_REENTRY,
                &own,
                u64::MAX,
                &[0u8; 32],
                &sel2,
                None,
            );
            if res.is_err() {
                // Forward the callee's error so the test sees the real revert.
                let size = self.host().return_data_size() as usize;
                let mut buf = alloc::vec![0u8; size];
                self.host().return_data_copy(&mut buf.as_mut_slice(), 0);
                self.host().return_value(ReturnFlags::REVERT, &buf);
            }

            Ok(())
        }

        /// Guarded. Calls out to a separate `attacker` contract, which re-enters
        /// this contract's `protected()`. This outbound call sets `ALLOW_REENTRY`
        /// so *this* contract permits being re-entered (pallet-revive keys the
        /// reentry decision on the re-entered contract's own outbound flag, not
        /// on the attacker's callback), letting the SDK guard — not
        /// pallet-revive's default reject — block it. Forwards the revert.
        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::non_reentrant]
        pub fn protected_calls_out(&mut self, attacker: Address) -> Result<(), Error> {
            let mut own = [0u8; 20];
            self.host().address(&mut own);

            // attacker.reenter(own): selector ++ own address right-aligned in a word.
            let mut calldata = [0u8; 36];
            calldata[..4].copy_from_slice(&const_selector("reenter(address)"));
            calldata[16..36].copy_from_slice(&own);

            let res = self.host().call_evm(
                CallFlags::ALLOW_REENTRY,
                &attacker.0,
                u64::MAX,
                &[0u8; 32],
                &calldata,
                None,
            );
            if res.is_err() {
                // Forward the callee's error so the test sees the real revert.
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

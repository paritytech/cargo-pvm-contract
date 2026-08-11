#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

// The imported interface is CallerCheck's *own* `recordContext()`. The E2E test
// deploys two instances and has one call the other, which is the only way to
// observe `caller()` and `origin()` diverging — in a direct call from an EOA
// they are the same address.
pvm_contract_sdk::abi_import! {
#![abi_import(alloc = true)]
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface ContextRecorder {
    function recordContext() external;
}
}

#[pvm_contract_sdk::contract("CallerCheck.sol", allocator = "pico")]
mod caller_check {
    use pvm_contract_sdk::{Address, CallError, Panic, RevertString, SolError, StorageFlags, U256};

    use super::context_recorder::ContextRecorder;

    // Raw slots, one per recorded context value.
    const LAST_CALLER_KEY: [u8; 32] = [0u8; 32];
    const LAST_ORIGIN_KEY: [u8; 32] = [1u8; 32];
    const LAST_SELF_KEY: [u8; 32] = [2u8; 32];

    #[derive(SolError, Debug)]
    pub enum Error {
        CallError(CallError),
        Panic(Panic),
        Revert(RevertString),
    }

    pub struct CallerCheck;

    impl CallerCheck {
        /// Payable so the E2E test can deploy a funded instance; that is the
        /// only way to give [`Self::get_balance`] a value big enough to prove
        /// the 256-bit decode.
        #[pvm_contract_sdk::constructor]
        #[pvm_contract_sdk::payable]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_caller(&self) -> Address {
            self.env().caller()
        }

        /// `tx.origin` — the transaction signer, unchanged at every call depth.
        #[pvm_contract_sdk::method]
        pub fn get_origin(&self) -> Address {
            self.env().origin()
        }

        /// `address(this)` — this contract's own address.
        #[pvm_contract_sdk::method]
        pub fn get_self_address(&self) -> Address {
            self.env().address()
        }

        /// `block.number`. Narrowed from the host's little-endian 32 bytes by
        /// reading the low limb and ignoring the rest, so a byte-order mismatch
        /// is silent: it reads back as 0 rather than reverting. The E2E test
        /// catches that by comparing against the node's own view (and asserting
        /// the node's value is itself non-zero).
        #[pvm_contract_sdk::method]
        pub fn get_block_number(&self) -> u64 {
            self.env().block_number()
        }

        /// `block.timestamp`, in seconds.
        #[pvm_contract_sdk::method]
        pub fn get_timestamp(&self) -> u64 {
            self.env().timestamp()
        }

        /// `block.chainid`.
        #[pvm_contract_sdk::method]
        pub fn get_chain_id(&self) -> u64 {
            self.env().chain_id()
        }

        /// `address(this).balance`. Unlike the three `u64` reads above this one
        /// keeps all 256 bits, so a funded instance is the only fixture that
        /// exercises the little-endian decode past the low limb — wei balances
        /// clear `u64::MAX` at ~18.4 ETH-equivalent.
        #[pvm_contract_sdk::method]
        pub fn get_balance(&self) -> U256 {
            self.env().balance()
        }

        /// `account.balance`. On chain this is the same query as
        /// [`Self::get_balance`] when `account` is this contract, which the E2E
        /// test asserts.
        #[pvm_contract_sdk::method]
        pub fn get_balance_of(&self, account: Address) -> U256 {
            self.env().balance_of(account)
        }

        #[pvm_contract_sdk::method]
        pub fn record_caller(&mut self) {
            let caller = self.env().caller();
            self.write_addr(&LAST_CALLER_KEY, caller);
        }

        /// Persist all three context reads so a *caller* can inspect what this
        /// contract saw during a nested call. Reading `getCaller()` directly
        /// cannot show that: a fresh top-level `eth_call` has the EOA as sender
        /// again, so the intermediate frame's view has to be recorded while it
        /// is live.
        #[pvm_contract_sdk::method]
        pub fn record_context(&mut self) {
            let env = self.env();
            let (caller, origin, address) = (env.caller(), env.origin(), env.address());
            self.write_addr(&LAST_CALLER_KEY, caller);
            self.write_addr(&LAST_ORIGIN_KEY, origin);
            self.write_addr(&LAST_SELF_KEY, address);
        }

        /// Call `recordContext()` on another instance, making this contract the
        /// intermediate frame: the callee then sees `caller() == address(this)`
        /// while `origin()` is still the EOA that signed the transaction.
        #[pvm_contract_sdk::method]
        pub fn record_context_on(&mut self, target: Address) -> Result<(), Error> {
            let recorder = ContextRecorder::from_address(target);
            recorder.record_context().call(self)?;
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_last_caller(&self) -> Address {
            self.read_addr(&LAST_CALLER_KEY)
        }

        #[pvm_contract_sdk::method]
        pub fn get_last_origin(&self) -> Address {
            self.read_addr(&LAST_ORIGIN_KEY)
        }

        #[pvm_contract_sdk::method]
        pub fn get_last_self(&self) -> Address {
            self.read_addr(&LAST_SELF_KEY)
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        /// Store `addr` right-aligned in a 32-byte slot, as solc would.
        fn write_addr(&mut self, key: &[u8; 32], addr: Address) {
            let mut buf = [0u8; 32];
            buf[12..32].copy_from_slice(addr.as_ref());
            self.host().set_storage(StorageFlags::empty(), key, &buf);
        }

        /// Inverse of [`Self::write_addr`]; an unset slot reads back as zero.
        fn read_addr(&self, key: &[u8; 32]) -> Address {
            let mut buf = [0u8; 32];
            let mut out = &mut buf[..];
            match self.host().get_storage(StorageFlags::empty(), key, &mut out) {
                Ok(_) => {
                    let mut addr = [0u8; 20];
                    addr.copy_from_slice(&buf[12..32]);
                    addr.into()
                }
                Err(_) => Address::from([0u8; 20]),
            }
        }
    }
}

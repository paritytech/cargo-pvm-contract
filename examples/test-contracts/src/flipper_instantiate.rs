#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

pvm_contract_sdk::abi_import! {
#![abi_import(alloc = true)]
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Flipper {
    constructor();
    function flip() external;
    function get() external view returns (bool);
}
}

#[pvm_contract_sdk::contract("FlipperCallAlloy.sol", allocator = "pico")]
mod flipper_instantiate {
    use pvm_contract_sdk::*;
    use pvm_contract_sdk::{CallError, RefTimeAndProofSizeLimits};

    use super::*;
    use flipper::{self, Flipper};

    sol_revert_enum! {
        pub enum Error {
            CallError(CallError)
        }
    }

    pub struct FlipperInstantiate;

    impl FlipperInstantiate {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn call_flipper(&mut self, addr: Address) -> Result<(), Error> {
            let flipper = Flipper::from_address(addr);
            let get = flipper.get();
            let flip = flipper.flip();

            let res = get.call(self)?;
            assert_eq!(res, false);
            let _ = flip.call(self)?;
            let res = get.call(self)?;
            assert_eq!(res, true);
            let mut code_hash = [0; 32];
            let _ = self.host().code_hash(&addr.0, &mut code_hash);
            let f = flipper::new_flipper();
            let deposit_limit = ruint::aliases::U256::from(u128::MAX);
            let deposit_limit = deposit_limit.to_be_bytes();
            let (addr, _) = f.instantiate(
                self,
                &code_hash,
                0,
                RefTimeAndProofSizeLimits {
                    ref_time_limit: u64::MAX,
                    proof_size_limit: u64::MAX,
                    deposit_limit,
                },
                None,
            )?;
            let flipper = Flipper::from_address(addr);
            let get = flipper.get();
            let flip = flipper.flip();

            let res = get.call(self)?;
            assert_eq!(res, false);
            let _ = flip.call(self)?;
            let res = get.call(self)?;
            assert_eq!(res, true);
            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
}

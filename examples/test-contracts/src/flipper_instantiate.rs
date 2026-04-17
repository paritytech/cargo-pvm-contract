#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

pvm_contract_macros::abi_import!(alloc = true, {
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Flipper {
    constructor();
    function flip() external;
    function get() external view returns (bool);
}
});

#[pvm_contract_macros::contract("FlipperCallAlloy.sol", allocator = "pico")]
mod flipper_instantiate {

    use pvm_contract_core::call::{CallError, RefTimeAndProofSizeLimits};
    use pvm_contract_types::PolkaVmHost as api;
    use pvm_contract_types::*;

    use super::*;
    use flipper::{self, Flipper};

    sol_revert_enum! {
        pub enum Error {
            CallError(CallError)
        }
    }

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn call_flipper(addr: Address) -> Result<(), Error> {
        let flipper = Flipper::from_address(addr);
        let get = flipper.get();
        let flip = flipper.flip();

        let res = get.call()?;
        assert_eq!(res, false);
        let _ = flip.call()?;
        let res = get.call()?;
        assert_eq!(res, true);
        // test deployed
        let mut code_hash = [0; 32];
        let _ = api::code_hash(&addr.0, &mut code_hash);
        let f = flipper::new_flipper();
        let deposit_limit = ruint::aliases::U256::from(u128::MAX);
        let deposit_limit = deposit_limit.to_be_bytes();
        let (addr, _) = f.instantiate(
            &code_hash,
            0,
            RefTimeAndProofSizeLimits {
                ref_time_limit: u64::MAX,
                proof_size_limit: u64::MAX,
                deposit_limit: deposit_limit,
            },
            None,
        )?;
        let flipper = Flipper::from_address(addr);
        let get = flipper.get();
        let flip = flipper.flip();

        let res = get.call()?;
        assert_eq!(res, false);
        let _ = flip.call()?;
        let res = get.call()?;
        assert_eq!(res, true);
        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }
}

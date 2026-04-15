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

    use pallet_revive_uapi::HostFnImpl as api;
    use pvm_contract_core::call::{CallError, RefTimeAndProofSizeLimits};
    use pvm_contract_types::*;

    use super::*;

    #[derive(Debug, Clone, Copy)]
    pub enum Error {
        Unexpected,
    }

    impl From<CallError> for Error {
        fn from(_value: CallError) -> Self {
            Self::Unexpected
        }
    }

    impl AsRef<[u8]> for Error {
        fn as_ref(&self) -> &[u8] {
            match *self {
                Error::Unexpected => b"Unexpected",
            }
        }
    }

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn call_flipper(addr: Address) -> Result<(), CallError> {
        let flipper = Flipper::from_address(addr);
        let get = flipper.get();
        let flip = flipper.flip();
        let mut input = [0u8; 512];
        let mut output = [0u8; 512];

        let res = get.call_raw(&mut input, &mut output)?;
        assert_eq!(res, false);
        let _ = flip.call_raw(&mut input, &mut output)?;
        let res = get.call_raw(&mut input, &mut output)?;
        assert_eq!(res, true);
        // test deployed
        let mut code_hash = [0; 32];
        let _ = api::code_hash(&addr.0, &mut code_hash);
        let f = new_Flipper();
        let deposit_limit = ruint::aliases::U256::from(u128::MAX);
        let deposit_limit = deposit_limit.to_be_bytes();
        let (addr, _) = f.instantiate_raw(
            &code_hash,
            0,
            RefTimeAndProofSizeLimits {
                ref_time_limit: u64::MAX,
                proof_size_limit: u64::MAX,
                deposit_limit: deposit_limit,
            },
            None,
            &mut input,
            &mut output,
        )?;
        let flipper = Flipper::from_address(addr);
        let get = flipper.get();
        let flip = flipper.flip();
        let mut input = [0u8; 512];
        let mut output = [0u8; 512];

        let res = get.call_raw(&mut input, &mut output)?;
        assert_eq!(res, false);
        let _ = flip.call_raw(&mut input, &mut output)?;
        let res = get.call_raw(&mut input, &mut output)?;
        assert_eq!(res, true);
        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }
}

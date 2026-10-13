#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_sdk::contract("Precompiles.sol", allocator = "pico")]
mod precompiles {
    use pvm_contract_sdk::{Address, precompiles};

    pub struct Precompiles;

    impl Precompiles {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        /// Returns the zero address on failed recovery, mirroring Solidity's
        /// builtin `ecrecover`.
        #[pvm_contract_sdk::method]
        pub fn recover(&self, hash: [u8; 32], v: u8, r: [u8; 32], s: [u8; 32]) -> Address {
            precompiles::ecrecover(self.host(), hash, v, r, s).unwrap_or(Address([0u8; 20]))
        }

        #[pvm_contract_sdk::method]
        pub fn verify_p256(
            &self,
            hash: [u8; 32],
            r: [u8; 32],
            s: [u8; 32],
            x: [u8; 32],
            y: [u8; 32],
        ) -> bool {
            precompiles::p256_verify(self.host(), hash, r, s, x, y)
        }
    }
}

#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("Packed.sol", buffer = 256)]
mod packed {
    use pvm_contract_sdk::Lazy;

    pub struct Packed {
        fee_bps: Lazy<u128>,
        max_supply: Lazy<u128>,
    }

    impl Packed {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn fee_bps(&self) -> u128 {
            self.fee_bps.get()
        }

        #[pvm_contract_sdk::method]
        pub fn max_supply(&self) -> u128 {
            self.max_supply.get()
        }

        #[pvm_contract_sdk::method]
        pub fn set_fee_bps(&mut self, v: u128) -> Result<(), pvm_contract_sdk::EmptyError> {
            self.fee_bps.set(&v);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn set_max_supply(&mut self, v: u128) -> Result<(), pvm_contract_sdk::EmptyError> {
            self.max_supply.set(&v);
            Ok(())
        }
    }
}

#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("Packed.sol", buffer = 256)]
mod packed {
    use pvm_contract_sdk::Lazy;

    #[derive(pvm_contract_sdk::SolType)]
    pub struct Settings {
        pub fee_bps: u128,
        pub max_supply: u128,
    }

    pub struct Packed {
        settings: Lazy<Settings>,
    }

    impl Packed {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn fee_bps(&self) -> u128 {
            self.settings.get().fee_bps
        }

        #[pvm_contract_sdk::method]
        pub fn max_supply(&self) -> u128 {
            self.settings.get().max_supply
        }

        #[pvm_contract_sdk::method]
        pub fn set_fee_bps(&mut self, v: u128) -> Result<(), pvm_contract_sdk::EmptyError> {
            let mut s = self.settings.get();
            s.fee_bps = v;
            self.settings.set(&s);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn set_max_supply(&mut self, v: u128) -> Result<(), pvm_contract_sdk::EmptyError> {
            let mut s = self.settings.get();
            s.max_supply = v;
            self.settings.set(&s);
            Ok(())
        }
    }
}

#![no_main]
#![no_std]

// Simplified Uniswap V2-style packed pool reserves. The two `u128` reserves
// land in the same storage slot (offsets 0 and 16) via the contract macro's
// auto-numbered slot walker, so `getReserves` is a single SLOAD and `sync`
// is a single SSTORE.
#[pvm_contract_sdk::contract("AmmReserves.sol", allocator = "bump")]
mod amm_reserves {
    use pvm_contract_sdk::Lazy;

    pub struct AmmReserves {
        reserve0: Lazy<u128>,
        reserve1: Lazy<u128>,
    }

    impl AmmReserves {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_reserves(&self) -> (u128, u128) {
            (self.reserve0.get(), self.reserve1.get())
        }

        #[pvm_contract_sdk::method]
        pub fn sync(
            &mut self,
            reserve0: u128,
            reserve1: u128,
        ) -> Result<(), pvm_contract_sdk::EmptyError> {
            self.reserve0.set(&reserve0);
            self.reserve1.set(&reserve1);
            Ok(())
        }
    }
}

#![no_main]
#![no_std]

// Simplified Uniswap V2-style packed pool reserves. The two `u128` fields
// live inside one `Lazy<Reserves>` so the storage cell occupies exactly one
// slot — `getReserves` is a single SLOAD and `sync` is a single SSTORE,
// matching solc's gas profile for `uint128 reserve0; uint128 reserve1;`.
//
// Compare with `packed_handle`-style sibling `Lazy<u128>` fields: those also
// land in the same slot via the macro's auto-numbered slot walker, but each
// `.get()` / `.set()` issues its own SLOAD (and `.set()` does a full RMW),
// so two accesses cost two host calls. The struct-in-Lazy form below
// batches both fields into a single host round-trip.
#[pvm_contract_sdk::contract("AmmReserves.sol", buffer = 256)]
mod amm_reserves {
    use pvm_contract_sdk::{EmptyError, Lazy, SolType};

    #[derive(SolType)]
    pub struct Reserves {
        pub reserve0: u128,
        pub reserve1: u128,
    }

    pub struct AmmReserves {
        reserves: Lazy<Reserves>,
    }

    impl AmmReserves {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_reserves(&self) -> (u128, u128) {
            let r = self.reserves.get();
            (r.reserve0, r.reserve1)
        }

        #[pvm_contract_sdk::method]
        pub fn sync(&mut self, reserve0: u128, reserve1: u128) -> Result<(), EmptyError> {
            self.reserves.set(&Reserves { reserve0, reserve1 });
            Ok(())
        }
    }
}

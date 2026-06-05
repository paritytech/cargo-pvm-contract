#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("Allowlist.sol", buffer = 256)]
mod allowlist {
    use pvm_contract_sdk::{Address, EmptyError, StorageVec};

    pub struct Allowlist {
        addresses: StorageVec<Address>,
    }

    impl Allowlist {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), EmptyError> {
            Ok(())
        }

        /// Append `a` to the end of the list. Duplicates allowed —
        /// `contains` returns `true` if any entry matches.
        #[pvm_contract_sdk::method]
        pub fn add(&mut self, a: Address) -> Result<(), EmptyError> {
            self.addresses.push(&a);
            Ok(())
        }

        /// Swap-and-pop removal: O(1) writes, preserves no order.
        /// Out-of-bounds `index` is a no-op.
        #[pvm_contract_sdk::method]
        pub fn remove(&mut self, index: u64) -> Result<(), EmptyError> {
            let len = self.addresses.len();
            if index >= len {
                return Ok(());
            }
            let last_idx = len - 1;
            if index != last_idx {
                let last = self.addresses.get(last_idx);
                self.addresses.set(index, &last);
            }
            self.addresses.pop();
            Ok(())
        }

        /// Linear scan — O(n). Realistic for small allowlists (governance,
        /// multisig owners). For large sets use `Mapping<Address, bool>`.
        #[pvm_contract_sdk::method]
        pub fn contains(&self, a: Address) -> bool {
            self.addresses.iter().any(|entry| entry == a)
        }

        #[pvm_contract_sdk::method]
        pub fn count(&self) -> u64 {
            self.addresses.len()
        }

        #[pvm_contract_sdk::method]
        pub fn at(&self, index: u64) -> Address {
            self.addresses.get(index)
        }
    }
}

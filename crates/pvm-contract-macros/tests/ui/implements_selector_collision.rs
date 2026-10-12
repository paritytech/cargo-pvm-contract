// An inherent `#[method]` and a folded interface method resolve to the SAME
// 4-byte selector (`transfer(address,uint256)`). Without the guard this is only an
// `unreachable_patterns` warning and one arm is silently dead; the macro must
// turn it into a hard compile error.
use pvm_contract_sdk::{Address, U256};

pub trait IErc20 {
    fn transfer(&mut self, to: Address, amount: U256) -> bool;
}

#[pvm_contract_macros::contract(implements(IErc20))]
mod token {
    use super::{Address, IErc20, U256};

    pub struct Token;

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn transfer(&mut self, to: Address, amount: U256) -> bool {
            let _ = (to, amount);
            false
        }
    }

    impl IErc20 for Token {
        fn transfer(&mut self, to: Address, amount: U256) -> bool {
            let _ = (to, amount);
            true
        }
    }
}

fn main() {}

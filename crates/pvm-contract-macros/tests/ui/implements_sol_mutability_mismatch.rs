// A folded method's inferred mutability must agree with the `.sol`
// declaration. The `.sol` declares `totalSupply` as `view`, but the trait impl
// takes `&mut self` (nonpayable) — a compile error, the safety net that also
// catches a mis-placed/omitted `#[payable]`.
use pvm_contract_sdk::U256;

pub trait IMutMismatch {
    fn total_supply(&mut self) -> U256;
}

#[pvm_contract_macros::contract("tests/ui/fixtures/IMutMismatch.sol", implements(IMutMismatch))]
mod token {
    use super::{IMutMismatch, U256};

    pub struct Token;

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IMutMismatch for Token {
        fn total_supply(&mut self) -> U256 {
            U256::ZERO
        }
    }
}

fn main() {}

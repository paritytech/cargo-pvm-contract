// `implements(IErc20)` names an interface with no `impl IErc20 for Token` block
// in the module — a typo guard.
pub trait IErc20 {
    fn total_supply(&self) -> u64;
}

#[pvm_contract_macros::contract(implements(IErc20))]
mod token {
    pub struct Token;

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }
}

fn main() {}

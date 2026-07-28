// `#[selector(name = "...")]` and `#[method(rename = "...")]` both set the Solidity
// name. Setting both is a conflict — picking one silently would drop the other.
#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::method(rename = "a")]
        #[pvm_contract_macros::selector(name = "b")]
        pub fn transfer_tokens(&self) {}
    }
}

fn main() {}

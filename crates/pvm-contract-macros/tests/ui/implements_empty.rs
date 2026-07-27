// `implements()` with no interfaces is almost certainly a mistake — reject it.
#[pvm_contract_macros::contract(implements())]
mod token {
    pub struct Token;

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }
}

fn main() {}

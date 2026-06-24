use pvm_contract_macros::SolType;

#[derive(SolType)]
pub enum WithDiscriminant {
    A = 5,
    B,
}

fn main() {}

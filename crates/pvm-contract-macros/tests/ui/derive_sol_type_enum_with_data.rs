use pvm_contract_macros::SolType;

#[derive(SolType)]
pub enum WithData {
    A(u64),
    B,
}

fn main() {}

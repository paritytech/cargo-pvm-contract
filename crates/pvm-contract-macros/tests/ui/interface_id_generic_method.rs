#[pvm_contract_macros::interface_id]
pub trait IGenericMethod {
    fn foo<T>(&self, x: T) -> u64;
}

fn main() {}

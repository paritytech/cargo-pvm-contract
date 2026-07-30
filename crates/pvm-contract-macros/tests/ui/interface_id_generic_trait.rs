// A generic interface has no single fixed ID (it would vary per type param).
#[pvm_contract_macros::interface_id]
pub trait IGeneric<T> {
    fn value(&self) -> T;
}

fn main() {}

// Two methods with the same Solidity signature produce the same selector, which
// would cancel in the interface-ID XOR. The eager guard rejects it.
#[pvm_contract_macros::interface_id]
pub trait IDup {
    fn foo(&self) -> u64;
    #[selector(name = "foo")]
    fn also_foo(&self) -> u64;
}

fn main() {}

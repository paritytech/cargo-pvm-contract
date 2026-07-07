// The macro owns `INTERFACE_ID`; the trait can't declare it itself.
#[pvm_contract_macros::interface_id]
pub trait IHasConst {
    const INTERFACE_ID: [u8; 4];
    fn foo(&self) -> u64;
}

fn main() {}

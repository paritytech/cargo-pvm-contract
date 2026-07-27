// `implements(...)` requires a contract storage struct in the module.
pub trait IThing {
    fn thing(&self) -> u64;
}

#[pvm_contract_macros::contract(implements(IThing))]
mod c {}

fn main() {}

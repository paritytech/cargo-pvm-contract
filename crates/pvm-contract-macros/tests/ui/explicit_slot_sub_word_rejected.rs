// `#[slot(0)] flag: Lazy<bool>` would be placed at byte 0 of slot 0 in
// explicit mode, while solc places `bool` right-aligned at byte 31. To
// avoid the silent non-solc layout, the macro emits a const-assert that
// rejects sub-word types (`PACKED_BYTES < 32`) on explicit-slot fields.

extern crate alloc;

#[pvm_contract_macros::contract]
mod c {
    use pvm_contract_sdk::Lazy;

    pub struct C {
        #[slot(0)]
        flag: Lazy<bool>,
    }

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

fn main() {}

// `#[derive(SolType, SolStorage)]` on a struct with > 8 storage slots that
// also contains a dynamic field (String / Bytes / nested dynamic struct)
// must compile-fail. The macro emits a const-block in the generated
// `write_to_storage` / `read_from_storage` / `try_read_from_storage` /
// `clear_storage` overrides that asserts `STORAGE_SLOTS <= 8` — the
// MAX_STATIC_SLOTS cap from `pvm-storage`. The inline stack buffer used
// by the macro-emitted body is `[[0u8; 32]; 8]`; oversized structs would
// overflow it, so we reject them up front.
//
// `SolType` alone wouldn't trigger this (it emits ABI traits only). The
// assertion lives on the `SolStorage` derive's dynamic-struct path.

extern crate alloc;

use pvm_contract_macros::{SolStorage, SolType};
use pvm_contract_types::U256;

// 9 U256 fields (one slot each) + 1 String field at slot 9 = 10 slots total,
// exceeding MAX_STATIC_SLOTS = 8. Triggers the compile-time assertion in the
// generated `write_to_storage` const-block.
#[derive(SolType, SolStorage)]
pub struct TooManySlotsWithDynamic {
    pub a0: U256,
    pub a1: U256,
    pub a2: U256,
    pub a3: U256,
    pub a4: U256,
    pub a5: U256,
    pub a6: U256,
    pub a7: U256,
    pub a8: U256,
    pub note: alloc::string::String,
}

fn main() {}

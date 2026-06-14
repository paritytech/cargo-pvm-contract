// `#[derive(SolType)]` on a struct with > 64 storage slots that also contains
// a dynamic field (String / Bytes / nested dynamic struct) must compile-fail.
// The internal `dynamic_mask: u64` bitmask in the generated
// `__STORAGE_LAYOUT` const tracks which slots are owned by dynamic fields;
// with the mask sized as `u64` it can index at most 64 slots, and the runtime
// `1u64 << __i` shift in the generated `write_to_storage` / `clear_storage`
// would otherwise be UB in release builds at slot >= 64.

extern crate alloc;

use pvm_contract_macros::SolType;
use pvm_contract_types::U256;

// 64 U256 fields (one slot each) + 1 String field at slot 64. Triggers
// the new compile-time assertion in the generated dynamic-impls block.
#[derive(SolType)]
pub struct TooManySlotsWithDynamic {
    pub a0:  U256, pub a1:  U256, pub a2:  U256, pub a3:  U256,
    pub a4:  U256, pub a5:  U256, pub a6:  U256, pub a7:  U256,
    pub a8:  U256, pub a9:  U256, pub a10: U256, pub a11: U256,
    pub a12: U256, pub a13: U256, pub a14: U256, pub a15: U256,
    pub a16: U256, pub a17: U256, pub a18: U256, pub a19: U256,
    pub a20: U256, pub a21: U256, pub a22: U256, pub a23: U256,
    pub a24: U256, pub a25: U256, pub a26: U256, pub a27: U256,
    pub a28: U256, pub a29: U256, pub a30: U256, pub a31: U256,
    pub a32: U256, pub a33: U256, pub a34: U256, pub a35: U256,
    pub a36: U256, pub a37: U256, pub a38: U256, pub a39: U256,
    pub a40: U256, pub a41: U256, pub a42: U256, pub a43: U256,
    pub a44: U256, pub a45: U256, pub a46: U256, pub a47: U256,
    pub a48: U256, pub a49: U256, pub a50: U256, pub a51: U256,
    pub a52: U256, pub a53: U256, pub a54: U256, pub a55: U256,
    pub a56: U256, pub a57: U256, pub a58: U256, pub a59: U256,
    pub a60: U256, pub a61: U256, pub a62: U256, pub a63: U256,
    pub note: alloc::string::String,
}

fn main() {}

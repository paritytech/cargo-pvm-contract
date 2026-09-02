// An interface method with a default body still contributes its selector to the
// XOR'd `INTERFACE_ID`, but `implements(...)` only folds methods the impl
// restates. A defaulted-but-unrestated method would therefore be advertised by
// ERC-165 `supportsInterface(INTERFACE_ID)` yet have no dispatch arm. Reject it.
#[pvm_contract_macros::interface_id]
pub trait IThing {
    fn required(&self) -> u64;

    fn defaulted(&self) -> u64 {
        0
    }
}

fn main() {}

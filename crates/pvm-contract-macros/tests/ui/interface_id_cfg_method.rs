// A `#[cfg]`-gated interface method still contributes its selector to the XOR
// (proc-macros run before cfg stripping), so `INTERFACE_ID` would not match the
// active method set. This must be a compile-time error.
#[pvm_contract_macros::interface_id]
pub trait IThing {
    fn always(&self) -> u64;

    #[cfg(any())]
    fn never(&self) -> u64;
}

fn main() {}

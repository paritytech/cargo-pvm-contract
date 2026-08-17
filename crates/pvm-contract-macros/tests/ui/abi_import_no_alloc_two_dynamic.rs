// Two offending parameters produce two diagnostics, each pointing at its own
// type. The previous inline-`compile_error!` shape emitted the same call-site
// span for every parameter, so the count stayed at two no matter how many
// parameters were at fault and the message named none of them.
pvm_contract_macros::abi_import! {
    interface NoAlloc {
        function greet(string memory who, bytes memory data) external;
    }
}

fn main() {}

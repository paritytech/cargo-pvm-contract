//! Shape test for `pvm_cdm::reference!`.
//!
//! Doesn't *run* the generated code (that would need a live PolkaVM host).
//! It only asserts the macro expansion type-checks against a stub that
//! mirrors what `abi_import!` emits — a 4-generic contract struct with a
//! `from_address` constructor on `<Pure, (), (), false>`.
//!
//! If the stub drifts from the real `abi_import!` output, this test breaks
//! and signals the macro needs updating. That's the intent.

use pvm_contract_sdk::{Address, Pure, SolDecode, SolEncode, StateMutability};

/// Minimal mirror of `abi_import!`'s output struct shape.
#[allow(dead_code)]
pub struct StubContract<
    Mutability: StateMutability,
    Inputs: SolEncode,
    Outputs: SolDecode,
    const INITIALIZED: bool,
> {
    _address: Address,
    _ph: core::marker::PhantomData<(Mutability, Inputs, Outputs)>,
}

impl StubContract<Pure, (), (), false> {
    pub fn from_address(address: Address) -> Self {
        Self {
            _address: address,
            _ph: core::marker::PhantomData,
        }
    }
}

// This is the actual shape assertion. If the macro fails to expand, or
// expands to something that doesn't type-check against the stub, the test
// file fails to compile — that's the test.
pvm_cdm::reference!(StubContract, "@test/stub");

#[test]
fn methods_exist_on_stub() {
    // Reference the generated methods as fn pointers so that removing either
    // cdm_lookup or cdm_from_env from the expansion fails the test.
    let _lookup: fn() -> StubContract<Pure, (), (), false> = StubContract::cdm_lookup;
    let _from_env: fn() -> StubContract<Pure, (), (), false> = StubContract::cdm_from_env;
}

#![cfg(not(feature = "abi-gen"))]
//! A folded interface method that returns `Result<_, Self::Error>` takes its
//! concrete error type from the `implements(ITrait<Error = Ty>)` binding — the
//! macro can't see the impl's `type Error` when it builds the ABI. The macro
//! emits a const-eval check that the binding equals the impl's real `type Error`
//! (see `tests/ui/implements_error_binding_mismatch.rs` for the rejection), so
//! the ABI-advertised error type can't drift from the one actually encoded.
//!
//! This test proves the runtime side of that guarantee: a folded `Err(e)`
//! diverges through the revert door carrying exactly the bound error type's
//! wire encoding.

use pvm_contract_sdk::{
    MockHostBuilder, OutSink, Outcome, SolDecode, SolError, U256, assert_reverts,
};

pub trait IFaulty {
    type Error;
    fn maybe(&self, ok: bool) -> Result<u64, Self::Error>;
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(IFaulty<Error = MyErr>))]
mod faulty {
    use super::{IFaulty, U256};

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct MyErr {
        pub code: U256,
    }

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IFaulty for C {
        type Error = MyErr;
        fn maybe(&self, ok: bool) -> Result<u64, Self::Error> {
            if ok {
                Ok(7)
            } else {
                Err(MyErr {
                    code: U256::from(3u64),
                })
            }
        }
    }
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

fn encode_bool(b: bool) -> Vec<u8> {
    let mut buf = vec![0u8; 32];
    if b {
        buf[31] = 1;
    }
    buf
}

#[test]
fn folded_ok_returns_encoded_value() {
    let mut contract = faulty::C::with_host(MockHostBuilder::new().build());
    let mut buf = [0u8; faulty::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;

    let outcome = faulty::route(
        &mut contract,
        selector("maybe(bool)"),
        &encode_bool(true),
        &mut out,
    );
    let Outcome::Return(n) = outcome else {
        panic!("expected Return, got {outcome:?}");
    };
    assert_eq!(u64::decode(out.view(n)).unwrap(), 7);
}

#[test]
fn folded_err_reverts_with_bound_error_type() {
    let mock = MockHostBuilder::new().build();
    let mut contract = faulty::C::with_host(mock.clone());

    // The impl's `type Error` is `MyErr`, bound via `implements(IFaulty<Error = MyErr>)`.
    // A folded `Err(MyErr { .. })` must revert carrying MyErr's exact ABI encoding.
    let err = faulty::MyErr {
        code: U256::from(3u64),
    };
    let mut expected = vec![0u8; err.encoded_size()];
    let written = err.encode_to(&mut expected);
    expected.truncate(written);

    let mut buf = [0u8; faulty::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    assert_reverts!(
        mock,
        expected,
        faulty::route(
            &mut contract,
            selector("maybe(bool)"),
            &encode_bool(false),
            &mut out
        )
    );
}

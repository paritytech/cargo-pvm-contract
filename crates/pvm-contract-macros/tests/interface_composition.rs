#![cfg(not(feature = "abi-gen"))]
//! `#[contract(implements(...))]` interface dispatch composition. One module
//! per concern: core dispatch/overrides, impl matching, per-method attributes,
//! the `<Error = Ty>` binding, the `.sol` path, and the ERC-165 pattern.

mod dispatch {
    //! `#[contract(implements(ITrait, ...))]` folds the methods of each in-module
    //! `impl ITrait for Contract` block into the dispatch table as real entry points
    //!, so an author writes forwarders once as a trait impl instead of a pile
    //! of inherent `#[method]`s. Overrides are just a different impl body.

    use pvm_contract_sdk::{
        Address, Lazy, Mapping, MockHostBuilder, OutSink, Outcome, SolDecode, U256,
    };
    use pvm_contract_types::const_selector;

    pub trait IErc20 {
        fn total_supply(&self) -> U256;
        fn balance_of(&self, account: Address) -> U256;
        fn transfer(&mut self, to: Address, amount: U256) -> bool;
    }

    pub trait IOwnable {
        fn owner(&self) -> Address;
    }

    // Shares the Rust name `value` with the inherent method below, but a different
    // signature — so a distinct selector `value(uint256)` vs `value()`.
    pub trait IValued {
        fn value(&self, key: U256) -> U256;
    }

    #[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
    #[pvm_contract_macros::contract(implements(IErc20, IOwnable, IValued))]
    mod token {
        use super::{Address, IErc20, IOwnable, IValued, Lazy, Mapping, U256};

        pub struct Token {
            total: Lazy<U256>,
            balances: Mapping<Address, U256>,
        }

        impl Token {
            #[pvm_contract_macros::constructor]
            pub fn new(&mut self) {}

            // Inherent method `value()` — shares its Rust name with the folded
            // `IValued::value(uint256)` below but has a different signature, so a
            // distinct selector. Exercises the fully-qualified trait call: neither
            // shadows the other, both dispatch.
            #[pvm_contract_macros::method]
            pub fn value(&self) -> U256 {
                U256::from(42)
            }
        }

        impl IValued for Token {
            fn value(&self, key: U256) -> U256 {
                key + U256::ONE
            }
        }

        impl IErc20 for Token {
            fn total_supply(&self) -> U256 {
                self.total.get()
            }
            fn balance_of(&self, account: Address) -> U256 {
                self.balances.get(&account)
            }
            // An "override": the impl body adds logic (rejects zero-amount
            // transfers) beyond a plain forward.
            fn transfer(&mut self, to: Address, amount: U256) -> bool {
                if amount == U256::ZERO {
                    return false;
                }
                self.balances.insert(&to, &amount);
                true
            }
        }

        impl IOwnable for Token {
            fn owner(&self) -> Address {
                Address([9u8; 20])
            }
        }
    }

    /// Route a matched method and return the encoded output. A folded method's
    /// success surfaces as `Outcome::Return(n)`, with the ABI-encoded return in the
    /// output buffer (a revert would diverge instead).
    fn route_ok(contract: &mut token::Token, sig: &str, input: &[u8]) -> Vec<u8> {
        let mut buf = [0u8; token::MAX_RETURN_LEN];
        let mut out: &mut [u8] = &mut buf;
        let outcome = token::route(contract, const_selector(sig), input, &mut out);
        let Outcome::Return(n) = outcome else {
            panic!("expected Return for `{sig}`, got {outcome:?}");
        };
        out.view(n).to_vec()
    }

    fn encode_transfer(to: Address, amount: U256) -> Vec<u8> {
        let mut input = vec![0u8; 32];
        input[12..].copy_from_slice(&to.0);
        input.extend_from_slice(&amount.to_be_bytes::<32>());
        input
    }

    #[test]
    fn folded_and_inherent_methods_dispatch() {
        let mut contract = token::Token::with_host(MockHostBuilder::new().build());
        let mut buf = [0u8; token::MAX_RETURN_LEN];

        // Two interfaces folded + the inherent `value()` all dispatch; a selector
        // not in the table misses (`Unhandled`).
        for sig in ["totalSupply()", "owner()", "value()"] {
            let mut out: &mut [u8] = &mut buf;
            assert!(
                matches!(
                    token::route(&mut contract, const_selector(sig), &[], &mut out),
                    Outcome::Return(_)
                ),
                "`{sig}` should dispatch"
            );
        }
        let mut out: &mut [u8] = &mut buf;
        assert_eq!(
            token::route(&mut contract, const_selector("nope()"), &[], &mut out),
            Outcome::Unhandled
        );
    }

    #[test]
    fn override_body_runs() {
        let mut contract = token::Token::with_host(MockHostBuilder::new().build());

        // amount == 0 hits the override's early-return `false`.
        let to = Address([7u8; 20]);
        let data = route_ok(
            &mut contract,
            "transfer(address,uint256)",
            &encode_transfer(to, U256::ZERO),
        );
        assert!(
            !bool::decode(&data).unwrap(),
            "zero transfer must return false"
        );

        // amount > 0 writes state and returns true; balance_of reads it back.
        let data = route_ok(
            &mut contract,
            "transfer(address,uint256)",
            &encode_transfer(to, U256::from(500u64)),
        );
        assert!(bool::decode(&data).unwrap());

        let mut acct = vec![0u8; 32];
        acct[12..].copy_from_slice(&to.0);
        let data = route_ok(&mut contract, "balanceOf(address)", &acct);
        assert_eq!(U256::decode(&data).unwrap(), U256::from(500u64));
    }

    #[test]
    fn inherent_and_folded_same_name_distinct_selectors() {
        let mut contract = token::Token::with_host(MockHostBuilder::new().build());

        // Inherent `value()` (selector `value()`) returns 42.
        let data = route_ok(&mut contract, "value()", &[]);
        assert_eq!(U256::decode(&data).unwrap(), U256::from(42));

        // Folded `IValued::value(uint256)` (selector `value(uint256)`) returns
        // key + 1 — same Rust name, different selector, neither shadows the other.
        let data = route_ok(
            &mut contract,
            "value(uint256)",
            &U256::from(100u64).to_be_bytes::<32>(),
        );
        assert_eq!(U256::decode(&data).unwrap(), U256::from(101u64));
    }
}

mod matching {
    //! The fold's impl-matching precision: which `impl` blocks are folded for a given
    //! `implements(...)` entry, and how the folded/collected methods are dispatched.
    //!
    //! - **Qualified path**: an `impl outer::IThing for C` is folded even when the
    //!   bare name isn't in scope, dispatching through the impl's own path.
    //! - **Sibling skip**: a same-trait impl for another struct is skipped, so the
    //!   contract's own impl is folded regardless of declaration order.
    //! - **Non-folded `#[method]`**: a `#[method]` on a same-last-segment but
    //!   different trait is still collected and dispatched via a fully-qualified
    //!   trait call.

    use pvm_contract_sdk::{MockHostBuilder, OutSink, Outcome, SolDecode, U256};
    use pvm_contract_types::const_selector;

    // --- Qualified path ----------------------------------------------------------

    // The trait lives in a submodule and is deliberately NOT re-imported under its
    // bare name `IThing` where the contract module can see it.
    pub mod outer {
        use super::U256;
        pub trait IThing {
            fn thing(&self) -> U256;
        }
    }

    #[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
    #[pvm_contract_macros::contract(implements(IThing))]
    mod qualified {
        use super::{U256, outer};

        pub struct C;

        impl C {
            #[pvm_contract_macros::constructor]
            pub fn new(&mut self) {}
        }

        // Implemented via the qualified path; bare `IThing` is not imported here.
        impl outer::IThing for C {
            fn thing(&self) -> U256 {
                U256::from(7u64)
            }
        }
    }

    #[test]
    fn qualified_path_impl_dispatches() {
        let mut contract = qualified::C::with_host(MockHostBuilder::new().build());
        let mut buf = [0u8; qualified::MAX_RETURN_LEN];
        let mut out: &mut [u8] = &mut buf;
        // Would fail to compile (bare `IThing` not in scope for the router) if the
        // fold used the `implements(...)` path instead of the `impl`'s path.
        assert!(matches!(
            qualified::route(&mut contract, const_selector("thing()"), &[], &mut out),
            Outcome::Return(_)
        ));
    }

    // --- Sibling-struct skip (order independence) --------------------------------

    pub trait ISibling {
        fn thing(&self) -> U256;
    }

    #[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
    #[pvm_contract_macros::contract(implements(ISibling))]
    mod sibling {
        use super::{ISibling, U256};

        pub struct Token;
        // A sibling struct that also implements the interface. Declared first and not
        // the contract struct, so the fold must skip it.
        pub struct Helper;

        impl ISibling for Helper {
            fn thing(&self) -> U256 {
                U256::from(111u64)
            }
        }

        impl Token {
            #[pvm_contract_macros::constructor]
            pub fn new(&mut self) {}
        }

        impl ISibling for Token {
            fn thing(&self) -> U256 {
                U256::from(42u64)
            }
        }
    }

    #[test]
    fn contract_impl_is_folded_not_the_sibling() {
        let mut contract = sibling::Token::with_host(MockHostBuilder::new().build());
        let mut buf = [0u8; sibling::MAX_RETURN_LEN];
        let mut out: &mut [u8] = &mut buf;

        let outcome = sibling::route(&mut contract, const_selector("thing()"), &[], &mut out);
        let Outcome::Return(n) = outcome else {
            panic!("expected Return, got {outcome:?}");
        };
        // 42 (Token's impl), not 111 (Helper's) — the sibling impl was skipped.
        assert_eq!(U256::decode(out.view(n)).unwrap(), U256::from(42u64));
    }

    // --- Non-folded `#[method]` on a same-last-segment trait ---------------------

    pub mod a {
        pub trait IThing {
            fn folded(&self) -> u64;
        }
    }
    pub mod b {
        pub trait IThing {
            fn extra(&self) -> u64;
        }
    }

    #[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
    #[pvm_contract_macros::contract(implements(a::IThing))]
    mod nonfolded {
        use super::a;
        use super::b;
        // `b::IThing` is intentionally NOT imported into method-call scope; the
        // collected `#[method]` dispatches via `<C as b::IThing>::extra`, which
        // resolves through the impl's own trait path regardless of imports.

        pub struct C;

        impl C {
            #[pvm_contract_macros::constructor]
            pub fn new(&mut self) {}
        }

        impl a::IThing for C {
            fn folded(&self) -> u64 {
                1
            }
        }

        // Same last segment `IThing`, different trait path -> NOT folded. Its
        // `#[method]` must still be collected as an ordinary entry point.
        impl b::IThing for C {
            #[pvm_contract_macros::method]
            fn extra(&self) -> u64 {
                2
            }
        }
    }

    fn route_u64(sig: &str) -> u64 {
        let mut contract = nonfolded::C::with_host(MockHostBuilder::new().build());
        let mut buf = [0u8; nonfolded::MAX_RETURN_LEN];
        let mut out: &mut [u8] = &mut buf;
        let outcome = nonfolded::route(&mut contract, const_selector(sig), &[], &mut out);
        let Outcome::Return(n) = outcome else {
            panic!("expected Return for `{sig}`, got {outcome:?}");
        };
        u64::decode(out.view(n)).unwrap()
    }

    #[test]
    fn folded_and_nonfolded_method_both_dispatch() {
        assert_eq!(route_u64("folded()"), 1); // from the folded a::IThing
        assert_eq!(route_u64("extra()"), 2); // from the non-folded b::IThing #[method]
    }
}

mod attributes {
    //! Per-method attributes on a *folded* interface method behave at runtime exactly
    //! as on an inherent `#[method]`. The parse-level tests in `contract.rs` prove the
    //! attributes are read into the folded `MethodInfo`; these drive the generated
    //! `route()` against a `MockHost` to prove the emitted guards actually fire.
    //!
    //! - `#[payable]` on the impl fn: the folded method accepts value, while a
    //!   non-payable folded sibling reverts on a value transfer.
    //! - `#[non_reentrant]` on the impl fn: the folded method reverts with the
    //!   OZ-compatible `ReentrancyGuardReentrantCall` when the lock is held, and
    //!   otherwise runs and leaves the lock clear.

    use pvm_contract_types::{
        HostApi, MockHost, MockHostBuilder, OutSink, Outcome, ReturnFlags, SolDecode, StorageFlags,
        const_keccak256, const_selector,
    };

    // ----------------------------------------------------------------------------
    // #[payable] on a folded method
    // ----------------------------------------------------------------------------

    pub trait IVault {
        // Payability isn't part of the Rust signature, so the trait can't carry it;
        // `#[payable]` goes on the impl fn (mirroring the inherent path).
        fn deposit(&mut self) -> u64;
        fn poke(&mut self) -> u64;
    }

    #[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
    #[pvm_contract_macros::contract(implements(IVault))]
    mod vault {
        use super::IVault;

        pub struct V;

        impl V {
            #[pvm_contract_macros::constructor]
            pub fn new(&mut self) {}
        }

        impl IVault for V {
            #[pvm_contract_macros::payable]
            fn deposit(&mut self) -> u64 {
                1
            }
            fn poke(&mut self) -> u64 {
                2
            }
        }
    }

    #[test]
    fn folded_payable_accepts_value_and_non_payable_sibling_rejects_it() {
        // A non-zero value transfer against a contract with mixed folded payability.
        let mock = MockHostBuilder::new().value_transferred([0x11; 32]).build();
        let mut contract = vault::V::with_host(mock.clone());
        let mut buf = [0u8; vault::MAX_RETURN_LEN];

        // The `#[payable]` folded method accepts the value and returns normally.
        let mut out: &mut [u8] = &mut buf;
        let outcome = vault::route(&mut contract, const_selector("deposit()"), &[], &mut out);
        let Outcome::Return(n) = outcome else {
            panic!("payable folded method should accept value, got {outcome:?}");
        };
        assert_eq!(u64::decode(out.view(n)).unwrap(), 1);

        // The non-payable folded sibling reverts with the framework's
        // `NonPayableValueReceived` selector via the per-arm value guard.
        let mut out: &mut [u8] = &mut buf;
        let rv = mock.expect_revert(|| {
            vault::route(&mut contract, const_selector("poke()"), &[], &mut out);
        });
        assert_eq!(rv.flags, ReturnFlags::REVERT);
        assert_eq!(
            rv.data.as_slice(),
            &pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED[..],
        );
    }

    #[test]
    fn folded_non_payable_accepts_zero_value() {
        // With no value transfer the non-payable folded method runs as usual.
        let mut contract = vault::V::with_host(MockHostBuilder::new().build());
        let mut buf = [0u8; vault::MAX_RETURN_LEN];
        let mut out: &mut [u8] = &mut buf;
        let outcome = vault::route(&mut contract, const_selector("poke()"), &[], &mut out);
        let Outcome::Return(n) = outcome else {
            panic!("expected Return, got {outcome:?}");
        };
        assert_eq!(u64::decode(out.view(n)).unwrap(), 2);
    }

    // ----------------------------------------------------------------------------
    // #[non_reentrant] on a folded method
    // ----------------------------------------------------------------------------

    pub trait IGuarded {
        fn guarded(&mut self) -> u64;
    }

    #[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
    #[pvm_contract_macros::contract(implements(IGuarded))]
    mod guarded {
        use super::IGuarded;

        pub struct G;

        impl G {
            #[pvm_contract_macros::constructor]
            pub fn new(&mut self) {}
        }

        impl IGuarded for G {
            #[pvm_contract_macros::non_reentrant]
            fn guarded(&mut self) -> u64 {
                7
            }
        }
    }

    const REENTRANCY_KEY: [u8; 32] = const_keccak256(b"pvm.guards.reentrancy");

    fn lock_is_set(mock: &MockHost) -> bool {
        let mut buf = [0u8; 32];
        mock.get_storage_or_zero(StorageFlags::empty(), &REENTRANCY_KEY, &mut buf);
        buf != [0u8; 32]
    }

    #[test]
    fn folded_non_reentrant_reverts_when_lock_held() {
        let mock = MockHostBuilder::new().build();
        let mut contract = guarded::G::with_host(mock.clone());
        // Simulate "a guarded section is already in progress".
        mock.set_storage_or_clear(StorageFlags::empty(), &REENTRANCY_KEY, &[1u8; 32]);

        let mut buf = [0u8; guarded::MAX_RETURN_LEN];
        let mut out: &mut [u8] = &mut buf;
        let rv = mock.expect_revert(|| {
            guarded::route(&mut contract, const_selector("guarded()"), &[], &mut out);
        });
        assert_eq!(rv.flags, ReturnFlags::REVERT);
        assert_eq!(
            &rv.data[..4],
            &const_selector("ReentrancyGuardReentrantCall()"),
        );
    }

    #[test]
    fn folded_non_reentrant_succeeds_and_clears_lock_when_unlocked() {
        let mock = MockHostBuilder::new().build();
        let mut contract = guarded::G::with_host(mock.clone());

        let mut buf = [0u8; guarded::MAX_RETURN_LEN];
        let mut out: &mut [u8] = &mut buf;
        let outcome = guarded::route(&mut contract, const_selector("guarded()"), &[], &mut out);
        let Outcome::Return(n) = outcome else {
            panic!("expected Return, got {outcome:?}");
        };
        assert_eq!(u64::decode(out.view(n)).unwrap(), 7);
        // The full guard sets-then-clears the lock across the call.
        assert!(!lock_is_set(&mock), "guard must leave the lock clear");
    }
}

mod error_binding {
    //! A folded interface method that returns `Result<_, Self::Error>` takes its
    //! concrete error type from the `implements(ITrait<Error = Ty>)` binding — the
    //! macro can't see the impl's `type Error` when it builds the ABI. The macro
    //! emits a const-eval check that the binding equals the impl's real `type Error`
    //! (a mismatch is rejected at compile time), so the ABI-advertised error type
    //! can't drift from the one actually encoded.
    //!
    //! This test proves the runtime side of that guarantee: a folded `Err(e)`
    //! diverges through the revert door carrying exactly the bound error type's
    //! wire encoding.

    use pvm_contract_sdk::{
        MockHostBuilder, OutSink, Outcome, SolDecode, SolError, U256, assert_reverts,
    };
    use pvm_contract_types::const_selector;

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
            const_selector("maybe(bool)"),
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
                const_selector("maybe(bool)"),
                &encode_bool(false),
                &mut out
            )
        );
    }
}

mod sol_path {
    //! `implements(...)` combined with a `.sol` interface. Folded methods are
    //! resolved against the `.sol` (coverage + parameter + mutability cross-checks)
    //! and dispatch under the `.sol`-derived selectors, exactly like inherent
    //! `#[method]`s.

    use pvm_contract_sdk::{Address, Outcome, U256};
    use pvm_contract_types::const_selector;

    pub trait IComposed {
        fn total_supply(&self) -> U256;
        fn balance_of(&self, account: Address) -> U256;
    }

    #[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
    #[pvm_contract_macros::contract("tests/fixtures/IComposed.sol", implements(IComposed))]
    mod token {
        use super::{Address, IComposed, U256};

        pub struct Token {
            total: pvm_contract_sdk::Lazy<U256>,
        }

        impl Token {
            #[pvm_contract_macros::constructor]
            pub fn new(&mut self) {}
        }

        impl IComposed for Token {
            fn total_supply(&self) -> U256 {
                self.total.get()
            }
            fn balance_of(&self, account: Address) -> U256 {
                let _ = account;
                U256::ZERO
            }
        }
    }

    #[test]
    fn folded_methods_dispatch_under_sol_selectors() {
        let mut contract =
            token::Token::with_host(pvm_contract_types::MockHostBuilder::new().build());
        let mut buf = [0u8; token::MAX_RETURN_LEN];

        // Both `.sol` functions are satisfied by the folded trait impl and dispatch
        // under the interface's canonical selectors.
        let mut out: &mut [u8] = &mut buf;
        assert!(matches!(
            token::route(
                &mut contract,
                const_selector("totalSupply()"),
                &[],
                &mut out
            ),
            Outcome::Return(_)
        ));
        let mut out: &mut [u8] = &mut buf;
        assert!(matches!(
            token::route(
                &mut contract,
                const_selector("balanceOf(address)"),
                &[0u8; 32],
                &mut out
            ),
            Outcome::Return(_)
        ));
    }
}

mod erc165 {
    //! ERC-165 falls out of `#[interface_id]` + `implements(...)` with
    //! no generated code: define an `IErc165` interface, list it in
    //! `implements(...)`, and hand-write the 3-liner using the `INTERFACE_ID` consts.

    use pvm_contract_sdk::{MockHostBuilder, OutSink, Outcome, SolDecode, U256};

    #[pvm_contract_macros::interface_id]
    pub trait IErc20 {
        fn total_supply(&self) -> U256;
        fn transfer(&mut self, to: pvm_contract_sdk::Address, amount: U256) -> bool;
    }

    pub trait IErc165 {
        fn supports_interface(&self, id: [u8; 4]) -> bool;
    }

    #[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
    #[pvm_contract_macros::contract(implements(IErc20, IErc165))]
    mod token {
        use super::{IErc20, IErc165, U256};
        use pvm_contract_sdk::Address;

        pub struct Token;

        impl Token {
            #[pvm_contract_macros::constructor]
            pub fn new(&mut self) {}
        }

        impl IErc20 for Token {
            fn total_supply(&self) -> U256 {
                U256::ZERO
            }
            fn transfer(&mut self, _to: Address, _amount: U256) -> bool {
                true
            }
        }

        impl IErc165 for Token {
            fn supports_interface(&self, id: [u8; 4]) -> bool {
                id == [0x01, 0xff, 0xc9, 0xa7] // ERC-165 itself
                || id == <Token as IErc20>::INTERFACE_ID
            }
        }
    }

    fn supports(contract: &mut token::Token, id: [u8; 4]) -> bool {
        let mut input = vec![0u8; 32];
        input[..4].copy_from_slice(&id);
        let mut buf = [0u8; token::MAX_RETURN_LEN];
        let mut out: &mut [u8] = &mut buf;
        let outcome = token::route(
            contract,
            pvm_contract_types::const_selector("supportsInterface(bytes4)"),
            &input,
            &mut out,
        );
        let Outcome::Return(n) = outcome else {
            panic!("expected Return, got {outcome:?}");
        };
        bool::decode(out.view(n)).unwrap()
    }

    #[test]
    fn supports_interface_answers_for_known_ids() {
        let mut contract = token::Token::with_host(MockHostBuilder::new().build());

        assert!(supports(&mut contract, [0x01, 0xff, 0xc9, 0xa7]));
        assert!(supports(
            &mut contract,
            <token::Token as IErc20>::INTERFACE_ID
        ));
        assert!(!supports(&mut contract, [0xde, 0xad, 0xbe, 0xef]));
    }
}

//! The corpus calldata, pushed through the macro's real dispatch.
//!
//! The TypeScript suite proves the ABI JSON and the SDK's encoders agree. This
//! closes the remaining link: the selector that `#[contract]` *dispatches on*
//! is the one the ABI advertises, and the bytes the method returns are the
//! `returndata` the fixture promises. Both come from different code paths
//! (`dispatch.rs` computes selectors from the Rust signature; abi-gen renders
//! the item from `abi_param`), so a drift between them would be invisible to
//! viem, which only ever sees the ABI.
//!
//! Runs against `MockHost` on the host target; no riscv build. The macro emits
//! `route()` / `with_host()` only when the crate is built *without* `abi-gen`
//! (that feature turns the contract into an ABI-emitting stub), so this test
//! is compiled only for `cargo test -p pvm-viem-roundtrip --no-default-features`.
#![cfg(not(feature = "abi-gen"))]

use pvm_contract_types::{MockHostBuilder, OutSink, Outcome};
use pvm_viem_roundtrip::corpus;
use pvm_viem_roundtrip::surface::abi_surface::{self, AbiSurface};

/// Corpus cases whose dispatch outcome is *known* to differ from what the
/// fixture (and viem) hold, with the reason. The suite's job is to find
/// these, not fix them: each is asserted to still diverge, so a fix cannot
/// land without retiring the entry.
///
/// Both entries share one root cause, pinned by
/// `dynamic_composite_decode_at_disagrees_with_decode` below: for a fixed
/// array of dynamic elements and for a dynamic `#[derive(SolType)]` struct,
/// `SolDecode::decode_at(input, head)` treats `head` as the *body* start
/// instead of dereferencing the head pointer the way `String` / `Vec` do.
/// Dispatch decodes every parameter through `decode_at`; every existing
/// encoding test in the workspace decoded through `decode`, which takes the
/// pointer-following branch itself, so the mismatch was invisible until the
/// corpus calldata was pushed through dispatch.
const KNOWN_DIVERGENT: &[(&str, Divergence, &str)] = &[
    (
        "echoFixedStrings",
        Divergence::WrongReturn,
        "the `[String; 2]` argument is misdecoded (each element comes back holding raw \
         tail bytes), so the echoed return is garbage that viem decodes into NUL-filled \
         strings",
    ),
    (
        "echoProfile",
        Divergence::Reverts,
        "the dynamic `Profile` argument fails to decode, so dispatch reverts with \
         `InvalidCalldata()` on calldata viem encoded",
    ),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Divergence {
    /// Dispatch returns, but not the fixture's bytes.
    WrongReturn,
    /// Dispatch reverts with a framework error.
    Reverts,
}

fn known_divergence(id: &str) -> Option<(Divergence, &'static str)> {
    KNOWN_DIVERGENT
        .iter()
        .find(|(case, _, _)| *case == id)
        .map(|(_, kind, reason)| (*kind, *reason))
}

fn unhex(s: &str) -> Vec<u8> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn every_abi_surface_function_case_dispatches_and_returns_its_fixture_bytes() {
    let fixtures = corpus::build();
    let surface = fixtures
        .contracts
        .iter()
        .find(|c| c.name == "abi-surface")
        .expect("abi-surface corpus entry");
    assert!(!surface.functions.is_empty());
    let mut divergent_seen = 0;

    for case in &surface.functions {
        let calldata = unhex(&case.calldata);
        let (selector, input) = calldata.split_at(4);
        let selector: [u8; 4] = selector.try_into().unwrap();

        let mock = MockHostBuilder::new().build();
        let mut contract = AbiSurface::with_host(mock.clone());
        // A plain slice is the no-alloc `OutSink`; the contract's own sink on
        // chain is the alloc one (stack + `Vec` spill), so the slice has to be
        // large enough for every dynamic return in the corpus, not just
        // `MAX_RETURN_LEN`, which is only the stack part.
        let mut buf = vec![0u8; 8 * 1024];
        let mut out: &mut [u8] = &mut buf;

        // Every revert on the host target unwinds through `MockHost::revert`;
        // catch it so the case id and the recorded payload are reported rather
        // than an opaque `Box<dyn Any>`.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            abi_surface::route(&mut contract, selector, input, &mut out)
        }));
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(_) => {
                let reverted = mock
                    .take_return_value()
                    .unwrap_or_else(|| panic!("{}: panicked without a recorded revert", case.id));
                let data = pvm_viem_roundtrip::hex(&reverted.data);
                if case.id == "alwaysFails" {
                    // The one method whose body returns `Err`: dispatch encodes
                    // it and reverts through the host with the `Unauthorized()`
                    // payload the error corpus pins.
                    let unauthorized = surface
                        .errors
                        .iter()
                        .find(|e| e.id == "Unauthorized")
                        .expect("Unauthorized error case");
                    assert_eq!(data, unauthorized.data, "{}: revert payload", case.id);
                    continue;
                }
                if let Some((Divergence::Reverts, _)) = known_divergence(&case.id) {
                    divergent_seen += 1;
                    continue;
                }
                panic!(
                    "{}: {} reverted unexpectedly with {data} (flags {:?})",
                    case.id, case.signature, reverted.flags
                );
            }
        };
        if let Some((Divergence::Reverts, reason)) = known_divergence(&case.id) {
            panic!(
                "{}: dispatch no longer reverts; remove it from KNOWN_DIVERGENT ({reason})",
                case.id
            );
        }
        let Outcome::Return(len) = outcome else {
            panic!(
                "{}: selector {} ({}) was not dispatched: {outcome:?}",
                case.id, case.selector, case.signature
            );
        };
        let returned = pvm_viem_roundtrip::hex(out.view(len));
        let expected = case.returndata.clone().unwrap_or_else(|| "0x".to_string());
        if let Some((Divergence::WrongReturn, reason)) = known_divergence(&case.id) {
            assert_ne!(
                returned, expected,
                "{}: dispatch now returns the fixture bytes; remove it from KNOWN_DIVERGENT \
                 ({reason})",
                case.id
            );
            divergent_seen += 1;
            continue;
        }
        assert_eq!(returned, expected, "{}: return data", case.id);
    }
    assert_eq!(
        divergent_seen,
        KNOWN_DIVERGENT.len(),
        "a KNOWN_DIVERGENT entry names a case that no longer exists"
    );
}

/// The root cause behind both `KNOWN_DIVERGENT` entries, isolated from
/// dispatch: the two `SolDecode` doors disagree on the same canonical bytes.
/// `decode` is what every encoding test in the workspace uses; `decode_at` is
/// what the macro's parameter decoding calls. Asserted as still-divergent so
/// the fix retires this test together with the entries above.
#[test]
fn dynamic_composite_decode_at_disagrees_with_decode() {
    use pvm_contract_types::SolDecode;
    use pvm_viem_roundtrip::surface::abi_surface::Profile;

    let fixtures = corpus::build();
    let surface = fixtures
        .contracts
        .iter()
        .find(|c| c.name == "abi-surface")
        .unwrap();
    let input_of = |id: &str| {
        let case = surface.functions.iter().find(|f| f.id == id).unwrap();
        unhex(&case.calldata)[4..].to_vec()
    };

    // A dynamic *leaf* is fine through both doors; it is the composite layer
    // that disagrees. That makes this a composite-decoding bug rather than a
    // convention difference between the two methods.
    let mixed = input_of("mixed");
    assert!(<(ruint::aliases::U256, String) as SolDecode>::decode_at(&mixed, 0).is_ok());

    let strings = input_of("echoFixedStrings");
    let via_decode = <[String; 2] as SolDecode>::decode(&strings).expect("decode accepts");
    assert_eq!(via_decode, ["".to_string(), "tail".to_string()]);
    let via_decode_at = <[String; 2] as SolDecode>::decode_at(&strings, 0).ok();
    assert_ne!(
        via_decode_at,
        Some(via_decode),
        "[String; 2]::decode_at now agrees with decode; retire the echoFixedStrings entry"
    );

    let profile = input_of("echoProfile");
    let via_decode = <Profile as SolDecode>::decode(&profile).expect("decode accepts");
    assert_eq!(via_decode.name, "profile");
    assert!(
        <Profile as SolDecode>::decode_at(&profile, 0).is_err(),
        "Profile::decode_at now accepts the bytes; retire the echoProfile entry"
    );
}

#[test]
fn an_unknown_selector_is_not_dispatched() {
    let mock = MockHostBuilder::new().build();
    let mut contract = AbiSurface::with_host(mock);
    let mut buf = vec![0u8; 8 * 1024];
    let mut out: &mut [u8] = &mut buf;
    assert_eq!(
        abi_surface::route(&mut contract, [0xde, 0xad, 0xbe, 0xef], &[], &mut out),
        Outcome::Unhandled
    );
}

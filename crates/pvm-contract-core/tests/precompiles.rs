//! Unit tests for the typed precompile wrappers.
//!
//! The wrappers own the input/output ABI layout and the fixed addresses; those
//! are what these tests pin down. The actual cryptography only runs on a node,
//! so here the precompile call itself is mocked through [`MockHost`] — the tests
//! assert the wrapper builds the spec-mandated calldata and decodes the raw
//! output correctly. Notice that secp256k1 / P-256 verification is not
//! performed.
//!
//! The signature constants below are nonetheless real, verified vectors, so
//! the on-chain e2e tests can reuse them against the actual precompiles.

use std::rc::Rc;

use pvm_contract_core::precompiles::{self, address};
use pvm_contract_types::{Address, Host, MockHost, MockHostBuilder};

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2));
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn arr32(s: &str) -> [u8; 32] {
    hex(s).try_into().unwrap()
}

fn host_with(mock: &MockHost) -> Host {
    Host::from_dyn(Rc::new(mock.clone()))
}

/// A 32-byte big-endian word holding an address in its low 20 bytes — the shape
/// `ecrecover` returns.
fn address_word(addr: [u8; 20]) -> Vec<u8> {
    let mut word = vec![0u8; 32];
    word[12..].copy_from_slice(&addr);
    word
}

/// A 32-byte big-endian word equal to `1` — the "valid" P256Verify output.
fn one_word() -> Vec<u8> {
    let mut word = vec![0u8; 32];
    word[31] = 1;
    word
}

// ---------------------------------------------------------------------------
// Fixed-address constants
// ---------------------------------------------------------------------------

#[test]
fn precompile_addresses_match_spec() {
    let at = |low: u8| {
        let mut a = [0u8; 20];
        a[19] = low;
        Address(a)
    };
    assert_eq!(address::ECRECOVER, at(0x01));
    assert_eq!(address::SHA256, at(0x02));
    assert_eq!(address::RIPEMD160, at(0x03));
    assert_eq!(address::IDENTITY, at(0x04));
    assert_eq!(address::MODEXP, at(0x05));
    assert_eq!(address::BN128_ADD, at(0x06));
    assert_eq!(address::BN128_MUL, at(0x07));
    assert_eq!(address::BN128_PAIRING, at(0x08));
    assert_eq!(address::BLAKE2F, at(0x09));
    assert_eq!(address::POINT_EVAL, at(0x0a));

    // P256Verify (RIP-7212) lives at 0x100 = 256.
    let mut p256 = [0u8; 20];
    p256[18] = 0x01;
    assert_eq!(address::P256_VERIFY, Address(p256));
}

// ---------------------------------------------------------------------------
// Usable from `&self` (view) methods
// ---------------------------------------------------------------------------

/// A stand-in for a contract struct holding a host handle. The wrappers must be
/// callable from a `&self` method (no `&mut`), mirroring how a `#[contract]`
/// view method reaches them via `self.host()`.
struct ViewContract {
    host: Host,
}

impl ViewContract {
    fn recover(&self, hash: [u8; 32], v: u8, r: [u8; 32], s: [u8; 32]) -> Option<Address> {
        precompiles::ecrecover(&self.host, hash, v, r, s)
    }

    fn verify(&self, hash: [u8; 32], sig: ([u8; 32], [u8; 32]), key: ([u8; 32], [u8; 32])) -> bool {
        precompiles::p256_verify(&self.host, hash, sig.0, sig.1, key.0, key.1)
    }
}

#[test]
fn wrappers_are_callable_from_view_methods() {
    let mock = MockHostBuilder::new().build();
    let addr: [u8; 20] = hex(ECR_ADDR).try_into().unwrap();
    mock.mock_call(address::ECRECOVER.0, Ok(address_word(addr)));
    mock.mock_call(address::P256_VERIFY.0, Ok(one_word()));

    let contract = ViewContract {
        host: host_with(&mock),
    };

    assert_eq!(
        contract.recover(arr32(ECR_HASH), ECR_V, arr32(ECR_R), arr32(ECR_S)),
        Some(Address(addr))
    );
    assert!(contract.verify(
        arr32(P256_HASH),
        (arr32(P256_R), arr32(P256_S)),
        (arr32(P256_X), arr32(P256_Y)),
    ));
}

// ---------------------------------------------------------------------------
// ecrecover — published Ethereum test vector
// ---------------------------------------------------------------------------

// Widely published ecrecover vector (hash, v, r, s) -> recovered address.
const ECR_HASH: &str = "456e9aea5e197a1f1af7a3e85a3212fa4049a3ba34c2289b4c860fc0b0c64ef3";
const ECR_V: u8 = 28;
const ECR_R: &str = "9242685bf161793cc25603c231bc2f568eb630ea16aa137d2664ac8038825608";
const ECR_S: &str = "4f8ae3bd7535248d0bd448298cc2e2071e56992d0774dc340c368ae950852ada";
const ECR_ADDR: &str = "7156526fbd7a3c72969b54f64e42c10fbb768c8a";

#[test]
fn ecrecover_builds_exact_128_byte_input() {
    let mock = MockHostBuilder::new().build();
    // Return the known address so the call succeeds; we assert on the input.
    let addr: [u8; 20] = hex(ECR_ADDR).try_into().unwrap();
    mock.mock_call(address::ECRECOVER.0, Ok(address_word(addr)));
    let host = host_with(&mock);

    let _ = precompiles::ecrecover(&host, arr32(ECR_HASH), ECR_V, arr32(ECR_R), arr32(ECR_S));

    // Spec input: hash(32) ‖ v(32, right-aligned) ‖ r(32) ‖ s(32) = 128 bytes.
    let mut expected = Vec::new();
    expected.extend_from_slice(&hex(ECR_HASH));
    let mut v_word = [0u8; 32];
    v_word[31] = ECR_V;
    expected.extend_from_slice(&v_word);
    expected.extend_from_slice(&hex(ECR_R));
    expected.extend_from_slice(&hex(ECR_S));
    assert_eq!(expected.len(), 128);

    let calls = mock.recorded_calls();
    assert_eq!(calls, vec![(address::ECRECOVER.0, expected)]);
}

#[test]
fn ecrecover_decodes_recovered_address() {
    let mock = MockHostBuilder::new().build();
    let addr: [u8; 20] = hex(ECR_ADDR).try_into().unwrap();
    mock.mock_call(address::ECRECOVER.0, Ok(address_word(addr)));
    let host = host_with(&mock);

    let recovered =
        precompiles::ecrecover(&host, arr32(ECR_HASH), ECR_V, arr32(ECR_R), arr32(ECR_S));

    assert_eq!(recovered, Some(Address(addr)));
}

#[test]
fn ecrecover_empty_output_is_none() {
    let mock = MockHostBuilder::new().build();
    // Failed recovery: the precompile returns empty output.
    mock.mock_call(address::ECRECOVER.0, Ok(Vec::new()));
    let host = host_with(&mock);

    let recovered =
        precompiles::ecrecover(&host, arr32(ECR_HASH), ECR_V, arr32(ECR_R), arr32(ECR_S));

    assert_eq!(recovered, None);
}

#[test]
fn ecrecover_zero_output_is_none() {
    let mock = MockHostBuilder::new().build();
    // A full 32-byte zero word decodes to the zero address, which the wrapper
    // reports as "no address" rather than `Address::ZERO`.
    mock.mock_call(address::ECRECOVER.0, Ok(vec![0u8; 32]));
    let host = host_with(&mock);

    let recovered =
        precompiles::ecrecover(&host, arr32(ECR_HASH), ECR_V, arr32(ECR_R), arr32(ECR_S));

    assert_eq!(recovered, None);
}

#[test]
fn ecrecover_unexpected_output_len_is_none() {
    // Anything other than the spec's 32-byte word is rejected outright — a
    // truncated or over-long output must not be reinterpreted as an address.
    for len in [1usize, 16, 31, 33, 64] {
        let mock = MockHostBuilder::new().build();
        mock.mock_call(address::ECRECOVER.0, Ok(vec![0xAAu8; len]));
        let host = host_with(&mock);

        let recovered =
            precompiles::ecrecover(&host, arr32(ECR_HASH), ECR_V, arr32(ECR_R), arr32(ECR_S));

        assert_eq!(recovered, None, "{len}-byte output must not decode");
    }
}

#[test]
fn ecrecover_reverted_call_is_none() {
    let mock = MockHostBuilder::new().build();
    mock.mock_call(address::ECRECOVER.0, Err(()));
    let host = host_with(&mock);

    let recovered =
        precompiles::ecrecover(&host, arr32(ECR_HASH), ECR_V, arr32(ECR_R), arr32(ECR_S));

    assert_eq!(recovered, None);
}

#[test]
fn ecrecover_normalizes_raw_recovery_id() {
    // Signing libraries commonly hand back v as 0/1; the precompile only
    // understands 27/28, so the wrapper adds the offset.
    let mock = MockHostBuilder::new().build();
    let addr: [u8; 20] = hex(ECR_ADDR).try_into().unwrap();
    mock.mock_call(address::ECRECOVER.0, Ok(address_word(addr)));
    let host = host_with(&mock);

    for (raw, normalized) in [(0u8, 27u8), (1, 28)] {
        let _ = precompiles::ecrecover(&host, arr32(ECR_HASH), raw, arr32(ECR_R), arr32(ECR_S));

        let calls = mock.take_recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1[32..64], {
            let mut word = [0u8; 32];
            word[31] = normalized;
            word
        });
    }
}

#[test]
fn ecrecover_passes_through_out_of_range_recovery_id() {
    // Normalization only lifts 0/1 into 27/28. An EIP-155 style v stays as-is
    // so the precompile — not the wrapper — decides it is invalid.
    let mock = MockHostBuilder::new().build();
    mock.mock_call(address::ECRECOVER.0, Ok(Vec::new()));
    let host = host_with(&mock);

    let _ = precompiles::ecrecover(&host, arr32(ECR_HASH), 37, arr32(ECR_R), arr32(ECR_S));

    let calls = mock.recorded_calls();
    assert_eq!(calls[0].1[63], 37);
}

// ---------------------------------------------------------------------------
// P256Verify (RIP-7212)
// ---------------------------------------------------------------------------

// go-ethereum's `CallP256Verify` vector, the same one pallet-revive's own
// P256Verify tests run against — a genuinely valid secp256r1 signature over
// `P256_HASH` by the public key `(P256_X, P256_Y)`.
const P256_HASH: &str = "4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4d";
const P256_R: &str = "a73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac";
const P256_S: &str = "36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d60";
const P256_X: &str = "4aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff3";
const P256_Y: &str = "7618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e";

fn p256_input_expected() -> Vec<u8> {
    let mut expected = Vec::new();
    for part in [P256_HASH, P256_R, P256_S, P256_X, P256_Y] {
        expected.extend_from_slice(&hex(part));
    }
    expected
}

#[test]
fn p256_verify_builds_exact_160_byte_input() {
    let mock = MockHostBuilder::new().build();
    mock.mock_call(address::P256_VERIFY.0, Ok(one_word()));
    let host = host_with(&mock);

    let _ = precompiles::p256_verify(
        &host,
        arr32(P256_HASH),
        arr32(P256_R),
        arr32(P256_S),
        arr32(P256_X),
        arr32(P256_Y),
    );

    let expected = p256_input_expected();
    assert_eq!(expected.len(), 160);
    let calls = mock.recorded_calls();
    assert_eq!(calls, vec![(address::P256_VERIFY.0, expected)]);
}

#[test]
fn p256_verify_valid_output_is_true() {
    let mock = MockHostBuilder::new().build();
    mock.mock_call(address::P256_VERIFY.0, Ok(one_word()));
    let host = host_with(&mock);

    let valid = precompiles::p256_verify(
        &host,
        arr32(P256_HASH),
        arr32(P256_R),
        arr32(P256_S),
        arr32(P256_X),
        arr32(P256_Y),
    );

    assert!(valid);
}

#[test]
fn p256_verify_empty_output_is_false() {
    let mock = MockHostBuilder::new().build();
    mock.mock_call(address::P256_VERIFY.0, Ok(Vec::new()));
    let host = host_with(&mock);

    let valid = precompiles::p256_verify(
        &host,
        arr32(P256_HASH),
        arr32(P256_R),
        arr32(P256_S),
        arr32(P256_X),
        arr32(P256_Y),
    );

    assert!(!valid);
}

#[test]
fn p256_verify_zero_output_is_false() {
    let mock = MockHostBuilder::new().build();
    // A full 32-byte zero word means "invalid".
    mock.mock_call(address::P256_VERIFY.0, Ok(vec![0u8; 32]));
    let host = host_with(&mock);

    let valid = precompiles::p256_verify(
        &host,
        arr32(P256_HASH),
        arr32(P256_R),
        arr32(P256_S),
        arr32(P256_X),
        arr32(P256_Y),
    );

    assert!(!valid);
}

#[test]
fn p256_verify_unexpected_output_len_is_false() {
    // Only the exact 32-byte `1` word counts as valid; a truncated or
    // over-long output must never be read as a success.
    for len in [1usize, 16, 31, 33, 64] {
        let mock = MockHostBuilder::new().build();
        let mut data = vec![0u8; len];
        data[len - 1] = 1;
        mock.mock_call(address::P256_VERIFY.0, Ok(data));
        let host = host_with(&mock);

        let valid = precompiles::p256_verify(
            &host,
            arr32(P256_HASH),
            arr32(P256_R),
            arr32(P256_S),
            arr32(P256_X),
            arr32(P256_Y),
        );

        assert!(!valid, "{len}-byte output must not verify");
    }
}

#[test]
fn p256_verify_reverted_call_is_false() {
    let mock = MockHostBuilder::new().build();
    mock.mock_call(address::P256_VERIFY.0, Err(()));
    let host = host_with(&mock);

    let valid = precompiles::p256_verify(
        &host,
        arr32(P256_HASH),
        arr32(P256_R),
        arr32(P256_S),
        arr32(P256_X),
        arr32(P256_Y),
    );

    assert!(!valid);
}

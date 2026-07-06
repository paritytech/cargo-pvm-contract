//! Unit tests for the typed precompile wrappers.
//!
//! The wrappers own the input/output ABI layout and the fixed addresses; those
//! are what these tests pin down. The actual cryptography only runs on a node,
//! so here the precompile call itself is mocked through [`MockHost`] — the tests
//! assert the wrapper builds the spec-mandated calldata and decodes the raw
//! output correctly, not that secp256k1 / P-256 verification is performed.

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
fn ecrecover_reverted_call_is_none() {
    let mock = MockHostBuilder::new().build();
    mock.mock_call(address::ECRECOVER.0, Err(()));
    let host = host_with(&mock);

    let recovered =
        precompiles::ecrecover(&host, arr32(ECR_HASH), ECR_V, arr32(ECR_R), arr32(ECR_S));

    assert_eq!(recovered, None);
}

// ---------------------------------------------------------------------------
// P256Verify (RIP-7212)
// ---------------------------------------------------------------------------

const P256_HASH: &str = "b5a77e7a90aa14e0bf5f337f06f597148676424fae26e175c6e5621c34351955";
const P256_R: &str = "289f319789da424845c9eac935245fcddd805950e2f02506d09be7e411199556";
const P256_S: &str = "3786d7b89cf7a8b9d3c0c1b6dbaebf95a25c3ba30f6bbaf4f5c4c8a3c6d2b1a9";
const P256_X: &str = "0ad99500288d466940031d72a9f5445a4d43784640855bf0a69874d2de5fe103";
const P256_Y: &str = "c5011e6ef2c42dcd50d5d3d29f99ae6eba2c80c9244f4c5422f0979ff0c3ba5e";

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

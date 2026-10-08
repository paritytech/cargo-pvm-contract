//! Typed wrappers for pallet-revive's builtin Ethereum precompiles.
//!
//! pallet-revive exposes the standard Ethereum precompiles at fixed addresses.
//! This module provides the fixed-address constants for the whole builtin set
//! plus typed wrappers for the two signature/crypto precompiles that OZ-style
//! contracts (ECDSA, EIP-712, ERC-2612 Permit, ERC-1271, P256) need most:
//! [`ecrecover`] (0x01) and [`p256_verify`] (RIP-7212, 0x100).
//!
//! Each wrapper builds the exact ABI input the precompile expects, calls the
//! fixed address with `READ_ONLY` (STATICCALL) semantics, and decodes the raw
//! 32-byte output. Because the calls are read-only, the wrappers take `&Host`
//! and are callable from `&self` (view) methods.
//!
//! The input/output layout and the addresses match the Ethereum specs, so a
//! signature that verifies on Ethereum verifies identically here — the wrapper
//! does not reinterpret the bytes, it only frames them.

use pvm_contract_types::{Address, CallFlags, Host, HostApi};

/// Fixed addresses of the builtin Ethereum precompiles in pallet-revive.
///
/// Addresses are 20-byte big-endian; the low bytes hold the precompile index
/// (`0x01`..=`0x0a` for the classic set, `0x100` for P256Verify / RIP-7212).
pub mod address {
    use pvm_contract_types::Address;

    /// Build a precompile address from its low 16-bit index.
    const fn precompile(index: u16) -> Address {
        let mut bytes = [0u8; 20];
        bytes[18] = (index >> 8) as u8;
        bytes[19] = index as u8;
        Address(bytes)
    }

    /// ecrecover — secp256k1 signature recovery (0x01).
    pub const ECRECOVER: Address = precompile(0x01);
    /// SHA-256 hash (0x02).
    pub const SHA256: Address = precompile(0x02);
    /// RIPEMD-160 hash (0x03).
    pub const RIPEMD160: Address = precompile(0x03);
    /// Identity / datacopy (0x04).
    pub const IDENTITY: Address = precompile(0x04);
    /// Modular exponentiation (0x05).
    pub const MODEXP: Address = precompile(0x05);
    /// alt_bn128 addition (0x06).
    pub const BN128_ADD: Address = precompile(0x06);
    /// alt_bn128 scalar multiplication (0x07).
    pub const BN128_MUL: Address = precompile(0x07);
    /// alt_bn128 pairing check (0x08).
    pub const BN128_PAIRING: Address = precompile(0x08);
    /// BLAKE2b F compression (0x09).
    pub const BLAKE2F: Address = precompile(0x09);
    /// KZG point evaluation — EIP-4844 (0x0a).
    ///
    /// pallet-revive reserves this address but does not implement the
    /// precompile yet; calling it fails with `UnsupportedPrecompileAddress`
    /// rather than silently transferring value.
    pub const POINT_EVAL: Address = precompile(0x0a);
    /// P256Verify — secp256r1 signature verification, RIP-7212 (0x100).
    pub const P256_VERIFY: Address = precompile(0x100);
}

/// Call a precompile at `callee` with `input`, reading its raw output into `out`.
///
/// Returns `true` only when the call succeeds and the precompile returns exactly
/// 32 bytes (the fixed output width of the wrappers here); an error or any other
/// output length (including empty output on failure) yields `false`.
fn call_precompile(host: &Host, callee: Address, input: &[u8], out: &mut [u8; 32]) -> bool {
    let value = [0u8; 32];
    if host
        .call_evm(
            CallFlags::READ_ONLY,
            &callee.0,
            u64::MAX,
            &value,
            input,
            None,
        )
        .is_err()
    {
        return false;
    }
    if host.return_data_size() != 32 {
        return false;
    }
    let mut slot = &mut out[..];
    host.return_data_copy(&mut slot, 0);
    true
}

/// Recover the signer address from an secp256k1 signature via the `ecrecover`
/// precompile (0x01).
///
/// Builds the 128-byte input `hash(32) ‖ v(32) ‖ r(32) ‖ s(32)`, where `v` is
/// the recovery id (27 or 28) placed right-aligned in its word. The precompile
/// returns the recovered address right-aligned in a 32-byte word.
///
/// A raw recovery id (`0` or `1`, what most signing libraries hand back) is
/// normalized to the EVM form by adding 27, so both conventions work. Values
/// already at or above 27 are passed through untouched — an out-of-range `v`
/// stays out of range and the precompile rejects it.
///
/// Returns `None` when recovery fails: the precompile signals this with empty
/// output, and this wrapper also maps an all-zero output word (the zero address)
/// to `None` so callers get a clean "no address" result instead of `Address::ZERO`.
///
/// Callable from `&self` methods.
pub fn ecrecover(host: &Host, hash: [u8; 32], v: u8, r: [u8; 32], s: [u8; 32]) -> Option<Address> {
    let mut input = [0u8; 128];
    input[..32].copy_from_slice(&hash);
    input[63] = if v < 27 { v + 27 } else { v };
    input[64..96].copy_from_slice(&r);
    input[96..128].copy_from_slice(&s);

    let mut out = [0u8; 32];
    if !call_precompile(host, address::ECRECOVER, &input, &mut out) {
        return None;
    }
    if out == [0u8; 32] {
        return None;
    }
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&out[12..32]);
    Some(Address(addr))
}

/// Verify a secp256r1 (P-256) signature via the P256Verify precompile
/// (RIP-7212, 0x100).
///
/// Builds the 160-byte input `hash(32) ‖ r(32) ‖ s(32) ‖ x(32) ‖ y(32)`, where
/// `(r, s)` is the signature and `(x, y)` is the public key. The precompile
/// returns a 32-byte word equal to `1` on a valid signature and empty/zero on
/// an invalid one.
///
/// Returns `true` only for the `1` output word; any other output (including
/// empty output or a failed call) is `false`.
///
/// Callable from `&self` methods.
pub fn p256_verify(
    host: &Host,
    hash: [u8; 32],
    r: [u8; 32],
    s: [u8; 32],
    x: [u8; 32],
    y: [u8; 32],
) -> bool {
    let mut input = [0u8; 160];
    input[..32].copy_from_slice(&hash);
    input[32..64].copy_from_slice(&r);
    input[64..96].copy_from_slice(&s);
    input[96..128].copy_from_slice(&x);
    input[128..160].copy_from_slice(&y);

    let mut out = [0u8; 32];
    if !call_precompile(host, address::P256_VERIFY, &input, &mut out) {
        return false;
    }
    let mut valid = [0u8; 32];
    valid[31] = 1;
    out == valid
}

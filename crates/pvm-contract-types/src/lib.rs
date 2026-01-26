#![no_std]

extern crate alloc;

use ruint::aliases::U256;

/// Trait for encoding Solidity types to ABI-encoded bytes.
///
/// This trait defines the interface for converting Rust types into their Solidity ABI-encoded
/// representation. The encoding follows the Solidity ABI specification where values are
/// typically padded to 32-byte boundaries.
///
/// # Associated Constants
///
/// - `SOL_NAME`: The Solidity type signature (e.g., "uint256", "address", "(uint256,uint256)")
/// - `ENCODED_SIZE`: The size in bytes when ABI-encoded (typically 32 for primitives)
///
/// # Example
///
/// For a `uint256` type:
/// - `SOL_NAME = "uint256"`
/// - `ENCODED_SIZE = 32`
pub trait SolEncode {
    /// The Solidity type name/signature for this type.
    ///
    /// Examples:
    /// - "uint256" for unsigned 256-bit integers
    /// - "address" for Ethereum addresses
    /// - "(uint256,uint256)" for tuples
    const SOL_NAME: &'static str;

    /// The size in bytes when this type is ABI-encoded.
    ///
    /// For most Solidity types, this is 32 bytes (one word).
    /// Complex types like arrays may have variable sizes.
    const ENCODED_SIZE: usize;

    /// Encode this value into the provided buffer at the current position.
    ///
    /// # Arguments
    ///
    /// * `buf` - A mutable byte buffer where the encoded value will be written.
    ///           The buffer must have at least `ENCODED_SIZE` bytes available.
    ///
    /// # Panics
    ///
    /// May panic if the buffer is too small to hold the encoded value.
    fn sol_encode_to(&self, buf: &mut [u8]);
}

/// Trait for decoding Solidity ABI-encoded bytes into Rust types.
///
/// This trait defines the interface for converting Solidity ABI-encoded bytes back into
/// Rust types. The decoding follows the Solidity ABI specification, reading from a
/// specified offset in the input buffer.
///
/// # Associated Constants
///
/// - `SOL_NAME`: The Solidity type signature (e.g., "uint256", "address", "(uint256,uint256)")
/// - `ENCODED_SIZE`: The size in bytes when ABI-encoded (typically 32 for primitives)
///
/// # Example
///
/// For a `uint256` type:
/// - `SOL_NAME = "uint256"`
/// - `ENCODED_SIZE = 32`
pub trait SolDecode: Sized {
    /// The Solidity type name/signature for this type.
    ///
    /// Examples:
    /// - "uint256" for unsigned 256-bit integers
    /// - "address" for Ethereum addresses
    /// - "(uint256,uint256)" for tuples
    const SOL_NAME: &'static str;

    /// The size in bytes when this type is ABI-encoded.
    ///
    /// For most Solidity types, this is 32 bytes (one word).
    /// Complex types like arrays may have variable sizes.
    const ENCODED_SIZE: usize;

    /// Decode a value from the provided input buffer starting at the given offset.
    ///
    /// # Arguments
    ///
    /// * `input` - The byte buffer containing ABI-encoded data.
    /// * `offset` - The byte offset in the buffer where this value starts.
    ///
    /// # Returns
    ///
    /// A new instance of `Self` decoded from the buffer.
    ///
    /// # Panics
    ///
    /// May panic if the buffer doesn't contain enough data at the specified offset.
    fn sol_decode(input: &[u8], offset: usize) -> Self;
}

// ============================================================================
// Primitive Type Implementations
// ============================================================================

// U256 (uint256)
impl SolEncode for U256 {
    const SOL_NAME: &'static str = "uint256";
    const ENCODED_SIZE: usize = 32;

    fn sol_encode_to(&self, buf: &mut [u8]) {
        let bytes = self.to_be_bytes::<32>();
        buf[..32].copy_from_slice(&bytes);
    }
}

impl SolDecode for U256 {
    const SOL_NAME: &'static str = "uint256";
    const ENCODED_SIZE: usize = 32;

    fn sol_decode(input: &[u8], offset: usize) -> Self {
        U256::from_be_slice(&input[offset..offset + 32])
    }
}

// u128 (uint128)
impl SolEncode for u128 {
    const SOL_NAME: &'static str = "uint128";
    const ENCODED_SIZE: usize = 32;

    fn sol_encode_to(&self, buf: &mut [u8]) {
        buf[..16].fill(0);
        buf[16..32].copy_from_slice(&self.to_be_bytes());
    }
}

impl SolDecode for u128 {
    const SOL_NAME: &'static str = "uint128";
    const ENCODED_SIZE: usize = 32;

    fn sol_decode(input: &[u8], offset: usize) -> Self {
        let bytes: [u8; 16] = input[offset + 16..offset + 32].try_into().unwrap();
        u128::from_be_bytes(bytes)
    }
}

// u64 (uint64)
impl SolEncode for u64 {
    const SOL_NAME: &'static str = "uint64";
    const ENCODED_SIZE: usize = 32;

    fn sol_encode_to(&self, buf: &mut [u8]) {
        buf[..24].fill(0);
        buf[24..32].copy_from_slice(&self.to_be_bytes());
    }
}

impl SolDecode for u64 {
    const SOL_NAME: &'static str = "uint64";
    const ENCODED_SIZE: usize = 32;

    fn sol_decode(input: &[u8], offset: usize) -> Self {
        let bytes: [u8; 8] = input[offset + 24..offset + 32].try_into().unwrap();
        u64::from_be_bytes(bytes)
    }
}

// u32 (uint32)
impl SolEncode for u32 {
    const SOL_NAME: &'static str = "uint32";
    const ENCODED_SIZE: usize = 32;

    fn sol_encode_to(&self, buf: &mut [u8]) {
        buf[..28].fill(0);
        buf[28..32].copy_from_slice(&self.to_be_bytes());
    }
}

impl SolDecode for u32 {
    const SOL_NAME: &'static str = "uint32";
    const ENCODED_SIZE: usize = 32;

    fn sol_decode(input: &[u8], offset: usize) -> Self {
        let bytes: [u8; 4] = input[offset + 28..offset + 32].try_into().unwrap();
        u32::from_be_bytes(bytes)
    }
}

// u16 (uint16)
impl SolEncode for u16 {
    const SOL_NAME: &'static str = "uint16";
    const ENCODED_SIZE: usize = 32;

    fn sol_encode_to(&self, buf: &mut [u8]) {
        buf[..30].fill(0);
        buf[30..32].copy_from_slice(&self.to_be_bytes());
    }
}

impl SolDecode for u16 {
    const SOL_NAME: &'static str = "uint16";
    const ENCODED_SIZE: usize = 32;

    fn sol_decode(input: &[u8], offset: usize) -> Self {
        u16::from_be_bytes([input[offset + 30], input[offset + 31]])
    }
}

// u8 (uint8)
impl SolEncode for u8 {
    const SOL_NAME: &'static str = "uint8";
    const ENCODED_SIZE: usize = 32;

    fn sol_encode_to(&self, buf: &mut [u8]) {
        buf[..31].fill(0);
        buf[31] = *self;
    }
}

impl SolDecode for u8 {
    const SOL_NAME: &'static str = "uint8";
    const ENCODED_SIZE: usize = 32;

    fn sol_decode(input: &[u8], offset: usize) -> Self {
        input[offset + 31]
    }
}

// bool
impl SolEncode for bool {
    const SOL_NAME: &'static str = "bool";
    const ENCODED_SIZE: usize = 32;

    fn sol_encode_to(&self, buf: &mut [u8]) {
        buf[..31].fill(0);
        buf[31] = if *self { 1 } else { 0 };
    }
}

impl SolDecode for bool {
    const SOL_NAME: &'static str = "bool";
    const ENCODED_SIZE: usize = 32;

    fn sol_decode(input: &[u8], offset: usize) -> Self {
        input[offset + 31] != 0
    }
}

// [u8; 20] (address)
impl SolEncode for [u8; 20] {
    const SOL_NAME: &'static str = "address";
    const ENCODED_SIZE: usize = 32;

    fn sol_encode_to(&self, buf: &mut [u8]) {
        buf[..12].fill(0);
        buf[12..32].copy_from_slice(self);
    }
}

impl SolDecode for [u8; 20] {
    const SOL_NAME: &'static str = "address";
    const ENCODED_SIZE: usize = 32;

    fn sol_decode(input: &[u8], offset: usize) -> Self {
        let mut result = [0u8; 20];
        result.copy_from_slice(&input[offset + 12..offset + 32]);
        result
    }
}

// [u8; 32] (bytes32)
impl SolEncode for [u8; 32] {
    const SOL_NAME: &'static str = "bytes32";
    const ENCODED_SIZE: usize = 32;

    fn sol_encode_to(&self, buf: &mut [u8]) {
        buf[..32].copy_from_slice(self);
    }
}

impl SolDecode for [u8; 32] {
    const SOL_NAME: &'static str = "bytes32";
    const ENCODED_SIZE: usize = 32;

    fn sol_decode(input: &[u8], offset: usize) -> Self {
        let mut result = [0u8; 32];
        result.copy_from_slice(&input[offset..offset + 32]);
        result
    }
}

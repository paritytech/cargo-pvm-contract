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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_u256() {
        let mut buf = [0u8; 32];

        // Test zero value
        let val = U256::from(0u64);
        val.sol_encode_to(&mut buf);
        assert_eq!(U256::sol_decode(&buf, 0), val);

        // Test small value
        let val = U256::from(42u64);
        val.sol_encode_to(&mut buf);
        assert_eq!(U256::sol_decode(&buf, 0), val);

        // Test large value
        let val = U256::from(u64::MAX);
        val.sol_encode_to(&mut buf);
        assert_eq!(U256::sol_decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u128() {
        let mut buf = [0u8; 32];

        // Test zero value
        let val = 0u128;
        val.sol_encode_to(&mut buf);
        assert_eq!(u128::sol_decode(&buf, 0), val);

        // Test small value
        let val = 12345u128;
        val.sol_encode_to(&mut buf);
        assert_eq!(u128::sol_decode(&buf, 0), val);

        // Test max value
        let val = u128::MAX;
        val.sol_encode_to(&mut buf);
        assert_eq!(u128::sol_decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u64() {
        let mut buf = [0u8; 32];

        // Test zero value
        let val = 0u64;
        val.sol_encode_to(&mut buf);
        assert_eq!(u64::sol_decode(&buf, 0), val);

        // Test small value
        let val = 999u64;
        val.sol_encode_to(&mut buf);
        assert_eq!(u64::sol_decode(&buf, 0), val);

        // Test max value
        let val = u64::MAX;
        val.sol_encode_to(&mut buf);
        assert_eq!(u64::sol_decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u32() {
        let mut buf = [0u8; 32];

        // Test zero value
        let val = 0u32;
        val.sol_encode_to(&mut buf);
        assert_eq!(u32::sol_decode(&buf, 0), val);

        // Test small value
        let val = 1234u32;
        val.sol_encode_to(&mut buf);
        assert_eq!(u32::sol_decode(&buf, 0), val);

        // Test max value
        let val = u32::MAX;
        val.sol_encode_to(&mut buf);
        assert_eq!(u32::sol_decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u16() {
        let mut buf = [0u8; 32];

        // Test zero value
        let val = 0u16;
        val.sol_encode_to(&mut buf);
        assert_eq!(u16::sol_decode(&buf, 0), val);

        // Test small value
        let val = 256u16;
        val.sol_encode_to(&mut buf);
        assert_eq!(u16::sol_decode(&buf, 0), val);

        // Test max value
        let val = u16::MAX;
        val.sol_encode_to(&mut buf);
        assert_eq!(u16::sol_decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u8() {
        let mut buf = [0u8; 32];

        // Test zero value
        let val = 0u8;
        val.sol_encode_to(&mut buf);
        assert_eq!(u8::sol_decode(&buf, 0), val);

        // Test small value
        let val = 42u8;
        val.sol_encode_to(&mut buf);
        assert_eq!(u8::sol_decode(&buf, 0), val);

        // Test max value
        let val = u8::MAX;
        val.sol_encode_to(&mut buf);
        assert_eq!(u8::sol_decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_bool() {
        let mut buf = [0u8; 32];

        // Test false
        let val = false;
        val.sol_encode_to(&mut buf);
        assert_eq!(bool::sol_decode(&buf, 0), val);

        // Test true
        let val = true;
        val.sol_encode_to(&mut buf);
        assert_eq!(bool::sol_decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_address() {
        let mut buf = [0u8; 32];

        // Test zero address
        let val = [0u8; 20];
        val.sol_encode_to(&mut buf);
        assert_eq!(<[u8; 20]>::sol_decode(&buf, 0), val);

        // Test non-zero address
        let val = [0x42u8; 20];
        val.sol_encode_to(&mut buf);
        assert_eq!(<[u8; 20]>::sol_decode(&buf, 0), val);

        // Test mixed address
        let mut val = [0u8; 20];
        val[0] = 0xAB;
        val[19] = 0xCD;
        val.sol_encode_to(&mut buf);
        assert_eq!(<[u8; 20]>::sol_decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_bytes32() {
        let mut buf = [0u8; 32];

        // Test zero bytes
        let val = [0u8; 32];
        val.sol_encode_to(&mut buf);
        assert_eq!(<[u8; 32]>::sol_decode(&buf, 0), val);

        // Test all ones
        let val = [0xFFu8; 32];
        val.sol_encode_to(&mut buf);
        assert_eq!(<[u8; 32]>::sol_decode(&buf, 0), val);

        // Test mixed pattern
        let mut val = [0u8; 32];
        val[0] = 0x12;
        val[15] = 0x34;
        val[31] = 0x56;
        val.sol_encode_to(&mut buf);
        assert_eq!(<[u8; 32]>::sol_decode(&buf, 0), val);
    }

    #[test]
    fn test_encoding_format_u8() {
        let mut buf = [0u8; 32];

        // u8 should be right-aligned (at byte 31)
        let val = 1u8;
        val.sol_encode_to(&mut buf);
        assert_eq!(buf[31], 1);
        assert!(buf[..31].iter().all(|&b| b == 0));

        // Test with max value
        let val = u8::MAX;
        val.sol_encode_to(&mut buf);
        assert_eq!(buf[31], u8::MAX);
        assert!(buf[..31].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_bool() {
        let mut buf = [0u8; 32];

        // true should be 0x01 at byte 31
        let val = true;
        val.sol_encode_to(&mut buf);
        assert_eq!(buf[31], 1);
        assert!(buf[..31].iter().all(|&b| b == 0));

        // false should be 0x00 at byte 31
        let val = false;
        val.sol_encode_to(&mut buf);
        assert_eq!(buf[31], 0);
        assert!(buf[..31].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_address() {
        let mut buf = [0u8; 32];

        // Address should have 12 zero prefix bytes
        let addr = [0x42u8; 20];
        addr.sol_encode_to(&mut buf);
        assert!(buf[..12].iter().all(|&b| b == 0));
        assert_eq!(&buf[12..32], &addr[..]);
    }

    #[test]
    fn test_encoding_format_u16() {
        let mut buf = [0u8; 32];

        // u16 should be right-aligned (at bytes 30-31)
        let val = 0x1234u16;
        val.sol_encode_to(&mut buf);
        assert_eq!(&buf[30..32], &[0x12, 0x34]);
        assert!(buf[..30].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_u32() {
        let mut buf = [0u8; 32];

        // u32 should be right-aligned (at bytes 28-31)
        let val = 0x12345678u32;
        val.sol_encode_to(&mut buf);
        assert_eq!(&buf[28..32], &[0x12, 0x34, 0x56, 0x78]);
        assert!(buf[..28].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_u64() {
        let mut buf = [0u8; 32];

        // u64 should be right-aligned (at bytes 24-31)
        let val = 0x0102030405060708u64;
        val.sol_encode_to(&mut buf);
        assert_eq!(
            &buf[24..32],
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
        assert!(buf[..24].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_u128() {
        let mut buf = [0u8; 32];

        // u128 should be right-aligned (at bytes 16-31)
        let val = 0x0102030405060708090A0B0C0D0E0F10u128;
        val.sol_encode_to(&mut buf);
        assert_eq!(
            &buf[16..32],
            &[
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
                0x0F, 0x10
            ]
        );
        assert!(buf[..16].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_bytes32() {
        let mut buf = [0u8; 32];

        // bytes32 should fill entire buffer
        let val = [0xAAu8; 32];
        val.sol_encode_to(&mut buf);
        assert_eq!(&buf[..], &val[..]);
    }
}

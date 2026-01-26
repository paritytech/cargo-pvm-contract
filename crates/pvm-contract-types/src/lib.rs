#![no_std]

extern crate alloc;

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

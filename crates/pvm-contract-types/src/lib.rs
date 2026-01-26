#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

use ruint::aliases::U256;

/// Trait for encoding Rust types to Solidity ABI-encoded bytes.
///
/// Implemented by both static types (fixed size at compile time) and
/// dynamic types (size determined at runtime).
pub trait SolEncode {
    /// The Solidity type name/signature (e.g., "uint256", "address", "string").
    const SOL_NAME: &'static str;

    /// Returns the encoded length in bytes for this value.
    ///
    /// For static types, this equals `StaticEncodedLen::ENCODED_SIZE`.
    /// For dynamic types (String, bytes), this is computed at runtime.
    fn encode_len(&self) -> usize;

    /// Encode this value into the provided buffer.
    ///
    /// The buffer must have at least `encode_len()` bytes available.
    fn encode_to(&self, buf: &mut [u8]);
}

/// Trait for dynamic types that need special encoding when embedded in structs/tuples.
///
/// Dynamic types (String, Vec<T>, bytes) encode differently when standalone vs embedded:
/// - Standalone: offset pointer + length + data
/// - Embedded (tail): length + data only (offset written by parent struct)
///
/// This trait is an implementation detail used by the derive macro.
pub trait DynSolEncode: SolEncode {
    /// Returns the tail length (excludes offset pointer).
    /// For dynamic types: length prefix + data + padding.
    fn tail_len(&self) -> usize;

    /// Encode just the tail portion (no offset pointer).
    /// Writes: length prefix + data + padding.
    fn encode_tail_to(&self, buf: &mut [u8]);
}

/// Marker trait for types with compile-time known encoded size.
///
/// Static types (U256, address, bool, etc.) implement this trait.
/// Dynamic types (String, bytes, `Vec<T>`) do NOT implement this trait.
///
/// The macro uses this trait to generate fixed-size stack buffers.
/// If a method returns a type without `StaticEncodedLen`, the user must
/// add `#[pvm_contract::method(dyn_len)]` to use dynamic allocation.
pub trait StaticEncodedLen: SolEncode {
    /// The size in bytes when ABI-encoded (compile-time constant).
    const ENCODED_SIZE: usize;
}

/// Trait for decoding Solidity ABI-encoded bytes into Rust types.
pub trait SolDecode: Sized {
    /// The Solidity type name/signature for this type.
    const SOL_NAME: &'static str;

    /// The size in bytes when this type is ABI-encoded.
    const ENCODED_SIZE: usize;

    /// Decode a value from the input buffer at the given offset.
    fn decode(input: &[u8], offset: usize) -> Self;
}

// ============================================================================
// Macro helpers for implementing traits with less boilerplate
// ============================================================================

macro_rules! impl_static_encode {
    ($ty:ty, $sol_name:expr, $size:expr, $encode_fn:expr) => {
        impl SolEncode for $ty {
            const SOL_NAME: &'static str = $sol_name;

            #[inline]
            fn encode_len(&self) -> usize {
                $size
            }

            fn encode_to(&self, buf: &mut [u8]) {
                $encode_fn(self, buf)
            }
        }

        impl StaticEncodedLen for $ty {
            const ENCODED_SIZE: usize = $size;
        }
    };
}

macro_rules! impl_decode {
    ($ty:ty, $sol_name:expr, $size:expr, $decode_fn:expr) => {
        impl SolDecode for $ty {
            const SOL_NAME: &'static str = $sol_name;
            const ENCODED_SIZE: usize = $size;

            fn decode(input: &[u8], offset: usize) -> Self {
                $decode_fn(input, offset)
            }
        }
    };
}

// ============================================================================
// Primitive Type Implementations
// ============================================================================

// U256 (uint256)
impl_static_encode!(U256, "uint256", 32, |val: &U256, buf: &mut [u8]| {
    buf[..32].copy_from_slice(&val.to_be_bytes::<32>());
});
impl_decode!(U256, "uint256", 32, |input: &[u8], offset: usize| {
    U256::from_be_slice(&input[offset..offset + 32])
});

// u128 (uint128)
impl_static_encode!(u128, "uint128", 32, |val: &u128, buf: &mut [u8]| {
    buf[..16].fill(0);
    buf[16..32].copy_from_slice(&val.to_be_bytes());
});
impl_decode!(u128, "uint128", 32, |input: &[u8], offset: usize| {
    let bytes: [u8; 16] = input[offset + 16..offset + 32].try_into().unwrap();
    u128::from_be_bytes(bytes)
});

// u64 (uint64)
impl_static_encode!(u64, "uint64", 32, |val: &u64, buf: &mut [u8]| {
    buf[..24].fill(0);
    buf[24..32].copy_from_slice(&val.to_be_bytes());
});
impl_decode!(u64, "uint64", 32, |input: &[u8], offset: usize| {
    let bytes: [u8; 8] = input[offset + 24..offset + 32].try_into().unwrap();
    u64::from_be_bytes(bytes)
});

// u32 (uint32)
impl_static_encode!(u32, "uint32", 32, |val: &u32, buf: &mut [u8]| {
    buf[..28].fill(0);
    buf[28..32].copy_from_slice(&val.to_be_bytes());
});
impl_decode!(u32, "uint32", 32, |input: &[u8], offset: usize| {
    let bytes: [u8; 4] = input[offset + 28..offset + 32].try_into().unwrap();
    u32::from_be_bytes(bytes)
});

// u16 (uint16)
impl_static_encode!(u16, "uint16", 32, |val: &u16, buf: &mut [u8]| {
    buf[..30].fill(0);
    buf[30..32].copy_from_slice(&val.to_be_bytes());
});
impl_decode!(u16, "uint16", 32, |input: &[u8], offset: usize| {
    u16::from_be_bytes([input[offset + 30], input[offset + 31]])
});

// u8 (uint8)
impl_static_encode!(u8, "uint8", 32, |val: &u8, buf: &mut [u8]| {
    buf[..31].fill(0);
    buf[31] = *val;
});
impl_decode!(u8, "uint8", 32, |input: &[u8], offset: usize| {
    input[offset + 31]
});

// bool
impl_static_encode!(bool, "bool", 32, |val: &bool, buf: &mut [u8]| {
    buf[..31].fill(0);
    buf[31] = if *val { 1 } else { 0 };
});
impl_decode!(bool, "bool", 32, |input: &[u8], offset: usize| {
    input[offset + 31] != 0
});

// [u8; 20] (address)
impl_static_encode!([u8; 20], "address", 32, |val: &[u8; 20], buf: &mut [u8]| {
    buf[..12].fill(0);
    buf[12..32].copy_from_slice(val);
});
impl_decode!([u8; 20], "address", 32, |input: &[u8], offset: usize| {
    let mut result = [0u8; 20];
    result.copy_from_slice(&input[offset + 12..offset + 32]);
    result
});

// [u8; 32] (bytes32)
impl_static_encode!([u8; 32], "bytes32", 32, |val: &[u8; 32], buf: &mut [u8]| {
    buf[..32].copy_from_slice(val);
});
impl_decode!([u8; 32], "bytes32", 32, |input: &[u8], offset: usize| {
    let mut result = [0u8; 32];
    result.copy_from_slice(&input[offset..offset + 32]);
    result
});

#[cfg(feature = "alloc")]
impl SolEncode for alloc::string::String {
    const SOL_NAME: &'static str = "string";

    fn encode_len(&self) -> usize {
        let data_len = self.len();
        let padding = (32 - (data_len % 32)) % 32;
        32 + 32 + data_len + padding
    }

    fn encode_to(&self, buf: &mut [u8]) {
        let bytes = self.as_bytes();
        let data_len = bytes.len();
        let padding = (32 - (data_len % 32)) % 32;

        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&32u64.to_be_bytes());

        buf[32..64].fill(0);
        buf[56..64].copy_from_slice(&(data_len as u64).to_be_bytes());

        buf[64..64 + data_len].copy_from_slice(bytes);
        buf[64 + data_len..64 + data_len + padding].fill(0);
    }
}

#[cfg(feature = "alloc")]
impl DynSolEncode for alloc::string::String {
    fn tail_len(&self) -> usize {
        let data_len = self.len();
        let padding = (32 - (data_len % 32)) % 32;
        32 + data_len + padding
    }

    fn encode_tail_to(&self, buf: &mut [u8]) {
        let bytes = self.as_bytes();
        let data_len = bytes.len();
        let padding = (32 - (data_len % 32)) % 32;

        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&(data_len as u64).to_be_bytes());

        buf[32..32 + data_len].copy_from_slice(bytes);
        buf[32 + data_len..32 + data_len + padding].fill(0);
    }
}

#[cfg(feature = "alloc")]
impl<T: SolEncode + StaticEncodedLen> SolEncode for alloc::vec::Vec<T> {
    const SOL_NAME: &'static str = "T[]";

    fn encode_len(&self) -> usize {
        32 + 32 + self.len() * T::ENCODED_SIZE
    }

    fn encode_to(&self, buf: &mut [u8]) {
        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&32u64.to_be_bytes());
        DynSolEncode::encode_tail_to(self, &mut buf[32..]);
    }
}

#[cfg(feature = "alloc")]
impl<T: SolEncode + StaticEncodedLen> DynSolEncode for alloc::vec::Vec<T> {
    fn tail_len(&self) -> usize {
        32 + self.len() * T::ENCODED_SIZE
    }

    fn encode_tail_to(&self, buf: &mut [u8]) {
        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&(self.len() as u64).to_be_bytes());

        let mut offset = 32;
        for elem in self.iter() {
            elem.encode_to(&mut buf[offset..offset + T::ENCODED_SIZE]);
            offset += T::ENCODED_SIZE;
        }
    }
}

#[cfg(feature = "alloc")]
impl<T: SolEncode + StaticEncodedLen> SolEncode for &[T] {
    const SOL_NAME: &'static str = "T[]";

    fn encode_len(&self) -> usize {
        32 + 32 + self.len() * T::ENCODED_SIZE
    }

    fn encode_to(&self, buf: &mut [u8]) {
        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&32u64.to_be_bytes());
        DynSolEncode::encode_tail_to(self, &mut buf[32..]);
    }
}

#[cfg(feature = "alloc")]
impl<T: SolEncode + StaticEncodedLen> DynSolEncode for &[T] {
    fn tail_len(&self) -> usize {
        32 + self.len() * T::ENCODED_SIZE
    }

    fn encode_tail_to(&self, buf: &mut [u8]) {
        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&(self.len() as u64).to_be_bytes());

        let mut offset = 32;
        for elem in self.iter() {
            elem.encode_to(&mut buf[offset..offset + T::ENCODED_SIZE]);
            offset += T::ENCODED_SIZE;
        }
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

        let val = U256::from(0u64);
        val.encode_to(&mut buf);
        assert_eq!(U256::decode(&buf, 0), val);

        let val = U256::from(42u64);
        val.encode_to(&mut buf);
        assert_eq!(U256::decode(&buf, 0), val);

        let val = U256::from(u64::MAX);
        val.encode_to(&mut buf);
        assert_eq!(U256::decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u128() {
        let mut buf = [0u8; 32];

        let val = 0u128;
        val.encode_to(&mut buf);
        assert_eq!(u128::decode(&buf, 0), val);

        let val = 12345u128;
        val.encode_to(&mut buf);
        assert_eq!(u128::decode(&buf, 0), val);

        let val = u128::MAX;
        val.encode_to(&mut buf);
        assert_eq!(u128::decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u64() {
        let mut buf = [0u8; 32];

        let val = 0u64;
        val.encode_to(&mut buf);
        assert_eq!(u64::decode(&buf, 0), val);

        let val = 999u64;
        val.encode_to(&mut buf);
        assert_eq!(u64::decode(&buf, 0), val);

        let val = u64::MAX;
        val.encode_to(&mut buf);
        assert_eq!(u64::decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u32() {
        let mut buf = [0u8; 32];

        let val = 0u32;
        val.encode_to(&mut buf);
        assert_eq!(u32::decode(&buf, 0), val);

        let val = 1234u32;
        val.encode_to(&mut buf);
        assert_eq!(u32::decode(&buf, 0), val);

        let val = u32::MAX;
        val.encode_to(&mut buf);
        assert_eq!(u32::decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u16() {
        let mut buf = [0u8; 32];

        let val = 0u16;
        val.encode_to(&mut buf);
        assert_eq!(u16::decode(&buf, 0), val);

        let val = 256u16;
        val.encode_to(&mut buf);
        assert_eq!(u16::decode(&buf, 0), val);

        let val = u16::MAX;
        val.encode_to(&mut buf);
        assert_eq!(u16::decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_u8() {
        let mut buf = [0u8; 32];

        let val = 0u8;
        val.encode_to(&mut buf);
        assert_eq!(u8::decode(&buf, 0), val);

        let val = 42u8;
        val.encode_to(&mut buf);
        assert_eq!(u8::decode(&buf, 0), val);

        let val = u8::MAX;
        val.encode_to(&mut buf);
        assert_eq!(u8::decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_bool() {
        let mut buf = [0u8; 32];

        let val = false;
        val.encode_to(&mut buf);
        assert_eq!(bool::decode(&buf, 0), val);

        let val = true;
        val.encode_to(&mut buf);
        assert_eq!(bool::decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_address() {
        let mut buf = [0u8; 32];

        let val = [0u8; 20];
        val.encode_to(&mut buf);
        assert_eq!(<[u8; 20]>::decode(&buf, 0), val);

        let val = [0x42u8; 20];
        val.encode_to(&mut buf);
        assert_eq!(<[u8; 20]>::decode(&buf, 0), val);

        let mut val = [0u8; 20];
        val[0] = 0xAB;
        val[19] = 0xCD;
        val.encode_to(&mut buf);
        assert_eq!(<[u8; 20]>::decode(&buf, 0), val);
    }

    #[test]
    fn test_roundtrip_bytes32() {
        let mut buf = [0u8; 32];

        let val = [0u8; 32];
        val.encode_to(&mut buf);
        assert_eq!(<[u8; 32]>::decode(&buf, 0), val);

        let val = [0xFFu8; 32];
        val.encode_to(&mut buf);
        assert_eq!(<[u8; 32]>::decode(&buf, 0), val);

        let mut val = [0u8; 32];
        val[0] = 0x12;
        val[15] = 0x34;
        val[31] = 0x56;
        val.encode_to(&mut buf);
        assert_eq!(<[u8; 32]>::decode(&buf, 0), val);
    }

    #[test]
    fn test_encoding_format_u8() {
        let mut buf = [0u8; 32];

        let val = 1u8;
        val.encode_to(&mut buf);
        assert_eq!(buf[31], 1);
        assert!(buf[..31].iter().all(|&b| b == 0));

        let val = u8::MAX;
        val.encode_to(&mut buf);
        assert_eq!(buf[31], u8::MAX);
        assert!(buf[..31].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_bool() {
        let mut buf = [0u8; 32];

        let val = true;
        val.encode_to(&mut buf);
        assert_eq!(buf[31], 1);
        assert!(buf[..31].iter().all(|&b| b == 0));

        let val = false;
        val.encode_to(&mut buf);
        assert_eq!(buf[31], 0);
        assert!(buf[..31].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_address() {
        let mut buf = [0u8; 32];

        let addr = [0x42u8; 20];
        addr.encode_to(&mut buf);
        assert!(buf[..12].iter().all(|&b| b == 0));
        assert_eq!(&buf[12..32], &addr[..]);
    }

    #[test]
    fn test_encoding_format_u16() {
        let mut buf = [0u8; 32];

        let val = 0x1234u16;
        val.encode_to(&mut buf);
        assert_eq!(&buf[30..32], &[0x12, 0x34]);
        assert!(buf[..30].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_u32() {
        let mut buf = [0u8; 32];

        let val = 0x12345678u32;
        val.encode_to(&mut buf);
        assert_eq!(&buf[28..32], &[0x12, 0x34, 0x56, 0x78]);
        assert!(buf[..28].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_u64() {
        let mut buf = [0u8; 32];

        let val = 0x0102030405060708u64;
        val.encode_to(&mut buf);
        assert_eq!(
            &buf[24..32],
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
        assert!(buf[..24].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_encoding_format_u128() {
        let mut buf = [0u8; 32];

        let val = 0x0102030405060708090A0B0C0D0E0F10u128;
        val.encode_to(&mut buf);
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

        let val = [0xAAu8; 32];
        val.encode_to(&mut buf);
        assert_eq!(&buf[..], &val[..]);
    }

    #[test]
    fn test_static_encoded_len() {
        assert_eq!(<U256 as StaticEncodedLen>::ENCODED_SIZE, 32);
        assert_eq!(<u128 as StaticEncodedLen>::ENCODED_SIZE, 32);
        assert_eq!(<u64 as StaticEncodedLen>::ENCODED_SIZE, 32);
        assert_eq!(<u32 as StaticEncodedLen>::ENCODED_SIZE, 32);
        assert_eq!(<u16 as StaticEncodedLen>::ENCODED_SIZE, 32);
        assert_eq!(<u8 as StaticEncodedLen>::ENCODED_SIZE, 32);
        assert_eq!(<bool as StaticEncodedLen>::ENCODED_SIZE, 32);
        assert_eq!(<[u8; 20] as StaticEncodedLen>::ENCODED_SIZE, 32);
        assert_eq!(<[u8; 32] as StaticEncodedLen>::ENCODED_SIZE, 32);
    }

    #[test]
    fn test_encode_len_matches_static() {
        assert_eq!(U256::from(42u64).encode_len(), 32);
        assert_eq!(100u128.encode_len(), 32);
        assert_eq!(100u64.encode_len(), 32);
        assert_eq!(100u32.encode_len(), 32);
        assert_eq!(100u16.encode_len(), 32);
        assert_eq!(100u8.encode_len(), 32);
        assert_eq!(true.encode_len(), 32);
        assert_eq!([0u8; 20].encode_len(), 32);
        assert_eq!([0u8; 32].encode_len(), 32);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_string_encode_len() {
        let s = alloc::string::String::from("hello");
        // 32 (offset) + 32 (length) + 5 (data) + 27 (padding) = 96
        assert_eq!(s.encode_len(), 96);

        let empty = alloc::string::String::from("");
        // 32 + 32 + 0 + 0 = 64
        assert_eq!(empty.encode_len(), 64);

        let long = alloc::string::String::from("a".repeat(32));
        // 32 + 32 + 32 + 0 = 96
        assert_eq!(long.encode_len(), 96);

        let long_plus_one = alloc::string::String::from("a".repeat(33));
        // 32 + 32 + 33 + 31 = 128
        assert_eq!(long_plus_one.encode_len(), 128);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_string_encode_to() {
        let s = alloc::string::String::from("hello");
        let mut buf = alloc::vec![0u8; s.encode_len()];
        s.encode_to(&mut buf);

        // Check offset pointer (bytes 24-31 = 32)
        assert_eq!(&buf[24..32], &[0, 0, 0, 0, 0, 0, 0, 32]);

        // Check length (bytes 56-63 = 5)
        assert_eq!(&buf[56..64], &[0, 0, 0, 0, 0, 0, 0, 5]);

        // Check data
        assert_eq!(&buf[64..69], b"hello");

        // Check padding (all zeros)
        assert!(buf[69..].iter().all(|&b| b == 0));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_string_encode_empty() {
        let s = alloc::string::String::from("");
        let mut buf = alloc::vec![0u8; s.encode_len()];
        s.encode_to(&mut buf);

        // Check offset pointer (bytes 24-31 = 32)
        assert_eq!(&buf[24..32], &[0, 0, 0, 0, 0, 0, 0, 32]);

        // Check length (bytes 56-63 = 0)
        assert_eq!(&buf[56..64], &[0, 0, 0, 0, 0, 0, 0, 0]);

        // All remaining bytes should be zero
        assert!(buf[64..].iter().all(|&b| b == 0));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_string_encode_32_bytes() {
        let s = alloc::string::String::from("a".repeat(32));
        let mut buf = alloc::vec![0u8; s.encode_len()];
        s.encode_to(&mut buf);

        // Check offset pointer
        assert_eq!(&buf[24..32], &[0, 0, 0, 0, 0, 0, 0, 32]);

        // Check length (bytes 56-63 = 32)
        assert_eq!(&buf[56..64], &[0, 0, 0, 0, 0, 0, 0, 32]);

        // Check data (all 'a')
        assert!(buf[64..96].iter().all(|&b| b == b'a'));

        // No padding needed for 32-byte aligned data
        assert_eq!(buf.len(), 96);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_string_tail_len() {
        let s = alloc::string::String::from("hello");
        // tail = 32 (length) + 5 (data) + 27 (padding) = 64
        assert_eq!(s.tail_len(), 64);
        // full = 32 (offset) + 64 (tail) = 96
        assert_eq!(s.encode_len(), 96);
        assert_eq!(s.encode_len() - s.tail_len(), 32);

        let empty = alloc::string::String::from("");
        // tail = 32 (length) + 0 (data) + 0 (padding) = 32
        assert_eq!(empty.tail_len(), 32);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_string_encode_tail_to() {
        let s = alloc::string::String::from("hello");
        let mut buf = alloc::vec![0u8; s.tail_len()];
        s.encode_tail_to(&mut buf);

        // Check length (bytes 24-31 = 5)
        assert_eq!(&buf[24..32], &[0, 0, 0, 0, 0, 0, 0, 5]);

        // Check data
        assert_eq!(&buf[32..37], b"hello");

        // Check padding (all zeros)
        assert!(buf[37..].iter().all(|&b| b == 0));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_vec_u256_empty() {
        let v: alloc::vec::Vec<U256> = alloc::vec![];
        assert_eq!(v.encode_len(), 64);
        assert_eq!(v.tail_len(), 32);

        let mut buf = alloc::vec![0u8; v.encode_len()];
        v.encode_to(&mut buf);

        assert_eq!(&buf[24..32], &[0, 0, 0, 0, 0, 0, 0, 32]);
        assert_eq!(&buf[56..64], &[0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_vec_u256_one_element() {
        let v: alloc::vec::Vec<U256> = alloc::vec![U256::from(42u64)];
        assert_eq!(v.encode_len(), 96);
        assert_eq!(v.tail_len(), 64);

        let mut buf = alloc::vec![0u8; v.encode_len()];
        v.encode_to(&mut buf);

        assert_eq!(&buf[24..32], &[0, 0, 0, 0, 0, 0, 0, 32]);
        assert_eq!(&buf[56..64], &[0, 0, 0, 0, 0, 0, 0, 1]);

        let mut expected_elem = [0u8; 32];
        U256::from(42u64).encode_to(&mut expected_elem);
        assert_eq!(&buf[64..96], &expected_elem);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_vec_address_encoding() {
        let addr1 = [0x11u8; 20];
        let addr2 = [0x22u8; 20];
        let v: alloc::vec::Vec<[u8; 20]> = alloc::vec![addr1, addr2];

        assert_eq!(v.encode_len(), 128);
        assert_eq!(v.tail_len(), 96);

        let mut buf = alloc::vec![0u8; v.encode_len()];
        v.encode_to(&mut buf);

        assert_eq!(&buf[24..32], &[0, 0, 0, 0, 0, 0, 0, 32]);
        assert_eq!(&buf[56..64], &[0, 0, 0, 0, 0, 0, 0, 2]);

        assert!(buf[64..76].iter().all(|&b| b == 0));
        assert!(buf[76..96].iter().all(|&b| b == 0x11));

        assert!(buf[96..108].iter().all(|&b| b == 0));
        assert!(buf[108..128].iter().all(|&b| b == 0x22));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_vec_tail_encoding() {
        let v: alloc::vec::Vec<U256> = alloc::vec![U256::from(1u64), U256::from(2u64)];
        let mut buf = alloc::vec![0u8; v.tail_len()];
        v.encode_tail_to(&mut buf);

        assert_eq!(&buf[24..32], &[0, 0, 0, 0, 0, 0, 0, 2]);

        let mut expected1 = [0u8; 32];
        let mut expected2 = [0u8; 32];
        U256::from(1u64).encode_to(&mut expected1);
        U256::from(2u64).encode_to(&mut expected2);
        assert_eq!(&buf[32..64], &expected1);
        assert_eq!(&buf[64..96], &expected2);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_slice_encoding() {
        let data: [U256; 2] = [U256::from(100u64), U256::from(200u64)];
        let slice: &[U256] = &data;

        assert_eq!(slice.encode_len(), 128);
        assert_eq!(slice.tail_len(), 96);

        let mut buf = alloc::vec![0u8; slice.encode_len()];
        slice.encode_to(&mut buf);

        assert_eq!(&buf[24..32], &[0, 0, 0, 0, 0, 0, 0, 32]);
        assert_eq!(&buf[56..64], &[0, 0, 0, 0, 0, 0, 0, 2]);

        let mut expected1 = [0u8; 32];
        let mut expected2 = [0u8; 32];
        U256::from(100u64).encode_to(&mut expected1);
        U256::from(200u64).encode_to(&mut expected2);
        assert_eq!(&buf[64..96], &expected1);
        assert_eq!(&buf[96..128], &expected2);
    }

    #[cfg(all(test, feature = "alloc"))]
    mod alloy_comparison_tests {
        use super::*;
        use alloy_core::primitives::{Address, FixedBytes, U256 as AlloyU256};
        use alloy_core::sol_types::SolValue;

        macro_rules! assert_encoding_eq {
            ($our_val:expr, $alloy_val:expr, $msg:expr) => {{
                let mut our_buf = alloc::vec![0u8; $our_val.encode_len()];
                $our_val.encode_to(&mut our_buf);
                let alloy_buf = $alloy_val.abi_encode();
                assert_eq!(our_buf, alloy_buf, $msg);
            }};
        }

        #[test]
        fn test_alloy_uint256() {
            let value = U256::from(42u64);
            let alloy_value = AlloyU256::from(42u64);
            assert_encoding_eq!(value, alloy_value, "U256 encoding mismatch");
        }

        #[test]
        fn test_alloy_address() {
            let addr = [0x42u8; 20];
            let alloy_addr = Address::from([0x42u8; 20]);
            assert_encoding_eq!(addr, alloy_addr, "address encoding mismatch");
        }

        #[test]
        fn test_alloy_bool() {
            assert_encoding_eq!(true, true, "bool true encoding mismatch");
            assert_encoding_eq!(false, false, "bool false encoding mismatch");
        }

        #[test]
        fn test_alloy_bytes32() {
            let value = [0xAAu8; 32];
            let alloy_value = FixedBytes::<32>::from([0xAAu8; 32]);
            assert_encoding_eq!(value, alloy_value, "bytes32 encoding mismatch");
        }

        #[test]
        fn test_alloy_string() {
            let s = alloc::string::String::from("hello");
            let alloy_s = alloc::string::String::from("hello");
            assert_encoding_eq!(s, alloy_s, "string encoding mismatch");
        }

        #[test]
        fn test_alloy_uint256_array() {
            let v: alloc::vec::Vec<U256> = alloc::vec![U256::from(1u64), U256::from(2u64)];
            let alloy_v: alloc::vec::Vec<AlloyU256> =
                alloc::vec![AlloyU256::from(1u64), AlloyU256::from(2u64)];
            assert_encoding_eq!(v, alloy_v, "uint256[] encoding mismatch");
        }
    }
}

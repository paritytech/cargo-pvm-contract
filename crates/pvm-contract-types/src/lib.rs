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

impl SolEncode for &str {
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

impl DynSolEncode for &str {
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Generates all primitive type tests from a declarative specification.
    macro_rules! sol_encode_tests {
        (
            // Roundtrip tests: encode then decode, verify equality
            roundtrip {
                $( $rt_name:ident : $rt_ty:ty => [ $($rt_val:expr),+ $(,)? ] );* $(;)?
            }

            // Encoding format tests: verify specific byte positions
            format {
                $( $fmt_name:ident : $fmt_val:expr => |$buf:ident| $check:expr );* $(;)?
            }

            // StaticEncodedLen: all types should have ENCODED_SIZE = 32
            static_size { $( $st_ty:ty ),* $(,)? }

            // encode_len() should match ENCODED_SIZE for static types
            encode_len { $( $el_val:expr ),* $(,)? }
        ) => {
            // Generate roundtrip tests
            $(
                #[test]
                fn $rt_name() {
                    let mut buf = [0u8; 32];
                    $(
                        let val: $rt_ty = $rt_val;
                        val.encode_to(&mut buf);
                        assert_eq!(<$rt_ty>::decode(&buf, 0), val, "roundtrip failed for {:?}", val);
                    )+
                }
            )*

            // Generate format tests
            $(
                #[test]
                fn $fmt_name() {
                    let mut $buf = [0u8; 32];
                    ($fmt_val).encode_to(&mut $buf);
                    $check
                }
            )*

            // Generate static size test
            #[test]
            fn test_static_encoded_len() {
                $( assert_eq!(<$st_ty as StaticEncodedLen>::ENCODED_SIZE, 32); )*
            }

            // Generate encode_len test
            #[test]
            fn test_encode_len_matches_static() {
                $( assert_eq!(($el_val).encode_len(), 32); )*
            }
        };
    }

    sol_encode_tests! {
        roundtrip {
            test_u256: U256 => [U256::from(0u64), U256::from(42u64), U256::from(u64::MAX)];
            test_u128: u128 => [0u128, 12345u128, u128::MAX];
            test_u64: u64 => [0u64, 999u64, u64::MAX];
            test_u32: u32 => [0u32, 1234u32, u32::MAX];
            test_u16: u16 => [0u16, 256u16, u16::MAX];
            test_u8: u8 => [0u8, 42u8, u8::MAX];
            test_bool: bool => [false, true];
            test_address: [u8; 20] => [[0u8; 20], [0x42u8; 20]];
            test_bytes32: [u8; 32] => [[0u8; 32], [0xFFu8; 32]];
        }

        format {
            test_fmt_u8: 0xABu8 => |buf| {
                assert_eq!(buf[31], 0xAB);
                assert!(buf[..31].iter().all(|&b| b == 0));
            };
            test_fmt_bool: true => |buf| {
                assert_eq!(buf[31], 1);
                assert!(buf[..31].iter().all(|&b| b == 0));
            };
            test_fmt_u16: 0x1234u16 => |buf| {
                assert_eq!(&buf[30..32], &[0x12, 0x34]);
                assert!(buf[..30].iter().all(|&b| b == 0));
            };
            test_fmt_u32: 0x12345678u32 => |buf| {
                assert_eq!(&buf[28..32], &[0x12, 0x34, 0x56, 0x78]);
                assert!(buf[..28].iter().all(|&b| b == 0));
            };
            test_fmt_u64: 0x0102030405060708u64 => |buf| {
                assert_eq!(&buf[24..32], &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
                assert!(buf[..24].iter().all(|&b| b == 0));
            };
            test_fmt_u128: 0x0102030405060708090A0B0C0D0E0F10u128 => |buf| {
                assert_eq!(&buf[16..32], &[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]);
                assert!(buf[..16].iter().all(|&b| b == 0));
            };
            test_fmt_address: [0x42u8; 20] => |buf| {
                assert!(buf[..12].iter().all(|&b| b == 0));
                assert!(buf[12..32].iter().all(|&b| b == 0x42));
            };
            test_fmt_bytes32: [0xAAu8; 32] => |buf| {
                assert!(buf.iter().all(|&b| b == 0xAA));
            };
        }

        static_size { U256, u128, u64, u32, u16, u8, bool, [u8; 20], [u8; 32] }

        encode_len {
            U256::from(42u64), 100u128, 100u64, 100u32, 100u16, 100u8,
            true, [0u8; 20], [0u8; 32]
        }
    }

    // ========================================================================
    // Dynamic type tests (String, Vec, &str) - require alloc
    // ========================================================================

    #[cfg(feature = "alloc")]
    mod dynamic_types {
        use super::*;

        #[test]
        fn test_string_encoding() {
            // encode_len: 32 (offset) + 32 (length) + data + padding
            assert_eq!(alloc::string::String::from("").encode_len(), 64);
            assert_eq!(alloc::string::String::from("hello").encode_len(), 96);
            assert_eq!(alloc::string::String::from("a".repeat(32)).encode_len(), 96);
            assert_eq!(
                alloc::string::String::from("a".repeat(33)).encode_len(),
                128
            );

            // tail_len = encode_len - 32 (no offset)
            let s = alloc::string::String::from("hello");
            assert_eq!(s.tail_len(), 64);
            assert_eq!(s.encode_len() - s.tail_len(), 32);

            // Verify encoding structure
            let mut buf = alloc::vec![0u8; s.encode_len()];
            s.encode_to(&mut buf);
            assert_eq!(&buf[24..32], &32u64.to_be_bytes());
            assert_eq!(&buf[56..64], &5u64.to_be_bytes());
            assert_eq!(&buf[64..69], b"hello");
            assert!(buf[69..].iter().all(|&b| b == 0));
        }

        #[test]
        fn test_vec_encoding() {
            let empty: alloc::vec::Vec<U256> = alloc::vec![];
            assert_eq!(empty.encode_len(), 64);
            assert_eq!(empty.tail_len(), 32);

            let one = alloc::vec![U256::from(42u64)];
            assert_eq!(one.encode_len(), 96);
            assert_eq!(one.tail_len(), 64);

            let two: alloc::vec::Vec<[u8; 20]> = alloc::vec![[0x11; 20], [0x22; 20]];
            assert_eq!(two.encode_len(), 128);

            // Verify encoding structure
            let v = alloc::vec![U256::from(1u64), U256::from(2u64)];
            let mut buf = alloc::vec![0u8; v.encode_len()];
            v.encode_to(&mut buf);
            assert_eq!(&buf[24..32], &32u64.to_be_bytes());
            assert_eq!(&buf[56..64], &2u64.to_be_bytes());
        }

        #[test]
        fn test_slice_encoding() {
            let data = [U256::from(100u64), U256::from(200u64)];
            let slice: &[U256] = &data;
            assert_eq!(slice.encode_len(), 128);
            assert_eq!(slice.tail_len(), 96);
        }

        #[test]
        fn test_str_encoding() {
            let s: &str = "hello";
            assert_eq!(s.encode_len(), 96);
            assert_eq!(s.tail_len(), 64);
        }
    }

    // ========================================================================
    // Alloy comparison tests - verify byte-for-byte compatibility
    // ========================================================================

    #[cfg(feature = "alloc")]
    mod alloy_comparison {
        use super::*;
        use alloy_core::primitives::{Address, FixedBytes};
        use alloy_core::sol_types::SolValue;

        trait AlloyEncode {
            fn alloy_encode(&self) -> alloc::vec::Vec<u8>;
        }

        impl AlloyEncode for U256 {
            fn alloy_encode(&self) -> alloc::vec::Vec<u8> {
                self.abi_encode()
            }
        }
        impl AlloyEncode for bool {
            fn alloy_encode(&self) -> alloc::vec::Vec<u8> {
                self.abi_encode()
            }
        }
        impl AlloyEncode for [u8; 20] {
            fn alloy_encode(&self) -> alloc::vec::Vec<u8> {
                Address::from(*self).abi_encode()
            }
        }
        impl AlloyEncode for [u8; 32] {
            fn alloy_encode(&self) -> alloc::vec::Vec<u8> {
                FixedBytes::from(*self).abi_encode()
            }
        }
        impl AlloyEncode for alloc::string::String {
            fn alloy_encode(&self) -> alloc::vec::Vec<u8> {
                self.abi_encode()
            }
        }
        impl AlloyEncode for &str {
            fn alloy_encode(&self) -> alloc::vec::Vec<u8> {
                alloc::string::String::from(*self).abi_encode()
            }
        }
        impl AlloyEncode for alloc::vec::Vec<U256> {
            fn alloy_encode(&self) -> alloc::vec::Vec<u8> {
                self.abi_encode()
            }
        }

        macro_rules! assert_matches_alloy {
            (with_decode { $( $name:ident: $ty:ty = $val:expr ),* $(,)? }) => {
                $(
                    #[test]
                    fn $name() {
                        let val: $ty = $val;
                        let mut buf = [0u8; 32];
                        val.encode_to(&mut buf);
                        assert_eq!(&buf[..], &val.alloy_encode()[..], "{}: encoding mismatch", stringify!($name));
                        assert_eq!(<$ty>::decode(&buf, 0), val, "{}: decode roundtrip failed", stringify!($name));
                    }
                )*
            };
            (encode_only { $( $name:ident: $val:expr ),* $(,)? }) => {
                $(
                    #[test]
                    fn $name() {
                        let val = $val;
                        let mut our_buf = alloc::vec![0u8; val.encode_len()];
                        val.encode_to(&mut our_buf);
                        assert_eq!(our_buf, val.alloy_encode(), "{}: encoding mismatch", stringify!($name));
                    }
                )*
            };
        }

        assert_matches_alloy!(with_decode {
            test_uint256: U256 = U256::from(42u64),
            test_address: [u8; 20] = [0x42u8; 20],
            test_bool_true: bool = true,
            test_bool_false: bool = false,
            test_bytes32: [u8; 32] = [0xAAu8; 32],
        });

        assert_matches_alloy!(encode_only {
            test_string: alloc::string::String::from("hello"),
            test_str: "hello",
            test_uint256_array: alloc::vec![U256::from(1u64), U256::from(2u64)],
        });
    }
}

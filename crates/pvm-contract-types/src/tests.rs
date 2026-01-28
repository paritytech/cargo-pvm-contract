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

/// Dynamic type tests (String, Vec, &str) - require alloc
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
    fn test_str_encoding() {
        let s: &str = "hello";
        assert_eq!(s.encode_len(), 96);
        assert_eq!(s.tail_len(), 64);
    }

    #[test]
    fn test_vec_string_encoding() {
        let empty: alloc::vec::Vec<alloc::string::String> = alloc::vec![];
        assert_eq!(empty.encode_len(), 64);
        assert_eq!(empty.tail_len(), 32);

        let one = alloc::vec![alloc::string::String::from("hello")];
        let expected_len = 32 + 32 + 32 + 64; // offset + length + elem_offset + "hello" tail
        assert_eq!(one.encode_len(), expected_len);

        let two = alloc::vec![
            alloc::string::String::from("hello"),
            alloc::string::String::from("world"),
        ];
        let expected_len = 32 + 32 + 64 + 64 + 64; // offset + length + 2 offsets + 2 tails
        assert_eq!(two.encode_len(), expected_len);
    }
}

/// Alloy comparison tests - verify byte-for-byte compatibility
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
    impl AlloyEncode for alloc::vec::Vec<alloc::string::String> {
        fn alloy_encode(&self) -> alloc::vec::Vec<u8> {
            self.abi_encode()
        }
    }

    macro_rules! assert_matches_alloy {
        (static { $( $name:ident: $ty:ty = $val:expr ),* $(,)? }) => {
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
        (dynamic { $( $name:ident: $ty:ty = $val:expr ),* $(,)? }) => {
            $(
                #[test]
                fn $name() {
                    let val: $ty = $val;
                    let mut buf = alloc::vec![0u8; val.encode_len()];
                    val.encode_to(&mut buf);
                    assert_eq!(buf, val.alloy_encode(), "{}: encoding mismatch", stringify!($name));
                    assert_eq!(<$ty>::decode(&buf, 0), val, "{}: decode roundtrip failed", stringify!($name));
                }
            )*
        };
        (encode_only { $( $name:ident: $val:expr ),* $(,)? }) => {
            $(
                #[test]
                fn $name() {
                    let val = $val;
                    let mut buf = alloc::vec![0u8; val.encode_len()];
                    val.encode_to(&mut buf);
                    assert_eq!(buf, val.alloy_encode(), "{}: encoding mismatch", stringify!($name));
                }
            )*
        };
    }

    assert_matches_alloy!(static {
        test_uint256: U256 = U256::from(42u64),
        test_address: [u8; 20] = [0x42u8; 20],
        test_bool_true: bool = true,
        test_bool_false: bool = false,
        test_bytes32: [u8; 32] = [0xAAu8; 32],
    });

    assert_matches_alloy!(dynamic {
        test_string: alloc::string::String = alloc::string::String::from("hello"),
        test_uint256_array: alloc::vec::Vec<U256> = alloc::vec![U256::from(1u64), U256::from(2u64)],
        test_string_array: alloc::vec::Vec<alloc::string::String> = alloc::vec![
            alloc::string::String::from("hello"),
            alloc::string::String::from("world"),
        ],
        test_string_array_empty: alloc::vec::Vec<alloc::string::String> = alloc::vec![],
        test_string_array_single: alloc::vec::Vec<alloc::string::String> = alloc::vec![
            alloc::string::String::from("test"),
        ],
    });

    assert_matches_alloy!(encode_only { test_str: "hello" });
}

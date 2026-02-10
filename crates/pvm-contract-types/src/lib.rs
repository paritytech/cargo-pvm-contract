#![no_std]

extern crate self as pvm_contract_types;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
mod alloc_types;

use ruint::aliases::U256;

/// Trait for encoding Rust types to Solidity ABI-encoded bytes.
pub trait SolEncode {
    const SOL_NAME: &'static str;
    const IS_DYNAMIC: bool;

    fn encode_len(&self) -> usize;
    fn encode_to(&self, buf: &mut [u8]);

    fn tail_len(&self) -> usize {
        self.encode_len()
    }

    fn encode_tail_to(&self, buf: &mut [u8]) {
        self.encode_to(buf);
    }
}

/// Marker trait for types with compile-time known encoded size.
pub trait StaticEncodedLen: SolEncode {
    const ENCODED_SIZE: usize;
}

/// Trait for decoding Solidity ABI-encoded bytes into Rust types.
pub trait SolDecode: SolEncode + Sized {
    /// Decode a value from ABI-encoded input.
    fn decode(input: &[u8]) -> Self {
        Self::decode_at(input, 0)
    }

    /// Internal offset-based decode helper used by generated code.
    #[doc(hidden)]
    fn decode_at(input: &[u8], offset: usize) -> Self;

    /// Internal tail decode helper used by dynamic container decoding.
    #[doc(hidden)]
    fn decode_tail(input: &[u8], offset: usize) -> Self {
        Self::decode_at(input, offset)
    }
}

macro_rules! impl_static_type {
    ($ty:ty, $sol_name:expr, $encode_fn:expr, $decode_fn:expr) => {
        impl SolEncode for $ty {
            const SOL_NAME: &'static str = $sol_name;
            const IS_DYNAMIC: bool = false;

            #[inline]
            fn encode_len(&self) -> usize {
                32
            }

            fn encode_to(&self, buf: &mut [u8]) {
                $encode_fn(self, buf)
            }
        }

        impl StaticEncodedLen for $ty {
            const ENCODED_SIZE: usize = 32;
        }

        impl SolDecode for $ty {
            fn decode_at(input: &[u8], offset: usize) -> Self {
                $decode_fn(input, offset)
            }
        }
    };
}

impl_static_type!(
    U256,
    "uint256",
    |val: &U256, buf: &mut [u8]| buf[..32].copy_from_slice(&val.to_be_bytes::<32>()),
    |input: &[u8], offset: usize| U256::from_be_slice(&input[offset..offset + 32])
);

impl_static_type!(
    u128,
    "uint128",
    |val: &u128, buf: &mut [u8]| {
        buf[..16].fill(0);
        buf[16..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        let bytes: [u8; 16] = input[offset + 16..offset + 32].try_into().unwrap();
        u128::from_be_bytes(bytes)
    }
);

impl_static_type!(
    u64,
    "uint64",
    |val: &u64, buf: &mut [u8]| {
        buf[..24].fill(0);
        buf[24..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        let bytes: [u8; 8] = input[offset + 24..offset + 32].try_into().unwrap();
        u64::from_be_bytes(bytes)
    }
);

impl_static_type!(
    u32,
    "uint32",
    |val: &u32, buf: &mut [u8]| {
        buf[..28].fill(0);
        buf[28..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        let bytes: [u8; 4] = input[offset + 28..offset + 32].try_into().unwrap();
        u32::from_be_bytes(bytes)
    }
);

impl_static_type!(
    u16,
    "uint16",
    |val: &u16, buf: &mut [u8]| {
        buf[..30].fill(0);
        buf[30..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| u16::from_be_bytes([input[offset + 30], input[offset + 31]])
);

impl_static_type!(
    u8,
    "uint8",
    |val: &u8, buf: &mut [u8]| {
        buf[..31].fill(0);
        buf[31] = *val;
    },
    |input: &[u8], offset: usize| input[offset + 31]
);

impl_static_type!(
    bool,
    "bool",
    |val: &bool, buf: &mut [u8]| {
        buf[..31].fill(0);
        buf[31] = if *val { 1 } else { 0 };
    },
    |input: &[u8], offset: usize| input[offset + 31] != 0
);

impl_static_type!(
    [u8; 20],
    "address",
    |val: &[u8; 20], buf: &mut [u8]| {
        buf[..12].fill(0);
        buf[12..32].copy_from_slice(val);
    },
    |input: &[u8], offset: usize| {
        let mut result = [0u8; 20];
        result.copy_from_slice(&input[offset + 12..offset + 32]);
        result
    }
);

impl_static_type!(
    [u8; 32],
    "bytes32",
    |val: &[u8; 32], buf: &mut [u8]| buf[..32].copy_from_slice(val),
    |input: &[u8], offset: usize| {
        let mut result = [0u8; 32];
        result.copy_from_slice(&input[offset..offset + 32]);
        result
    }
);

impl SolEncode for &str {
    const SOL_NAME: &'static str = "string";
    const IS_DYNAMIC: bool = true;

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

#[cfg(test)]
mod tests;

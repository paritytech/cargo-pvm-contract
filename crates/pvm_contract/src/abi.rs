extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Trait for Solidity ABI encoding/decoding of types.
///
/// Each implementation corresponds to a Solidity ABI type and provides
/// the canonical name, encoding, and decoding logic.
pub trait SolAbi: Sized {
    /// The canonical Solidity type name (e.g. "uint256", "address", "bool").
    const SOL_NAME: &'static str;

    /// The ABI JSON type name. Defaults to SOL_NAME; overridden to "tuple" for structs.
    const ABI_TYPE: &'static str = Self::SOL_NAME;

    /// JSON fragment for struct components. Empty for non-struct types.
    /// For structs, starts with `,"components":[...]` (leading comma).
    const ABI_COMPONENTS: &'static str = "";

    /// Size of the head portion in the ABI encoding (always 32 for non-tuple types).
    const HEAD_SIZE: usize = 32;

    /// Size of this value's slot when it appears in an enclosing tuple.
    ///
    /// Static types are inlined into the tuple head, while dynamic types occupy
    /// a single 32-byte offset word.
    const SLOT_SIZE: usize = if Self::IS_DYNAMIC { 32 } else { Self::HEAD_SIZE };

    /// Whether this is a dynamic type (string, bytes, dynamic arrays).
    const IS_DYNAMIC: bool = false;

    /// ABI-encode this value, appending 32-byte word(s) to `buf`.
    fn abi_encode(&self, buf: &mut Vec<u8>);

    /// ABI-decode a value from `data` starting at byte `offset`.
    fn abi_decode(data: &[u8], offset: usize) -> Self;
}

/// ABI-encode a single function return value.
///
/// Solidity return data is encoded as the ABI tuple of all outputs. For a
/// single dynamic output, that means emitting a top-level offset word before
/// the value body.
pub fn encode_return_value<T: SolAbi>(value: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    if T::IS_DYNAMIC {
        let mut offset = [0u8; 32];
        offset[24..32].copy_from_slice(&(T::SLOT_SIZE as u64).to_be_bytes());
        buf.extend_from_slice(&offset);
    }
    value.abi_encode(&mut buf);
    buf
}

// -- bool --

impl SolAbi for bool {
    const SOL_NAME: &'static str = "bool";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = [0u8; 32];
        out[31] = if *self { 1 } else { 0 };
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        data[offset + 31] != 0
    }
}

// -- u8 --

impl SolAbi for u8 {
    const SOL_NAME: &'static str = "uint8";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = [0u8; 32];
        out[31] = *self;
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        data[offset + 31]
    }
}

// -- u16 --

impl SolAbi for u16 {
    const SOL_NAME: &'static str = "uint16";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = [0u8; 32];
        out[30..32].copy_from_slice(&self.to_be_bytes());
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        u16::from_be_bytes([data[offset + 30], data[offset + 31]])
    }
}

// -- u32 --

impl SolAbi for u32 {
    const SOL_NAME: &'static str = "uint32";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = [0u8; 32];
        out[28..32].copy_from_slice(&self.to_be_bytes());
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        u32::from_be_bytes(data[offset + 28..offset + 32].try_into().unwrap())
    }
}

// -- u64 --

impl SolAbi for u64 {
    const SOL_NAME: &'static str = "uint64";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = [0u8; 32];
        out[24..32].copy_from_slice(&self.to_be_bytes());
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        u64::from_be_bytes(data[offset + 24..offset + 32].try_into().unwrap())
    }
}

// -- u128 --

impl SolAbi for u128 {
    const SOL_NAME: &'static str = "uint128";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = [0u8; 32];
        out[16..32].copy_from_slice(&self.to_be_bytes());
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        u128::from_be_bytes(data[offset + 16..offset + 32].try_into().unwrap())
    }
}

// -- i8 --

impl SolAbi for i8 {
    const SOL_NAME: &'static str = "int8";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = [0u8; 32];
        if *self < 0 {
            out = [0xff; 32];
        }
        out[31] = *self as u8;
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        data[offset + 31] as i8
    }
}

// -- i16 --

impl SolAbi for i16 {
    const SOL_NAME: &'static str = "int16";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = if *self < 0 { [0xff; 32] } else { [0u8; 32] };
        out[30..32].copy_from_slice(&self.to_be_bytes());
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        i16::from_be_bytes([data[offset + 30], data[offset + 31]])
    }
}

// -- i32 --

impl SolAbi for i32 {
    const SOL_NAME: &'static str = "int32";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = if *self < 0 { [0xff; 32] } else { [0u8; 32] };
        out[28..32].copy_from_slice(&self.to_be_bytes());
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        i32::from_be_bytes(data[offset + 28..offset + 32].try_into().unwrap())
    }
}

// -- i64 --

impl SolAbi for i64 {
    const SOL_NAME: &'static str = "int64";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = if *self < 0 { [0xff; 32] } else { [0u8; 32] };
        out[24..32].copy_from_slice(&self.to_be_bytes());
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        i64::from_be_bytes(data[offset + 24..offset + 32].try_into().unwrap())
    }
}

// -- i128 --

impl SolAbi for i128 {
    const SOL_NAME: &'static str = "int128";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = if *self < 0 { [0xff; 32] } else { [0u8; 32] };
        out[16..32].copy_from_slice(&self.to_be_bytes());
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        i128::from_be_bytes(data[offset + 16..offset + 32].try_into().unwrap())
    }
}

// -- Address (ethereum_types::H160) --

impl SolAbi for crate::Address {
    const SOL_NAME: &'static str = "address";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let mut out = [0u8; 32];
        out[12..32].copy_from_slice(self.as_bytes());
        buf.extend_from_slice(&out);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&data[offset + 12..offset + 32]);
        crate::Address::from(addr)
    }
}

// -- U256 (alloy_primitives::U256) --

impl SolAbi for crate::U256 {
    const SOL_NAME: &'static str = "uint256";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_be_bytes::<32>());
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        crate::U256::from_be_slice(&data[offset..offset + 32])
    }
}

// -- I256 (alloy_primitives::I256) --

impl SolAbi for crate::I256 {
    const SOL_NAME: &'static str = "int256";

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_be_bytes::<32>());
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        crate::I256::try_from_be_slice(&data[offset..offset + 32]).unwrap()
    }
}

// -- Fixed-size byte arrays [u8; N] --

macro_rules! impl_sol_abi_fixed_bytes {
    ($($n:literal),*) => {
        $(
            impl SolAbi for [u8; $n] {
                const SOL_NAME: &'static str = concat!("bytes", stringify!($n));

                fn abi_encode(&self, buf: &mut Vec<u8>) {
                    let mut out = [0u8; 32];
                    out[..$n].copy_from_slice(self);
                    buf.extend_from_slice(&out);
                }

                fn abi_decode(data: &[u8], offset: usize) -> Self {
                    let mut bytes = [0u8; $n];
                    bytes.copy_from_slice(&data[offset..offset + $n]);
                    bytes
                }
            }
        )*
    };
}

impl_sol_abi_fixed_bytes!(1, 2, 4, 8, 16, 20, 32);

// -- String (dynamic) --

impl SolAbi for String {
    const SOL_NAME: &'static str = "string";
    const IS_DYNAMIC: bool = true;

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let s: &str = self.as_str();
        let len = s.len();
        let padded_len = (len + 31) / 32 * 32;

        // Encode length
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&(len as u64).to_be_bytes());
        buf.extend_from_slice(&len_bytes);

        // Encode data + padding
        buf.extend_from_slice(s.as_bytes());
        buf.resize(buf.len() + padded_len - len, 0);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        let dyn_offset =
            crate::U256::from_be_slice(&data[offset..offset + 32]).as_limbs()[0] as usize;
        let length =
            crate::U256::from_be_slice(&data[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
        let bytes = &data[dyn_offset + 32..dyn_offset + 32 + length];
        String::from_utf8_lossy(bytes).into_owned()
    }
}

// -- Vec<u8> (dynamic bytes) --

impl SolAbi for Vec<u8> {
    const SOL_NAME: &'static str = "bytes";
    const IS_DYNAMIC: bool = true;

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let len = self.len();
        let padded_len = (len + 31) / 32 * 32;

        // Encode length
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&(len as u64).to_be_bytes());
        buf.extend_from_slice(&len_bytes);

        // Encode data + padding
        buf.extend_from_slice(self);
        buf.resize(buf.len() + padded_len - len, 0);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        let dyn_offset =
            crate::U256::from_be_slice(&data[offset..offset + 32]).as_limbs()[0] as usize;
        let length =
            crate::U256::from_be_slice(&data[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
        data[dyn_offset + 32..dyn_offset + 32 + length].to_vec()
    }
}

// -- Option<T> wrapper (encoded as tuple (bool, T)) --
// Blanket impl covers all T: SolAbi, including user-defined #[derive(SolAbi)] types.
// Note: SOL_NAME/ABI_TYPE/ABI_COMPONENTS use placeholder values here because
// concatcp! cannot work with generic type parameters. The derive macro handles
// these const strings inline when Option<T> fields appear in derived structs.

impl<T: SolAbi> SolAbi for Option<T> {
    const SOL_NAME: &'static str = "(bool,?)";
    const ABI_TYPE: &'static str = "tuple";
    const ABI_COMPONENTS: &'static str = "";
    const HEAD_SIZE: usize = 32 + T::HEAD_SIZE;
    const IS_DYNAMIC: bool = T::IS_DYNAMIC;

    fn abi_encode(&self, buf: &mut alloc::vec::Vec<u8>) {
        if T::IS_DYNAMIC {
            match self {
                Some(val) => {
                    true.abi_encode(buf);
                    let mut offset_word = [0u8; 32];
                    offset_word[24..32].copy_from_slice(&64u64.to_be_bytes());
                    buf.extend_from_slice(&offset_word);
                    val.abi_encode(buf);
                }
                None => {
                    false.abi_encode(buf);
                    let mut offset_word = [0u8; 32];
                    offset_word[24..32].copy_from_slice(&64u64.to_be_bytes());
                    buf.extend_from_slice(&offset_word);
                    buf.extend_from_slice(&[0u8; 32]);
                }
            }
        } else {
            match self {
                Some(val) => {
                    true.abi_encode(buf);
                    val.abi_encode(buf);
                }
                None => {
                    false.abi_encode(buf);
                    buf.resize(buf.len() + T::HEAD_SIZE, 0);
                }
            }
        }
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        if T::IS_DYNAMIC {
            let dyn_offset =
                crate::U256::from_be_slice(&data[offset..offset + 32]).as_limbs()[0] as usize;
            let body = &data[dyn_offset..];
            let is_some = bool::abi_decode(body, 0);
            if !is_some {
                return None;
            }
            return Some(T::abi_decode(body, 32));
        }

        let is_some = bool::abi_decode(data, offset);
        if !is_some {
            return None;
        }
        Some(T::abi_decode(data, offset + 32))
    }
}

// -- Selector computation --

pub fn compute_selector(name: &str, param_type_names: &[&str]) -> [u8; 4] {
    use tiny_keccak::{Hasher, Keccak};

    let mut sig = String::new();
    sig.push_str(name);
    sig.push('(');
    for (i, pname) in param_type_names.iter().enumerate() {
        if i > 0 {
            sig.push(',');
        }
        sig.push_str(pname);
    }
    sig.push(')');

    let mut hasher = Keccak::v256();
    hasher.update(sig.as_bytes());
    let mut output = [0u8; 32];
    hasher.finalize(&mut output);
    [output[0], output[1], output[2], output[3]]
}

#[cfg(test)]
mod tests {
    use super::{SolAbi, encode_return_value};
    use crate as pvm_contract;

    #[derive(pvm_contract_macros::SolAbi, Debug, PartialEq, Eq)]
    struct StaticPair {
        left: u64,
        right: u64,
    }

    #[derive(pvm_contract_macros::SolAbi, Debug, PartialEq, Eq)]
    struct DynamicPair {
        id: u64,
        label: alloc::string::String,
    }

    #[test]
    fn encode_single_dynamic_return_prefixes_top_level_offset() {
        let encoded = encode_return_value(&alloc::string::String::from("hi"));
        let mut expected_offset = [0u8; 32];
        expected_offset[24..32].copy_from_slice(&32u64.to_be_bytes());
        assert_eq!(&encoded[..32], &expected_offset);
        assert_eq!(
            alloc::string::String::abi_decode(&encoded, 0),
            alloc::string::String::from("hi")
        );
    }

    #[test]
    fn dynamic_struct_decode_uses_top_level_offset() {
        let value = DynamicPair {
            id: 7,
            label: alloc::string::String::from("arena"),
        };
        let encoded = encode_return_value(&value);
        assert_eq!(DynamicPair::abi_decode(&encoded, 0), value);
    }

    #[test]
    fn dynamic_option_decode_uses_top_level_offset() {
        let value = Some(alloc::string::String::from("ghost"));
        let encoded = encode_return_value(&value);
        assert_eq!(
            Option::<alloc::string::String>::abi_decode(&encoded, 0),
            value
        );
    }

    #[test]
    fn static_types_keep_in_place_encoding() {
        let value = StaticPair { left: 1, right: 2 };
        let encoded = encode_return_value(&value);
        assert_eq!(encoded.len(), StaticPair::HEAD_SIZE);
        assert_eq!(StaticPair::abi_decode(&encoded, 0), value);
    }
}

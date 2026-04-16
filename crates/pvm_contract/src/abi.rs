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

    /// Whether this is a dynamic type (string, bytes, dynamic arrays).
    const IS_DYNAMIC: bool = false;

    /// ABI-encode this value, appending 32-byte word(s) to `buf`.
    fn abi_encode(&self, buf: &mut Vec<u8>);

    /// ABI-decode a value from `data` starting at byte `offset`.
    fn abi_decode(data: &[u8], offset: usize) -> Self;
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

// -- Bytes newtype (Solidity `bytes`) --
//
// Wraps `Vec<u8>` so the "bytes" encoding (one byte per byte, tightly packed
// with 32-byte padding) is distinct from `Vec<u8>`, which encodes as `uint8[]`
// via the generic `Vec<T>` impl below.

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Bytes(pub Vec<u8>);

impl From<Vec<u8>> for Bytes {
    fn from(v: Vec<u8>) -> Self { Bytes(v) }
}

impl From<Bytes> for Vec<u8> {
    fn from(b: Bytes) -> Self { b.0 }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] { &self.0 }
}

impl core::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] { &self.0 }
}

impl SolAbi for Bytes {
    const SOL_NAME: &'static str = "bytes";
    const IS_DYNAMIC: bool = true;

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        let len = self.0.len();
        let padded_len = (len + 31) / 32 * 32;

        // Encode length
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&(len as u64).to_be_bytes());
        buf.extend_from_slice(&len_bytes);

        // Encode data + padding
        buf.extend_from_slice(&self.0);
        buf.resize(buf.len() + padded_len - len, 0);
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        let dyn_offset =
            crate::U256::from_be_slice(&data[offset..offset + 32]).as_limbs()[0] as usize;
        let length =
            crate::U256::from_be_slice(&data[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
        Bytes(data[dyn_offset + 32..dyn_offset + 32 + length].to_vec())
    }
}

// -- Vec<T> (dynamic array, Solidity `T[]`) --
//
// Blanket impl covers all `T: SolAbi`, including user-defined `#[derive(SolAbi)]`
// types. SOL_NAME/ABI_TYPE/ABI_COMPONENTS use placeholder values here because
// `concatcp!` cannot reference generic type parameters (Rust E0401). The derive
// and contract macros inline these const strings with the concrete `T` wherever
// `Vec<T>` appears as a field, parameter, or return type.

impl<T: SolAbi> SolAbi for Vec<T> {
    const SOL_NAME: &'static str = "?[]";
    const ABI_TYPE: &'static str = "?[]";
    const ABI_COMPONENTS: &'static str = "";
    const HEAD_SIZE: usize = 32;
    const IS_DYNAMIC: bool = true;

    fn abi_encode(&self, buf: &mut Vec<u8>) {
        // Length prefix
        let mut len_bytes = [0u8; 32];
        len_bytes[24..32].copy_from_slice(&(self.len() as u64).to_be_bytes());
        buf.extend_from_slice(&len_bytes);

        if self.is_empty() {
            return;
        }

        if T::IS_DYNAMIC {
            // Elements tuple: head of n pointers + concatenated tails.
            // Offsets are relative to the start of the elements area (after the length word).
            let elems_start = buf.len();
            let head_len = self.len() * 32;
            buf.resize(elems_start + head_len, 0);
            let mut tail = Vec::new();
            for (i, elem) in self.iter().enumerate() {
                let off = (head_len + tail.len()) as u64;
                let hp = elems_start + i * 32;
                buf[hp + 24..hp + 32].copy_from_slice(&off.to_be_bytes());
                elem.abi_encode(&mut tail);
            }
            buf.extend_from_slice(&tail);
        } else {
            // Static elements encode in-place, each occupying T::HEAD_SIZE bytes.
            for elem in self.iter() {
                elem.abi_encode(buf);
            }
        }
    }

    fn abi_decode(data: &[u8], offset: usize) -> Self {
        // Vec<T> is always dynamic: the slot at `offset` holds a pointer to [len | elements].
        let dyn_offset =
            crate::U256::from_be_slice(&data[offset..offset + 32]).as_limbs()[0] as usize;
        let length =
            crate::U256::from_be_slice(&data[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
        let elems_start = dyn_offset + 32;

        let mut out = Vec::with_capacity(length);
        if T::IS_DYNAMIC {
            // Each element's head slot is at elems_start + i*32 and holds an offset
            // relative to elems_start pointing to the element's tail data.
            let sub = &data[elems_start..];
            for i in 0..length {
                out.push(T::abi_decode(sub, i * 32));
            }
        } else {
            let stride = T::HEAD_SIZE;
            for i in 0..length {
                out.push(T::abi_decode(data, elems_start + i * stride));
            }
        }
        out
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
        // When T is dynamic, Option is dynamic too and the slot at `offset` is a
        // pointer (relative to `data`'s start) to Option's own encoding. Shift
        // `data` to Option's start so the inner pointer (for dynamic T, written
        // relative to Option's start) resolves correctly.
        let sd = if T::IS_DYNAMIC {
            let base =
                crate::U256::from_be_slice(&data[offset..offset + 32]).as_limbs()[0] as usize;
            &data[base..]
        } else {
            &data[offset..]
        };
        let is_some = bool::abi_decode(sd, 0);
        if !is_some {
            return None;
        }
        Some(T::abi_decode(sd, 32))
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

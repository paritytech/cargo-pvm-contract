#![doc = include_str!("../../../specs/abi.md")]
#![no_std]

extern crate self as pvm_contract_types;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
mod alloc_types;

#[doc(hidden)]
pub use const_format;
use ruint::aliases::U256;

/// Fixed-size buffer for compile-time string concatenation.
///
/// Use [`ConstStr::new`] to concatenate two `&str` values in a `const`
/// context, then call [`ConstStr::as_str`] to obtain the resulting `&str`.
pub struct ConstStr {
    buf: [u8; 256],
    len: usize,
}

impl ConstStr {
    /// Concatenates `a` and `b` into a new [`ConstStr`].
    pub const fn new(a: &str, b: &str) -> Self {
        let a = a.as_bytes();
        let b = b.as_bytes();
        let len = a.len() + b.len();
        assert!(len <= 256, "concatenated string exceeds 256 bytes");

        let mut buf = [0u8; 256];
        let mut i = 0;
        while i < a.len() {
            buf[i] = a[i];
            i += 1;
        }
        let mut j = 0;
        while j < b.len() {
            buf[i + j] = b[j];
            j += 1;
        }
        Self { buf, len }
    }

    /// Appends `s` to this [`ConstStr`], returning a new [`ConstStr`].
    pub const fn append(self, s: &str) -> Self {
        let s = s.as_bytes();
        let new_len = self.len + s.len();
        assert!(new_len <= 256, "appended string exceeds 256 bytes");

        let mut buf = self.buf;
        let mut i = 0;
        while i < s.len() {
            buf[self.len + i] = s[i];
            i += 1;
        }
        Self { buf, len: new_len }
    }

    /// Appends the decimal representation of `n` to this [`ConstStr`].
    pub const fn append_usize(self, n: usize) -> Self {
        if n == 0 {
            return self.append("0");
        }
        let mut digits = [0u8; 20];
        let mut num_digits = 0;
        let mut val = n;
        while val > 0 {
            digits[num_digits] = b'0' + (val % 10) as u8;
            val /= 10;
            num_digits += 1;
        }
        let mut buf = self.buf;
        let mut new_len = self.len;
        let mut i = num_digits;
        while i > 0 {
            i -= 1;
            assert!(new_len < 256, "appended usize exceeds 256 bytes");
            buf[new_len] = digits[i];
            new_len += 1;
        }
        Self { buf, len: new_len }
    }

    /// Returns the concatenated string as a `&str`.
    pub const fn as_str(&self) -> &str {
        let (used, _) = self.buf.split_at(self.len);
        match core::str::from_utf8(used) {
            Ok(s) => s,
            Err(_) => panic!("invalid UTF-8"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address(pub [u8; 20]);

impl Address {
    pub const ZERO: Self = Self([0u8; 20]);
}

impl From<[u8; 20]> for Address {
    fn from(value: [u8; 20]) -> Self {
        Self(value)
    }
}

impl From<Address> for [u8; 20] {
    fn from(value: Address) -> Self {
        value.0
    }
}

impl AsRef<[u8; 20]> for Address {
    fn as_ref(&self) -> &[u8; 20] {
        &self.0
    }
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Marker trait for types that can be elements of `[T; N]` fixed arrays.
///
/// All types implementing `SolEncode` should also implement this trait,
/// **except `u8`**: bare `[u8; N]` arrays encode as Solidity `bytesN`
/// (a single left-aligned word), matching alloy's behavior. Use wrapper
/// types like `Address` or `#[derive(SolType)]` structs for other semantics.
pub trait SolArrayElement: SolEncode {}

/// Computes the 4-byte Solidity function selector at compile time.
pub const fn const_selector(sig: &str) -> [u8; 4] {
    let hash = keccak_const::Keccak256::new()
        .update(sig.as_bytes())
        .finalize();
    [hash[0], hash[1], hash[2], hash[3]]
}

/// Trait for encoding Rust types to Solidity ABI-encoded bytes.
pub trait SolEncode {
    const IS_DYNAMIC: bool;

    /// The canonical Solidity type name (e.g. "uint256", "address", "(uint64,uint64)").
    const SOL_NAME: &'static str;

    /// Size of the head portion in ABI encoding. Defaults to 32 (one ABI word).
    /// Overridden by structs to the sum of their field HEAD_SIZEs.
    const HEAD_SIZE: usize = 32;

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

    /// Offset-based decode helper used by generated code and custom decoders.
    fn decode_at(input: &[u8], offset: usize) -> Self;

    /// Tail decode helper used by dynamic container decoding.
    fn decode_tail(input: &[u8], offset: usize) -> Self {
        Self::decode_at(input, offset)
    }
}

macro_rules! impl_static_type {
    ($ty:ty, $sol_name:expr, $encode_fn:expr, $decode_fn:expr) => {
        impl SolEncode for $ty {
            const IS_DYNAMIC: bool = false;
            const SOL_NAME: &'static str = $sol_name;

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
    // Variant that also emits SolArrayElement
    ($ty:ty, $sol_name:expr, $encode_fn:expr, $decode_fn:expr, array_element) => {
        impl_static_type!($ty, $sol_name, $encode_fn, $decode_fn);
        impl SolArrayElement for $ty {}
    };
}

impl_static_type!(
    U256,
    "uint256",
    |val: &U256, buf: &mut [u8]| buf[..32].copy_from_slice(&val.to_be_bytes::<32>()),
    |input: &[u8], offset: usize| U256::from_be_slice(&input[offset..offset + 32]),
    array_element
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
    },
    array_element
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
    },
    array_element
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
    },
    array_element
);

impl_static_type!(
    u16,
    "uint16",
    |val: &u16, buf: &mut [u8]| {
        buf[..30].fill(0);
        buf[30..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| u16::from_be_bytes([input[offset + 30], input[offset + 31]]),
    array_element
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
    i128,
    "int128",
    |val: &i128, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..16].fill(fill);
        buf[16..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        let bytes: [u8; 16] = input[offset + 16..offset + 32].try_into().unwrap();
        i128::from_be_bytes(bytes)
    },
    array_element
);

impl_static_type!(
    i64,
    "int64",
    |val: &i64, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..24].fill(fill);
        buf[24..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        let bytes: [u8; 8] = input[offset + 24..offset + 32].try_into().unwrap();
        i64::from_be_bytes(bytes)
    },
    array_element
);

impl_static_type!(
    i32,
    "int32",
    |val: &i32, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..28].fill(fill);
        buf[28..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        let bytes: [u8; 4] = input[offset + 28..offset + 32].try_into().unwrap();
        i32::from_be_bytes(bytes)
    },
    array_element
);

impl_static_type!(
    i16,
    "int16",
    |val: &i16, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..30].fill(fill);
        buf[30..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| i16::from_be_bytes([input[offset + 30], input[offset + 31]]),
    array_element
);

impl_static_type!(
    i8,
    "int8",
    |val: &i8, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..31].fill(fill);
        buf[31] = *val as u8;
    },
    |input: &[u8], offset: usize| input[offset + 31] as i8,
    array_element
);

impl_static_type!(
    bool,
    "bool",
    |val: &bool, buf: &mut [u8]| {
        buf[..31].fill(0);
        buf[31] = if *val { 1 } else { 0 };
    },
    |input: &[u8], offset: usize| input[offset + 31] != 0,
    array_element
);

impl_static_type!(
    Address,
    "address",
    |val: &Address, buf: &mut [u8]| {
        buf[..12].fill(0);
        buf[12..32].copy_from_slice(&val.0);
    },
    |input: &[u8], offset: usize| {
        let mut result = [0u8; 20];
        result.copy_from_slice(&input[offset + 12..offset + 32]);
        Address(result)
    },
    array_element
);

impl SolEncode for &str {
    const IS_DYNAMIC: bool = true;
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

// ---------------------------------------------------------------------------
// [u8; N] impl — encodes as Solidity `bytesN` (left-aligned in one word),
// matching alloy's behavior. For `T[N]` array semantics, see the
// `SolArrayElement`-bounded blanket impl below.
// ---------------------------------------------------------------------------

impl<const N: usize> SolEncode for [u8; N] {
    const IS_DYNAMIC: bool = false;
    const SOL_NAME: &'static str = {
        struct H<const N: usize>;
        impl<const N: usize> H<N> {
            const V: ConstStr = ConstStr::new("bytes", "").append_usize(N);
        }
        H::<N>::V.as_str()
    };

    #[inline]
    fn encode_len(&self) -> usize {
        32
    }

    fn encode_to(&self, buf: &mut [u8]) {
        buf[..N].copy_from_slice(self);
        buf[N..32].fill(0);
    }
}

impl<const N: usize> StaticEncodedLen for [u8; N] {
    const ENCODED_SIZE: usize = 32;
}

impl<const N: usize> SolDecode for [u8; N] {
    fn decode_at(input: &[u8], offset: usize) -> Self {
        let mut result = [0u8; N];
        result.copy_from_slice(&input[offset..offset + N]);
        result
    }
}

impl<const N: usize> SolArrayElement for [u8; N] {}

// ---------------------------------------------------------------------------
// Blanket impl for fixed-size arrays [T; N] where T: SolArrayElement
// (excludes [u8; N] which has its own bytesN impl above)
// ---------------------------------------------------------------------------

impl<T: SolArrayElement, const N: usize> SolEncode for [T; N] {
    const IS_DYNAMIC: bool = T::IS_DYNAMIC;
    const SOL_NAME: &'static str = {
        struct H<T, const N: usize>(core::marker::PhantomData<T>);
        impl<T: SolEncode, const N: usize> H<T, N> {
            const V: ConstStr = ConstStr::new(T::SOL_NAME, "[").append_usize(N).append("]");
        }
        H::<T, N>::V.as_str()
    };
    const HEAD_SIZE: usize = if T::IS_DYNAMIC {
        32 * N
    } else {
        T::HEAD_SIZE * N
    };

    fn encode_len(&self) -> usize {
        if T::IS_DYNAMIC {
            N * 32 + self.iter().map(|e| e.tail_len()).sum::<usize>()
        } else {
            T::HEAD_SIZE * N
        }
    }

    fn encode_to(&self, buf: &mut [u8]) {
        if T::IS_DYNAMIC {
            let mut tail_offset = N * 32;
            for (i, elem) in self.iter().enumerate() {
                let ho = i * 32;
                buf[ho..ho + 24].fill(0);
                buf[ho + 24..ho + 32].copy_from_slice(&(tail_offset as u64).to_be_bytes());
                let tl = elem.tail_len();
                elem.encode_tail_to(&mut buf[tail_offset..tail_offset + tl]);
                tail_offset += tl;
            }
        } else {
            let mut offset = 0;
            for elem in self.iter() {
                elem.encode_to(&mut buf[offset..]);
                offset += T::HEAD_SIZE;
            }
        }
    }
}

impl<T: SolArrayElement + StaticEncodedLen, const N: usize> StaticEncodedLen for [T; N] {
    const ENCODED_SIZE: usize = T::ENCODED_SIZE * N;
}

impl<T: SolArrayElement + SolDecode, const N: usize> SolDecode for [T; N] {
    fn decode_at(input: &[u8], offset: usize) -> Self {
        core::array::from_fn(|i| {
            if T::IS_DYNAMIC {
                let ho = offset + i * 32;
                let field_offset =
                    u64::from_be_bytes(input[ho + 24..ho + 32].try_into().unwrap()) as usize;
                T::decode_tail(input, offset + field_offset)
            } else {
                T::decode_at(input, offset + i * T::HEAD_SIZE)
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Tuple impls for arities 1-12
// ---------------------------------------------------------------------------

macro_rules! impl_tuple_sol {
    // We need to build SOL_NAME via ConstStr chain. Since macro repetition
    // can't incrementally build a chain, we use a two-macro approach:
    // the outer macro generates the impl, the inner builds the name.
    (@sol_name $first:ident $(, $rest:ident)*) => {{
        struct H<$first $(, $rest)*>(core::marker::PhantomData<($first, $($rest,)*)>);
        impl<$first: SolEncode $(, $rest: SolEncode)*> H<$first $(, $rest)*> {
            const V: ConstStr = ConstStr::new("(", $first::SOL_NAME)
                $(.append(",").append($rest::SOL_NAME))*
                .append(")");
        }
        H::<$first $(, $rest)*>::V.as_str()
    }};

    ($(($idx:tt : $T:ident)),+) => {
        impl<$($T: SolEncode),+> SolEncode for ($($T,)+) {
            const IS_DYNAMIC: bool = false $(|| $T::IS_DYNAMIC)+;
            const SOL_NAME: &'static str = impl_tuple_sol!(@sol_name $($T),+);
            const HEAD_SIZE: usize = 0 $(+ if $T::IS_DYNAMIC { 32 } else { $T::HEAD_SIZE })+;

            fn encode_len(&self) -> usize {
                Self::HEAD_SIZE
                    $(+ if $T::IS_DYNAMIC { self.$idx.tail_len() } else { 0 })+
            }

            fn encode_to(&self, buf: &mut [u8]) {
                let mut __ho = 0usize;
                let mut __to = Self::HEAD_SIZE;
                $(
                    if $T::IS_DYNAMIC {
                        buf[__ho..__ho + 24].fill(0);
                        buf[__ho + 24..__ho + 32]
                            .copy_from_slice(&(__to as u64).to_be_bytes());
                        __ho += 32;
                        let __tl = self.$idx.tail_len();
                        self.$idx.encode_tail_to(&mut buf[__to..__to + __tl]);
                        __to += __tl;
                    } else {
                        self.$idx.encode_to(&mut buf[__ho..]);
                        __ho += $T::HEAD_SIZE;
                    }
                )+
            }
        }

        impl<$($T: SolEncode),+> SolArrayElement for ($($T,)+) {}

        impl<$($T: SolDecode),+> SolDecode for ($($T,)+) {
            fn decode_at(input: &[u8], offset: usize) -> Self {
                let mut __ho = offset;
                ($(
                    {
                        let __val = if $T::IS_DYNAMIC {
                            let __fo = u64::from_be_bytes(
                                input[__ho + 24..__ho + 32].try_into().unwrap(),
                            ) as usize;
                            __ho += 32;
                            $T::decode_tail(input, offset + __fo)
                        } else {
                            let __val = $T::decode_at(input, __ho);
                            __ho += $T::HEAD_SIZE;
                            __val
                        };
                        __val
                    },
                )+)
            }
        }
    };
}

impl_tuple_sol!((0: A));
impl_tuple_sol!((0: A), (1: B));
impl_tuple_sol!((0: A), (1: B), (2: C));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_), (8: I));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_), (8: I), (9: J));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_), (8: I), (9: J), (10: K));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_), (8: I), (9: J), (10: K), (11: L));

#[cfg(test)]
mod tests;

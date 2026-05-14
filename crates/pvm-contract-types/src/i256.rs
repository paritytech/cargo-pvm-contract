//! Signed 256-bit integer matching Solidity's `int256`.
//!
//! Newtype around [`ruint::aliases::U256`] storing values in two's-complement
//! representation. Arithmetic and bitwise ops that are bit-identical between
//! signed and unsigned interpretations forward to `U256`'s `wrapping_*`
//! methods. Sign-aware ops ([`Ord`](core::cmp::Ord), [`Shr`](core::ops::Shr),
//! [`Div`](core::ops::Div), [`Rem`](core::ops::Rem)) are implemented directly.
//!
//! Overflow-aware variants — `checked_add`, `checked_sub`, `checked_mul`
//! (plus their `overflowing_*` counterparts) — are provided for safe-math
//! contracts that need revert-on-overflow semantics.
//!
//! Wire format is 32 big-endian bytes, identical to Solidity's `int256`
//! ABI encoding.

use core::cmp::Ordering;
use core::fmt;
use core::ops::{
    Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Div, DivAssign,
    Mul, MulAssign, Neg, Not, Rem, RemAssign, Shl, ShlAssign, Shr, ShrAssign, Sub, SubAssign,
};
use core::str::FromStr;

use ruint::aliases::U256;

/// Signed 256-bit integer in two's-complement representation.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct I256(U256);

impl I256 {
    /// `0`.
    pub const ZERO: Self = Self(U256::ZERO);
    /// `1`.
    pub const ONE: Self = Self(U256::ONE);
    /// `-1` — all bits set.
    pub const MINUS_ONE: Self = Self(U256::from_limbs([u64::MAX; 4]));
    /// Most-negative value: `-2^255` (only the sign bit set).
    pub const MIN: Self = Self(U256::from_limbs([0, 0, 0, 1u64 << 63]));
    /// Most-positive value: `2^255 - 1` (all bits except the sign bit).
    pub const MAX: Self = Self(U256::from_limbs([
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX >> 1,
    ]));

    const SIGN_BIT_LIMB: u64 = 1u64 << 63;

    /// Wrap a `U256` whose bits are the two's-complement representation of
    /// the signed value.
    #[inline]
    pub const fn from_raw(value: U256) -> Self {
        Self(value)
    }

    /// Inner two's-complement bits as a `U256`.
    #[inline]
    pub const fn into_raw(self) -> U256 {
        self.0
    }

    /// `true` if the value is strictly negative.
    #[inline]
    pub fn is_negative(&self) -> bool {
        self.0.as_limbs()[3] & Self::SIGN_BIT_LIMB != 0
    }

    /// `true` if the value is strictly positive.
    #[inline]
    pub fn is_positive(&self) -> bool {
        !self.is_negative() && self.0 != U256::ZERO
    }

    /// Wrapping absolute value. `I256::MIN.abs() == I256::MIN`.
    #[inline]
    pub fn abs(self) -> Self {
        if self.is_negative() { -self } else { self }
    }

    /// Absolute value as an unsigned `U256`. `I256::MIN.unsigned_abs() == 1 << 255`.
    #[inline]
    pub fn unsigned_abs(self) -> U256 {
        if self.is_negative() {
            self.0.wrapping_neg()
        } else {
            self.0
        }
    }

    /// Construct from a big-endian byte slice. Slice must be exactly 32 bytes.
    #[inline]
    pub fn from_be_slice(bytes: &[u8]) -> Self {
        Self(U256::from_be_slice(bytes))
    }

    /// 32-byte big-endian representation matching Solidity's `int256` wire format.
    #[inline]
    pub fn to_be_bytes(self) -> [u8; 32] {
        self.0.to_be_bytes::<32>()
    }

    // -----------------------------------------------------------------
    // Overflow-aware arithmetic.
    //
    // Each operation is exposed in two forms:
    // - `overflowing_*` returns `(result, did_overflow)` with the result
    //   wrapped on overflow (matching Rust's `iN::overflowing_*` shape).
    // - `checked_*` returns `None` on overflow (the standard safe-math
    //   primitive Solidity contracts rely on for revert-on-overflow).
    // -----------------------------------------------------------------

    /// Overflowing two's-complement addition.
    /// Overflow happens iff both operands share a sign and the result has
    /// the opposite sign.
    #[inline]
    pub fn overflowing_add(self, rhs: Self) -> (Self, bool) {
        let result = Self(self.0.wrapping_add(rhs.0));
        let overflow =
            self.is_negative() == rhs.is_negative() && result.is_negative() != self.is_negative();
        (result, overflow)
    }

    /// Checked two's-complement addition. Returns `None` on overflow.
    #[inline]
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        let (result, overflow) = self.overflowing_add(rhs);
        if overflow { None } else { Some(result) }
    }

    /// Overflowing two's-complement subtraction.
    /// Overflow happens iff the operands have different signs and the
    /// result's sign matches `rhs`'s sign.
    #[inline]
    pub fn overflowing_sub(self, rhs: Self) -> (Self, bool) {
        let result = Self(self.0.wrapping_sub(rhs.0));
        let overflow =
            self.is_negative() != rhs.is_negative() && result.is_negative() == rhs.is_negative();
        (result, overflow)
    }

    /// Checked two's-complement subtraction. Returns `None` on overflow.
    #[inline]
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        let (result, overflow) = self.overflowing_sub(rhs);
        if overflow { None } else { Some(result) }
    }

    /// Overflowing two's-complement multiplication.
    /// Returns `(wrapped_result, overflow)`. Overflow happens when:
    /// - the unsigned multiplication of absolute values overflows `U256`, or
    /// - the unsigned absolute result doesn't fit in `I256` with the
    ///   intended sign (positive: must be `<= I256::MAX`; negative: must
    ///   be `<= 2^255` so it fits as `I256::MIN`).
    #[inline]
    pub fn overflowing_mul(self, rhs: Self) -> (Self, bool) {
        if self.0 == U256::ZERO || rhs.0 == U256::ZERO {
            return (Self::ZERO, false);
        }
        let result_negative = self.is_negative() ^ rhs.is_negative();
        let (abs, abs_overflow) = self.unsigned_abs().overflowing_mul(rhs.unsigned_abs());
        // Build the signed result and detect range overflow.
        // Positive: abs must fit in [0, 2^255 - 1].
        // Negative: abs must fit in [0, 2^255]. (abs == 2^255 is exactly I256::MIN.)
        let max_abs_pos = Self::MAX.0;
        let signed = if result_negative {
            Self(abs.wrapping_neg())
        } else {
            Self(abs)
        };
        let range_overflow = if result_negative {
            abs > Self::MIN.unsigned_abs()
        } else {
            abs > max_abs_pos
        };
        (signed, abs_overflow || range_overflow)
    }

    /// Checked two's-complement multiplication. Returns `None` on overflow.
    #[inline]
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        let (result, overflow) = self.overflowing_mul(rhs);
        if overflow { None } else { Some(result) }
    }
}

// ---------------------------------------------------------------------------
// From<iN> — sign-extend native signed ints into 256 bits.
// ---------------------------------------------------------------------------

macro_rules! impl_from_signed {
    ($($t:ty),+) => {
        $(
            impl From<$t> for I256 {
                #[inline]
                fn from(value: $t) -> Self {
                    let mut buf = [if value < 0 { 0xffu8 } else { 0u8 }; 32];
                    let bytes = value.to_be_bytes();
                    let off = 32 - bytes.len();
                    buf[off..].copy_from_slice(&bytes);
                    Self::from_be_slice(&buf)
                }
            }
        )+
    };
}

impl_from_signed!(i8, i16, i32, i64, i128, isize);

impl From<I256> for U256 {
    #[inline]
    fn from(value: I256) -> Self {
        value.0
    }
}

// ---------------------------------------------------------------------------
// Bit-identical ops — forward to U256 wrapping arithmetic.
// ---------------------------------------------------------------------------

impl Add for I256 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0.wrapping_add(rhs.0))
    }
}

impl Sub for I256 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0.wrapping_sub(rhs.0))
    }
}

impl Mul for I256 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Self(self.0.wrapping_mul(rhs.0))
    }
}

impl Neg for I256 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self(self.0.wrapping_neg())
    }
}

impl Not for I256 {
    type Output = Self;
    #[inline]
    fn not(self) -> Self {
        Self(!self.0)
    }
}

impl BitAnd for I256 {
    type Output = Self;
    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitOr for I256 {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitXor for I256 {
    type Output = Self;
    #[inline]
    fn bitxor(self, rhs: Self) -> Self {
        Self(self.0 ^ rhs.0)
    }
}

impl Shl<usize> for I256 {
    type Output = Self;
    #[inline]
    fn shl(self, rhs: usize) -> Self {
        Self(self.0.wrapping_shl(rhs))
    }
}

// ---------------------------------------------------------------------------
// Sign-aware ops.
// ---------------------------------------------------------------------------

impl Shr<usize> for I256 {
    type Output = Self;

    /// Arithmetic right shift: sign-extends.
    #[inline]
    fn shr(self, rhs: usize) -> Self {
        let logical = self.0.wrapping_shr(rhs);
        if self.is_negative() && rhs > 0 {
            // OR in the high `rhs` bits so the result is sign-extended.
            // For shifts >= 256 the sign-extended result is all-ones (-1)
            // for negative inputs.
            if rhs >= 256 {
                return Self::MINUS_ONE;
            }
            let mask = (!U256::ZERO).wrapping_shl(256 - rhs);
            Self(logical | mask)
        } else {
            Self(logical)
        }
    }
}

impl Div for I256 {
    type Output = Self;

    /// Two's-complement division, truncating toward zero.
    /// Panics if `rhs == 0`. `I256::MIN / -1 == I256::MIN` (wrapping).
    fn div(self, rhs: Self) -> Self {
        assert!(rhs.0 != U256::ZERO, "I256: divide by zero");
        // MIN / -1 overflows in two's complement; wrap to MIN to match
        // Rust's signed wrapping_div behavior.
        if self == Self::MIN && rhs == Self::MINUS_ONE {
            return Self::MIN;
        }
        let neg = self.is_negative() ^ rhs.is_negative();
        let abs = self.unsigned_abs().wrapping_div(rhs.unsigned_abs());
        if neg {
            Self(abs.wrapping_neg())
        } else {
            Self(abs)
        }
    }
}

impl Rem for I256 {
    type Output = Self;

    /// Two's-complement remainder. Sign matches the dividend.
    /// Panics if `rhs == 0`.
    fn rem(self, rhs: Self) -> Self {
        assert!(rhs.0 != U256::ZERO, "I256: remainder by zero");
        if self == Self::MIN && rhs == Self::MINUS_ONE {
            return Self::ZERO;
        }
        let abs = self.unsigned_abs().wrapping_rem(rhs.unsigned_abs());
        if self.is_negative() {
            Self(abs.wrapping_neg())
        } else {
            Self(abs)
        }
    }
}

impl Ord for I256 {
    /// Two's-complement signed comparison: flip the sign bit on both sides
    /// and compare as unsigned. That maps `MIN..=-1` to `0..=2^255-1` and
    /// `0..=MAX` to `2^255..=2^256-1`, preserving signed order.
    fn cmp(&self, other: &Self) -> Ordering {
        let sign = U256::from_limbs([0, 0, 0, Self::SIGN_BIT_LIMB]);
        (self.0 ^ sign).cmp(&(other.0 ^ sign))
    }
}

impl PartialOrd for I256 {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Assign-op forwards (compose with the by-value impls above).
// ---------------------------------------------------------------------------

macro_rules! impl_assign {
    ($trait:ident, $method:ident, $op:tt) => {
        impl $trait for I256 {
            #[inline]
            fn $method(&mut self, rhs: Self) {
                *self = *self $op rhs;
            }
        }
    };
}

impl_assign!(AddAssign, add_assign, +);
impl_assign!(SubAssign, sub_assign, -);
impl_assign!(MulAssign, mul_assign, *);
impl_assign!(DivAssign, div_assign, /);
impl_assign!(RemAssign, rem_assign, %);
impl_assign!(BitAndAssign, bitand_assign, &);
impl_assign!(BitOrAssign, bitor_assign, |);
impl_assign!(BitXorAssign, bitxor_assign, ^);

impl ShlAssign<usize> for I256 {
    #[inline]
    fn shl_assign(&mut self, rhs: usize) {
        *self = *self << rhs;
    }
}

impl ShrAssign<usize> for I256 {
    #[inline]
    fn shr_assign(&mut self, rhs: usize) {
        *self = *self >> rhs;
    }
}

// ---------------------------------------------------------------------------
// Display / Debug / FromStr.
// ---------------------------------------------------------------------------

impl fmt::Display for I256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_negative() {
            f.write_str("-")?;
            fmt::Display::fmt(&self.unsigned_abs(), f)
        } else {
            fmt::Display::fmt(&self.0, f)
        }
    }
}

impl fmt::Debug for I256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Error returned by [`I256::from_str`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseI256Error(ruint::ParseError);

impl fmt::Display for ParseI256Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for I256 {
    type Err = ParseI256Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (negative, rest) = match s.as_bytes().first() {
            Some(b'-') => (true, &s[1..]),
            Some(b'+') => (false, &s[1..]),
            _ => (false, s),
        };
        let value = U256::from_str(rest).map_err(ParseI256Error)?;
        if negative {
            Ok(Self(value.wrapping_neg()))
        } else {
            Ok(Self(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_have_correct_bit_patterns() {
        // ZERO: all zero.
        assert_eq!(I256::ZERO.to_be_bytes(), [0u8; 32]);

        // ONE: last byte is 1, rest zero.
        let mut expected = [0u8; 32];
        expected[31] = 1;
        assert_eq!(I256::ONE.to_be_bytes(), expected);

        // MINUS_ONE: all 0xff (two's complement -1).
        assert_eq!(I256::MINUS_ONE.to_be_bytes(), [0xff; 32]);

        // MIN: -2^255 = only the sign bit set = 0x80 followed by 31 zero bytes.
        let mut expected = [0u8; 32];
        expected[0] = 0x80;
        assert_eq!(I256::MIN.to_be_bytes(), expected);

        // MAX: 2^255 - 1 = 0x7f followed by 31 0xff bytes.
        let mut expected = [0xff; 32];
        expected[0] = 0x7f;
        assert_eq!(I256::MAX.to_be_bytes(), expected);

        // Relationships between constants.
        assert_eq!(I256::MAX + I256::ONE, I256::MIN); // wrapping overflow
        assert_eq!(I256::MIN - I256::ONE, I256::MAX); // wrapping underflow
        assert_eq!(I256::MINUS_ONE + I256::ONE, I256::ZERO);
    }

    #[test]
    fn overflowing_returns_correct_wrapped_value_and_flag() {
        // add: MAX + 1 wraps to MIN.
        assert_eq!(I256::MAX.overflowing_add(I256::ONE), (I256::MIN, true));
        // add: no overflow case.
        assert_eq!(
            I256::ONE.overflowing_add(I256::ONE),
            (I256::from(2i32), false)
        );
        // add: MIN + (-1) wraps to MAX.
        assert_eq!(
            I256::MIN.overflowing_add(I256::MINUS_ONE),
            (I256::MAX, true)
        );

        // sub: MIN - 1 wraps to MAX.
        assert_eq!(I256::MIN.overflowing_sub(I256::ONE), (I256::MAX, true));
        // sub: no overflow case.
        assert_eq!(
            I256::ZERO.overflowing_sub(I256::ONE),
            (I256::MINUS_ONE, false)
        );

        // mul: MAX * 2 overflows.
        let (result, overflow) = I256::MAX.overflowing_mul(I256::from(2i32));
        assert!(overflow);
        assert_eq!(result, I256::from(-2i32)); // MAX*2 = 2^256-2, wraps to -2

        // mul: no overflow case.
        assert_eq!(
            I256::from(3i32).overflowing_mul(I256::from(4i32)),
            (I256::from(12i32), false)
        );
    }

    #[test]
    fn signed_ordering_across_boundaries() {
        // Basic ordering.
        assert!(I256::MIN < I256::MAX);
        assert!(I256::MIN < I256::ZERO);
        assert!(I256::MINUS_ONE < I256::ZERO);
        assert!(I256::ZERO < I256::ONE);
        assert!(I256::ZERO < I256::MAX);

        // MIN is the smallest; MAX is the largest.
        assert!(I256::MIN < I256::MINUS_ONE);
        assert!(I256::ONE < I256::MAX);

        // Adjacent to sign boundary.
        assert!(I256::MINUS_ONE < I256::ZERO);
        assert!(I256::ZERO < I256::ONE);

        // Self-equality.
        assert_eq!(I256::MIN.cmp(&I256::MIN), core::cmp::Ordering::Equal);
        assert_eq!(I256::MAX.cmp(&I256::MAX), core::cmp::Ordering::Equal);
    }

    #[test]
    fn shifts_at_extreme_counts() {
        // Shift by 0: identity.
        assert_eq!(I256::MAX << 0, I256::MAX);
        assert_eq!(I256::MAX >> 0, I256::MAX);
        assert_eq!(I256::MIN << 0, I256::MIN);
        assert_eq!(I256::MIN >> 0, I256::MIN);

        // Shift by 255: only the sign bit remains (or is shifted out).
        assert_eq!(I256::ONE << 255, I256::MIN); // 1 << 255 = -2^255 (sign bit)
        assert_eq!(I256::MIN >> 255, I256::MINUS_ONE); // arithmetic: sign-extend

        // Shift by 256: full width → 0 for shl; -1 for negative asr, 0 for positive.
        assert_eq!(I256::ONE << 256, I256::ZERO);
        assert_eq!(I256::MAX << 256, I256::ZERO);
        assert_eq!(I256::MIN >> 256, I256::MINUS_ONE); // all-ones from sign extension
        assert_eq!(I256::MAX >> 256, I256::ZERO); // positive: all zeros

        // MINUS_ONE >> any positive shift = MINUS_ONE (arithmetic shift fills with 1s).
        assert_eq!(I256::MINUS_ONE >> 1, I256::MINUS_ONE);
        assert_eq!(I256::MINUS_ONE >> 128, I256::MINUS_ONE);
        assert_eq!(I256::MINUS_ONE >> 255, I256::MINUS_ONE);
    }

    #[test]
    fn from_signed_native_ints_sign_extends() {
        // Negative i8 must sign-extend to all-1s in the high bytes.
        let v = I256::from(-1i8);
        assert_eq!(v.to_be_bytes(), [0xff; 32]);
        // Positive value must not.
        let v = I256::from(1i8);
        let mut expected = [0u8; 32];
        expected[31] = 1;
        assert_eq!(v.to_be_bytes(), expected);
    }

    #[test]
    fn unsigned_abs_handles_min() {
        // |I256::MIN| = 2^255, which doesn't fit in I256 but fits in U256.
        let abs = I256::MIN.unsigned_abs();
        assert_eq!(abs, U256::from_limbs([0, 0, 0, 1u64 << 63]));
    }

    #[test]
    fn min_div_minus_one_wraps_to_min() {
        // Edge case: -2^255 / -1 overflows in two's complement; we wrap to MIN.
        assert_eq!(I256::MIN / I256::MINUS_ONE, I256::MIN);
        assert_eq!(I256::MIN % I256::MINUS_ONE, I256::ZERO);
    }

    #[test]
    fn checked_arithmetic_overflow_edge_cases() {
        // Add overflow: MAX + 1 = MIN (wrapping); checked returns None.
        assert_eq!(I256::MAX.checked_add(I256::ONE), None);
        assert_eq!(I256::MIN.checked_add(I256::MINUS_ONE), None);
        assert_eq!(I256::MAX.checked_add(I256::ZERO), Some(I256::MAX));
        assert_eq!(I256::MIN.checked_add(I256::ZERO), Some(I256::MIN));
        // Crossing zero is fine.
        assert_eq!(I256::ONE.checked_add(I256::MINUS_ONE), Some(I256::ZERO));

        // Sub overflow: MIN - 1 underflows; MAX - (-1) overflows.
        assert_eq!(I256::MIN.checked_sub(I256::ONE), None);
        assert_eq!(I256::MAX.checked_sub(I256::MINUS_ONE), None);
        assert_eq!(I256::ZERO.checked_sub(I256::MIN), None); // -MIN doesn't fit
        assert_eq!(I256::ZERO.checked_sub(I256::ONE), Some(I256::MINUS_ONE));

        // Mul overflow: MIN * -1 doesn't fit (would be 2^255).
        assert_eq!(I256::MIN.checked_mul(I256::MINUS_ONE), None);
        assert_eq!(I256::MAX.checked_mul(I256::from(2i32)), None);
        assert_eq!(I256::MIN.checked_mul(I256::from(2i32)), None);
        // Identity / zero cases.
        assert_eq!(I256::MAX.checked_mul(I256::ONE), Some(I256::MAX));
        assert_eq!(I256::MIN.checked_mul(I256::ONE), Some(I256::MIN));
        assert_eq!(I256::MAX.checked_mul(I256::ZERO), Some(I256::ZERO));
        assert_eq!(I256::MIN.checked_mul(I256::ZERO), Some(I256::ZERO));
    }

    // -----------------------------------------------------------------
    // Arithmetic vs native `i128`: any pair of `i128` values fits in
    // `I256`, and Rust's `i128` arithmetic gives us a ground-truth
    // two's-complement oracle without any external dependency.
    // -----------------------------------------------------------------

    /// Small-but-diverse set of `i128` values hitting sign transitions,
    /// boundaries, and a couple of generic patterns.
    const I128_SAMPLES: &[i128] = &[
        0,
        1,
        -1,
        2,
        -2,
        42,
        -42,
        i128::MAX,
        i128::MIN,
        i128::MAX - 1,
        i128::MIN + 1,
        i64::MAX as i128,
        i64::MIN as i128,
        (i64::MAX as i128) + 1,
        (i64::MIN as i128) - 1,
    ];

    #[test]
    fn arithmetic_matches_i128_oracle() {
        // Key insight: i128 wraps at 2^127 but I256 wraps at 2^255. When an
        // i128 op overflows, the "true" mathematical result still fits in
        // I256, so the wrapped i128 value isn't a valid oracle. Use
        // `checked_*` and only assert when i128 produces a non-overflowed
        // answer — that's exactly the range where i128 is a ground truth.
        for &a in I128_SAMPLES {
            for &b in I128_SAMPLES {
                let ai = I256::from(a);
                let bi = I256::from(b);

                if let Some(sum) = a.checked_add(b) {
                    assert_eq!(ai + bi, I256::from(sum), "add {a} + {b}");
                }
                if let Some(diff) = a.checked_sub(b) {
                    assert_eq!(ai - bi, I256::from(diff), "sub {a} - {b}");
                }

                // cmp is total and lossless under sign-extension — always safe.
                assert_eq!(ai.cmp(&bi), a.cmp(&b), "cmp {a} vs {b}");

                // Bitwise ops on sign-extended i128s must match sign-extended
                // bitwise results — these never overflow.
                assert_eq!(ai & bi, I256::from(a & b), "and {a} & {b}");
                assert_eq!(ai | bi, I256::from(a | b), "or {a} | {b}");
                assert_eq!(ai ^ bi, I256::from(a ^ b), "xor {a} ^ {b}");

                // div / rem: skip b == 0, and skip i128::MIN / -1 which
                // overflows i128 but is well-defined in I256.
                if b != 0
                    && let Some(q) = a.checked_div(b)
                {
                    assert_eq!(ai / bi, I256::from(q), "div {a} / {b}");
                    let r = a.checked_rem(b).expect("rem defined when div defined");
                    assert_eq!(ai % bi, I256::from(r), "rem {a} % {b}");
                }
            }

            let ai = I256::from(a);
            // Unary neg: skip i128::MIN which overflows in i128 but wraps to
            // itself in both types (covered in `negation_wraps_only_at_min`).
            if let Some(neg) = a.checked_neg() {
                assert_eq!(-ai, I256::from(neg), "neg {a}");
            }
            // not: bit inversion never overflows.
            assert_eq!(!ai, I256::from(!a), "not {a}");
        }
    }

    #[test]
    fn shifts_match_i128_oracle() {
        // Shifts at arbitrary positions: compare only when the native i128
        // result captures the full answer (i.e. no bits are lost to
        // overflow). For shifts where the answer doesn't fit in i128,
        // `checked_shl` returns `None` and we fall back to algebraic
        // identity testing elsewhere.
        for &a in I128_SAMPLES {
            let ai = I256::from(a);
            for shift in [0u32, 1, 7, 31, 63, 100, 126] {
                if let Some(shifted) = a.checked_shl(shift) {
                    // Further: the shifted result must round-trip through
                    // i128 without losing information (no high bits set
                    // outside the sign-extended i128 range). `checked_shl`
                    // only checks `shift < bits`; it doesn't check overflow,
                    // so verify explicitly.
                    if a == 0 || (shifted >> shift) == a {
                        assert_eq!(
                            ai << (shift as usize),
                            I256::from(shifted),
                            "shl {a} << {shift}"
                        );
                    }
                }
                // Arithmetic right shift: i128's `>>` is arithmetic and never
                // overflows for shift < 128, so it's always a valid oracle.
                if shift < 128 {
                    assert_eq!(
                        ai >> (shift as usize),
                        I256::from(a >> shift),
                        "shr {a} >> {shift}"
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // Full-256-bit algebraic identities — no external oracle needed.
    // These verify our implementation is internally consistent across
    // the entire value space, not just the i128 subset.
    // -----------------------------------------------------------------

    #[test]
    fn algebraic_identities_hold_across_full_range() {
        // Fixed stack array so this test stays no_std-clean.
        // Values include the WIDE_SAMPLES plus a few that exercise the high
        // 128 bits i128 can't represent.
        let all: [I256; 9] = [
            I256::ZERO,
            I256::ONE,
            I256::MINUS_ONE,
            I256::MIN,
            I256::MAX,
            I256(U256::from_limbs([1, 1, 1, 1])),
            I256(U256::from_limbs([u64::MAX, 0, 0, 0])).wrapping_shl_by(128),
            I256::from(i128::MAX) + I256::ONE, // just past the i128 boundary
            -(I256::from(i128::MIN) - I256::ONE),
        ];

        for &a in &all {
            // a + (-a) == 0 for all a except MIN (where -MIN wraps to MIN).
            if a != I256::MIN {
                assert_eq!(a + (-a), I256::ZERO, "a + (-a) for {a}");
            }
            // a - a == 0 always.
            assert_eq!(a - a, I256::ZERO, "a - a for {a}");
            // --a == a (double-negation wraps through MIN too).
            assert_eq!(-(-a), a, "--a for {a}");
            // !!a == a.
            assert_eq!(!!a, a, "!!a for {a}");
            // a * 1 == a; a * 0 == 0.
            assert_eq!(a * I256::ONE, a, "a * 1 for {a}");
            assert_eq!(a * I256::ZERO, I256::ZERO, "a * 0 for {a}");
            // a | 0 == a; a & !0 == a; a ^ a == 0.
            assert_eq!(a | I256::ZERO, a, "a | 0 for {a}");
            assert_eq!(a & I256::MINUS_ONE, a, "a & -1 for {a}");
            assert_eq!(a ^ a, I256::ZERO, "a ^ a for {a}");
            // Nonzero `a / a == 1`.
            if a != I256::ZERO {
                assert_eq!(a / a, I256::ONE, "a / a for {a}");
            }
        }

        for &a in &all {
            for &b in &all {
                // Commutativity of + and *.
                assert_eq!(a + b, b + a, "add-commutes {a},{b}");
                assert_eq!(a * b, b * a, "mul-commutes {a},{b}");
                // (a + b) - b == a (wraps consistently).
                assert_eq!((a + b) - b, a, "add-sub inverse {a},{b}");
                // Comparison reflects i256-MIN < 0.
                if I256::is_negative(&a) && !I256::is_negative(&b) {
                    assert!(a < b, "negative {a} < non-negative {b}");
                }
            }
        }
    }

    #[test]
    fn encode_decode_round_trips_for_boundaries() {
        use crate::{SolDecode, SolEncode};
        for val in [
            I256::ZERO,
            I256::ONE,
            I256::MINUS_ONE,
            I256::MIN,
            I256::MAX,
            I256::from(-2i64),
            I256::from(i128::MIN),
            I256::from(i128::MAX),
        ] {
            let mut buf = [0u8; 32];
            val.encode_to(&mut buf);
            assert_eq!(
                I256::decode(&buf).unwrap(),
                val,
                "round-trip failed for {val}"
            );
        }
    }

    #[test]
    fn from_str_parses_signed_decimal() {
        // FromStr doesn't need alloc; Display (which does) is covered in
        // tests.rs where the alloc feature is already enabled.
        assert_eq!("0".parse::<I256>().unwrap(), I256::ZERO);
        assert_eq!("-1".parse::<I256>().unwrap(), I256::MINUS_ONE);
        assert_eq!("+5".parse::<I256>().unwrap(), I256::from(5i32));
        assert_eq!("-42".parse::<I256>().unwrap(), I256::from(-42i32));
    }

    #[test]
    fn negation_wraps_only_at_min() {
        // Two's-complement identity: -MIN wraps back to MIN.
        assert_eq!(-I256::MIN, I256::MIN);
        // Every other MIN-adjacent value negates correctly.
        assert_eq!(-(I256::MIN + I256::ONE), I256::MAX);
        assert_eq!(-I256::MAX, I256::MIN + I256::ONE);
    }

    // Test-only helper used by the algebraic identity tests.
    impl I256 {
        fn wrapping_shl_by(self, rhs: usize) -> Self {
            Self(self.0.wrapping_shl(rhs))
        }
    }
}

### Description 
part of #65, namely Introduction of `DecodeError`

### Changes 
- Introduce `pvm-contract-types` `DecodeError`
  - Single struct with no variants.
  - not an enum because 99% of the time users never look at the actual error and proper errors increase binary size.
  - implements `SolError` that returns `InvalidCalldata()` error
- changes to `SolDecode` trait: 
  - now is failable, all methods return a `Result<Self, DecodeError>`
```rust
  /// Trait for decoding Solidity ABI-encoded bytes into Rust types.
pub trait SolDecode: SolEncode + Sized {
    /// Decode from top-level ABI encoding produced by [`SolEncode::encode_to`].
    /// Symmetric with `encode_to`:
    /// - Tuples (IS_TUPLE=true): decode body directly
    /// - Dynamic non-tuples: read offset pointer at position 0, decode body at offset
    /// - Static non-tuples: decode body directly
    fn decode(input: &[u8]) -> Result<Self, DecodeError> {
        if Self::IS_TUPLE || !Self::IS_DYNAMIC {
            Self::decode_at(input, 0)
        } else {
            // Dynamic non-tuple: encode_to wrote [offset=32][body]
            // Read offset, then decode the body at that position
            let offset = input
                .get(24..32)
                .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
                .ok_or(DecodeError)
                .map(u64::from_be_bytes)? as usize;
            Self::decode_tail(input, offset)
        }
    }

    /// Offset-based decode helper used by generated code and custom decoders.
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError>;

    /// Tail decode helper used by dynamic container decoding.
    #[inline(always)]
    fn decode_tail(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        Self::decode_at(input, offset)
    }
}
```
- introduced `StaticDecode` as per suggestion in #65 
  - implemented  and derived for all `T::IS_STATIC` types.
  ```rust
  pub trait StaticDecode: SolDecode + SolEncode + StaticEncodedLen + Sized {
    /// SAFETY contract: caller guarantees `input.len() >= offset + ENCODED_SIZE`.
    /// Caller is the dispatch codegen that checks total size once at entry.
    fn decode_unchecked(input: &[u8], offset: usize) -> Self;
  }
  ```
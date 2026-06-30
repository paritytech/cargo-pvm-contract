use crate::{DecodeError, SolDecode, SolEncode};

/// Wrapper for raw byte data that encodes as Solidity `bytes` (packed encoding).
///
/// Use `Bytes` instead of `Vec<u8>` when the Solidity signature uses `bytes`.
/// `Vec<u8>` encodes as `uint8[]` (array of 32-byte-padded elements), while
/// `Bytes` encodes as `bytes` (length-prefixed packed data), matching alloy's
/// distinction between `Bytes` and `Vec<u8>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bytes(pub alloc::vec::Vec<u8>);

impl SolEncode for Bytes {
    const IS_DYNAMIC: bool = true;
    const SOL_NAME: &'static str = "bytes";

    fn encode_body_len(&self) -> usize {
        let data_len = self.0.len();
        let padding = (32 - (data_len % 32)) % 32;
        32 + data_len + padding
    }

    fn encode_body_to(&self, buf: &mut [u8]) {
        let data_len = self.0.len();
        let padding = (32 - (data_len % 32)) % 32;

        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&(data_len as u64).to_be_bytes());

        buf[32..32 + data_len].copy_from_slice(&self.0);
        buf[32 + data_len..32 + data_len + padding].fill(0);
    }

    fn indexed_topic(&self) -> [u8; 32] {
        crate::keccak256(&self.0)
    }
}

impl crate::SolArrayElement for Bytes {}

impl SolDecode for Bytes {
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let data_offset = crate::read_word_offset(input, offset)?;
        Self::decode_tail(input, data_offset)
    }

    fn decode_tail(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let len = crate::read_word_offset(input, offset)?;
        let data_start = crate::checked_sum([offset, 32])?;
        let data_end = crate::checked_sum([data_start, len])?;
        let data = &input.get(data_start..data_end).ok_or(DecodeError)?;
        Ok(Bytes(data.to_vec()))
    }
}

impl From<alloc::vec::Vec<u8>> for Bytes {
    fn from(v: alloc::vec::Vec<u8>) -> Self {
        Self(v)
    }
}

impl From<Bytes> for alloc::vec::Vec<u8> {
    fn from(b: Bytes) -> Self {
        b.0
    }
}

/// `Bytes` uses solc's `bytes` storage layout: short (< 32 B) values fit
/// inline at the field's slot; longer ones store the length in the slot and
/// spill the body to `keccak256(slot) + i`. Same machinery as `String`.
impl crate::StorageEncode for Bytes {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 32;
    const HAS_DYNAMIC_BODY: bool = true;

    fn write_to_storage(&self, host: &crate::Host, base_key: &[u8; 32]) {
        crate::storage_codec::write_dynamic_bytes(host, base_key, &self.0);
    }

    fn clear_storage(host: &crate::Host, base_key: &[u8; 32]) {
        crate::storage_codec::clear_dynamic_bytes(host, base_key);
    }
}

impl crate::StorageDecode for Bytes {
    fn read_from_storage(host: &crate::Host, base_key: &[u8; 32]) -> Self {
        Bytes(crate::storage_codec::read_dynamic_bytes(host, base_key))
    }

    fn try_read_from_storage(host: &crate::Host, base_key: &[u8; 32]) -> Option<Self> {
        // Header-only peek: zero header means nothing written. Empty values
        // are tagged with EMPTY_INLINE_SENTINEL at byte 30 (see storage_codec).
        use crate::HostApi;
        let mut header = [0u8; 32];
        host.get_storage_or_zero(crate::StorageFlags::empty(), base_key, &mut header);
        if header == [0u8; 32] {
            return None;
        }
        Some(Self::read_from_storage(host, base_key))
    }
}

#[cfg(feature = "abi-gen")]
impl crate::StorageTypeName for Bytes {
    fn name() -> alloc::string::String {
        alloc::string::String::from(<Self as SolEncode>::SOL_NAME)
    }
}

impl SolEncode for alloc::string::String {
    const IS_DYNAMIC: bool = true;
    const SOL_NAME: &'static str = "string";

    fn encode_body_len(&self) -> usize {
        let data_len = self.len();
        let padding = (32 - (data_len % 32)) % 32;
        32 + data_len + padding
    }

    fn encode_body_to(&self, buf: &mut [u8]) {
        let bytes = self.as_bytes();
        let data_len = bytes.len();
        let padding = (32 - (data_len % 32)) % 32;

        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&(data_len as u64).to_be_bytes());

        buf[32..32 + data_len].copy_from_slice(bytes);
        buf[32 + data_len..32 + data_len + padding].fill(0);
    }

    fn indexed_topic(&self) -> [u8; 32] {
        crate::keccak256(self.as_bytes())
    }
}

impl crate::SolArrayElement for alloc::string::String {}

impl SolDecode for alloc::string::String {
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let data_offset = crate::read_word_offset(input, offset)?;
        Self::decode_tail(input, data_offset)
    }

    fn decode_tail(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let len = crate::read_word_offset(input, offset)?;
        let data_start = crate::checked_sum([offset, 32])?;
        let data_end = crate::checked_sum([data_start, len])?;
        let data = &input.get(data_start..data_end).ok_or(DecodeError)?;
        alloc::string::String::from_utf8(data.to_vec()).map_err(|_| DecodeError)
    }
}

impl<T: SolEncode> SolEncode for alloc::vec::Vec<T> {
    const IS_DYNAMIC: bool = true;
    const SOL_NAME: &'static str = {
        struct H<T>(core::marker::PhantomData<T>);
        impl<T: SolEncode> H<T> {
            const V: crate::ConstStr = crate::ConstStr::new(T::SOL_NAME, "[]");
        }
        H::<T>::V.as_str()
    };

    fn encode_body_len(&self) -> usize {
        if T::IS_DYNAMIC {
            let tails_len: usize = self.iter().map(|e| e.encode_body_len()).sum();
            32 + self.len() * 32 + tails_len
        } else {
            32 + self.iter().map(|e| e.encode_body_len()).sum::<usize>()
        }
    }

    fn encode_body_to(&self, buf: &mut [u8]) {
        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&(self.len() as u64).to_be_bytes());

        if T::IS_DYNAMIC {
            let mut offset_pos = 32;
            let mut tail_pos = 32 + self.len() * 32;
            for elem in self.iter() {
                let rel_offset = tail_pos - 32;
                buf[offset_pos..offset_pos + 32].fill(0);
                buf[offset_pos + 24..offset_pos + 32]
                    .copy_from_slice(&(rel_offset as u64).to_be_bytes());
                offset_pos += 32;

                let tail_len = elem.encode_body_len();
                elem.encode_body_to(&mut buf[tail_pos..tail_pos + tail_len]);
                tail_pos += tail_len;
            }
        } else {
            let mut pos = 32;
            for elem in self.iter() {
                let len = elem.encode_body_len();
                elem.encode_body_to(&mut buf[pos..pos + len]);
                pos += len;
            }
        }
    }

    /// For `Vec<T>`, ABI type is `T_abi_type[]` and components come from T.
    #[cfg(feature = "abi-gen")]
    fn abi_param(name: &str) -> crate::AbiParam {
        let inner = T::abi_param("");
        crate::AbiParam {
            name: name.into(),
            param_type: alloc::format!("{}[]", inner.param_type),
            components: inner.components,
        }
    }
}

impl<T: SolEncode> crate::SolArrayElement for alloc::vec::Vec<T> {}

impl<T: SolDecode> SolDecode for alloc::vec::Vec<T> {
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        // Read the head pointer with checked arithmetic so an attacker-controlled
        // offset near `usize::MAX` fails closed instead of wrapping — symmetric
        // with `String`/`Bytes::decode_at` and `Vec::decode_tail`.
        let data_offset = crate::read_word_offset(input, offset)?;
        Self::decode_tail(input, data_offset)
    }

    fn decode_tail(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let len = crate::read_word_offset(input, offset)?;
        if len > input.len() / 32 {
            return Err(DecodeError);
        }
        let mut result = alloc::vec::Vec::with_capacity(len);
        let array_data_start = crate::checked_sum([offset, 32])?;

        if T::IS_DYNAMIC {
            for i in 0..len {
                let head_pos = crate::checked_sum([array_data_start, i * 32])?;
                let elem_offset = crate::read_word_offset(input, head_pos)?;

                let elem_start = crate::checked_sum([array_data_start, elem_offset])?;
                result.push(T::decode_tail(input, elem_start)?);
            }
        } else {
            let mut elem_offset = array_data_start;
            for _ in 0..len {
                let elem = T::decode_at(input, elem_offset)?;
                elem_offset = crate::checked_sum([elem_offset, T::HEAD_SIZE])?;
                result.push(elem);
            }
        }
        Ok(result)
    }
}

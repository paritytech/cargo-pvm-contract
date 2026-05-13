use crate::{DecodeError, SolDecode, SolEncode};

/// Wrapper for raw byte data that encodes as Solidity `bytes` (packed encoding).
///
/// Use `Bytes` instead of `Vec<u8>` when the Solidity signature uses `bytes`.
/// `Vec<u8>` encodes as `uint8[]` (array of 32-byte-padded elements), while
/// `Bytes` encodes as `bytes` (length-prefixed packed data), matching alloy's
/// distinction between `Bytes` and `Vec<u8>`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

impl crate::SolArrayElement for Bytes {}

impl SolDecode for Bytes {
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let data_offset = input
            .get(offset + 24..offset + 32)
            .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(u64::from_be_bytes)? as usize;
        Self::decode_tail(input, data_offset)
    }

    fn decode_tail(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let len = input
            .get(offset + 24..offset + 32)
            .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(u64::from_be_bytes)? as usize;
        let data = &input
            .get(offset + 32..offset + 32 + len)
            .ok_or(DecodeError)?;
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
}

impl crate::SolArrayElement for alloc::string::String {}

impl SolDecode for alloc::string::String {
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let data_offset = input
            .get(offset + 24..offset + 32)
            .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(u64::from_be_bytes)? as usize;
        Self::decode_tail(input, data_offset)
    }

    fn decode_tail(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let len = input
            .get(offset + 24..offset + 32)
            .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(u64::from_be_bytes)? as usize;
        let data = &input
            .get(offset + 32..offset + 32 + len)
            .ok_or(DecodeError)?;
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
        let data_offset = input
            .get(offset + 24..offset + 32)
            .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(u64::from_be_bytes)? as usize;
        Self::decode_tail(input, data_offset)
    }

    fn decode_tail(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let len = input
            .get(offset + 24..offset + 32)
            .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(u64::from_be_bytes)? as usize;
        if len > input.len() / 32 {
            return Err(DecodeError);
        }
        let mut result = alloc::vec::Vec::with_capacity(len);
        let array_data_start = offset + 32;

        if T::IS_DYNAMIC {
            for i in 0..len {
                let elem_offset = input
                    .get(array_data_start + i * 32 + 24..array_data_start + i * 32 + 32)
                    .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
                    .ok_or(DecodeError)
                    .map(u64::from_be_bytes)? as usize;

                result.push(T::decode_tail(input, array_data_start + elem_offset)?);
            }
        } else {
            let mut elem_offset = array_data_start;
            for _ in 0..len {
                let elem = T::decode_at(input, elem_offset)?;
                elem_offset += T::HEAD_SIZE;
                result.push(elem);
            }
        }
        Ok(result)
    }
}

use crate::{DecodeError, SolError};

/// Standard Solidity `Error(string)` revert.
///
/// Selector: `0x08c379a0` = `keccak256("Error(string)")[0:4]`
///
/// This is what Solidity's `require(condition, "message")` produces.
/// It's the most common error type in the Ethereum ecosystem.
///
/// Warning
///
/// Decoding said error is impossible due to ownership constraints rust's type system enforces.
/// All errors successfully decoded by this type will return `RevertString("")`.
/// To enable proper decoding, enable `alloc` feature.
#[allow(unused)]
#[derive(Debug, PartialEq)]
pub struct RevertString<'a>(pub &'a str);

impl SolError for RevertString<'_> {
    const SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
    const SIGNATURE: &'static str = "Error(string)";

    fn encoded_size(&self) -> usize {
        // 68 = 4 (selector) + 32 (offset word) + 32 (length word)
        68 + ((self.0.len() + 31) & !31)
    }

    fn encode_to(&self, buf: &mut [u8]) -> usize {
        buf[0..4].copy_from_slice(&Self::SELECTOR);
        let buf = &mut buf[4..];
        let size = {
            let str_bytes = self.0.as_bytes();

            // ABI string encoding: [offset: 32 bytes][length: 32 bytes][data: padded to 32]
            // Need at least 64 bytes for the offset + length words
            if buf.len() < 64 {
                let n = buf.len().min(32);
                buf[..n].fill(0);
                return 4 + n;
            }

            // Truncate string to fit buffer, aligned down to 32-byte boundary
            let max_data = (buf.len() - 64) & !31;
            let actual_len = str_bytes.len().min(max_data);
            let padding = (32 - (actual_len % 32)) % 32;
            let total = 64 + actual_len + padding;

            buf[..total].fill(0);
            buf[24..32].copy_from_slice(&32u64.to_be_bytes()); // offset
            buf[56..64].copy_from_slice(&(actual_len as u64).to_be_bytes()); // length
            buf[64..64 + actual_len].copy_from_slice(&str_bytes[..actual_len]); // data

            total
        };
        4 + size
    }

    fn decode_at(input: &[u8], offset: usize) -> Result<Option<Self>, DecodeError> {
        if input.len() < 4 {
            return Err(DecodeError);
        }
        if input
            .get(offset..offset + 4)
            .is_some_and(|x| x == Self::SELECTOR)
        {
            Ok(Some(Self("")))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn revert_string_truncates_long_message() {
        // With a 100-byte buffer: max_data_space = 36, rounded down to 32.
        let long_msg = "a".repeat(200);
        let error = RevertString(&long_msg);
        let mut buf = [0u8; 104];
        let len = error.encode_to(&mut buf);

        // Should not panic, and should fit in buffer
        assert!(len <= 104);

        // Verify the encoded length field matches the truncated string
        let encoded_len =
            u64::from_be_bytes(buf[32 + 24 + 4..32 + 32 + 4].try_into().unwrap()) as usize;
        assert!(encoded_len < long_msg.len());
        assert!(encoded_len <= 32); // 100 - 64 = 36, rounded down to 32
    }

    #[test]
    fn revert_string_fits_in_256_byte_revert_buffer() {
        // Simulate the full revert_data path with a 256-byte buffer
        // (4 selector + up to 252 params)
        let msg = "x".repeat(180); // long but should fit
        let error = RevertString(&msg);
        let mut buf = [0u8; 256];
        let len = error.encode_to(&mut buf);
        assert!(len <= 256);
        assert_eq!(&buf[0..4], &RevertString::SELECTOR);

        // Decode with alloy to verify it's valid
        let decoded = alloy_core::sol_types::decode_revert_reason(&buf[..len]);
        assert!(decoded.is_some());
    }

    #[test]
    fn revert_string_very_long_truncates_in_revert_buffer() {
        // A 300-char string must be truncated to fit in 256-byte revert buffer
        let msg = "y".repeat(300);
        let error = RevertString(&msg);
        let mut buf = [0u8; 256];
        let len = error.encode_to(&mut buf);
        assert!(len <= 256);

        // The encoded string length should be less than 300
        let encoded_str_len =
            u64::from_be_bytes(buf[4 + 32 + 24..4 + 32 + 32].try_into().unwrap()) as usize;
        assert!(encoded_str_len < 300);
    }
}

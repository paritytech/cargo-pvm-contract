use crate::{DecodeError, SolDecode, SolError};

/// Standard Solidity `Error(string)` revert.
///
/// Selector: `0x08c379a0` = `keccak256("Error(string)")[0:4]`
///
/// This is what Solidity's `require(condition, "message")` produces.
/// It's the most common error type in the Ethereum ecosystem.
#[derive(Debug, PartialEq)]
pub struct RevertString(pub alloc::string::String);

impl SolError for RevertString {
    const SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
    const SIGNATURE: &'static str = "Error(string)";

    fn encoded_size(&self) -> usize {
        // 68 = 4 (selector) + 32 (offset word) + 32 (length word)
        68 + ((self.0.len() + 31) & !31)
    }

    fn encode_to(&self, buf: &mut [u8]) -> usize {
        crate::revert_string::RevertString::encode_to(
            &crate::revert_string::RevertString(&self.0),
            buf,
        )
    }

    fn decode_at(input: &[u8], offset: usize) -> Result<Option<Self>, DecodeError> {
        if input.len() < 4 {
            return Err(DecodeError);
        }
        if input
            .get(offset..offset + 4)
            .is_some_and(|x| x == Self::SELECTOR)
        {
            let (data,) = <(alloc::string::String,) as SolDecode>::decode(
                input.get(4..).ok_or_else(|| DecodeError)?,
            )?;
            Ok(Some(Self(data)))
        } else {
            Ok(None)
        }
    }
}

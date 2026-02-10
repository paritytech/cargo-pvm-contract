use tiny_keccak::{Hasher, Keccak};

pub fn compute_selector(canonical_signature: &str) -> [u8; 4] {
    let mut hasher = Keccak::v256();
    hasher.update(canonical_signature.as_bytes());
    let mut output = [0u8; 32];
    hasher.finalize(&mut output);
    [output[0], output[1], output[2], output[3]]
}

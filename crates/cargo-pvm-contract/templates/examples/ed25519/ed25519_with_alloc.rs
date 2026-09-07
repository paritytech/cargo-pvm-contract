#![no_main]
#![no_std]

#[pvm_contract_sdk::contract("Ed25519.sol", allocator = "bump")]
mod ed25519 {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use pvm_contract_sdk::EmptyError;

    pub struct Ed25519;

    impl Ed25519 {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), EmptyError> {
            Ok(())
        }

        /// Verifies the signature `(signature_r, signature_s)` over the 41-byte
        /// message `message_1 || message_2` under `public_key`. A public key
        /// that is not a valid compressed Edwards point verifies as `false`
        /// rather than reverting.
        #[pvm_contract_sdk::method]
        pub fn check(
            public_key: [u8; 32],
            signature_r: [u8; 32],
            signature_s: [u8; 32],
            message_1: [u8; 32],
            message_2: [u8; 9],
        ) -> bool {
            let Ok(verifying_key) = VerifyingKey::from_bytes(&public_key) else {
                return false;
            };

            let mut signature_bytes = [0u8; 64];
            signature_bytes[..32].copy_from_slice(&signature_r);
            signature_bytes[32..].copy_from_slice(&signature_s);

            let mut message = [0u8; 41];
            message[..32].copy_from_slice(&message_1);
            message[32..].copy_from_slice(&message_2);

            verifying_key
                .verify(&message, &Signature::from_bytes(&signature_bytes))
                .is_ok()
        }
    }
}

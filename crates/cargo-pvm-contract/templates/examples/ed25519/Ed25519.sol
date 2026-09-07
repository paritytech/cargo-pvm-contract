// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

// Ed25519 signature verification carried out entirely in contract code, over
// the ABI of the `ed25519` workload in resolc-compiler-tests — the `check`
// signature of the Rainbow verifier. The 41-byte message arrives split across a
// `bytes32` and a `bytes9`, the way that verifier took it, and the contract
// joins the halves before hashing. Nothing is delegated to a precompile:
// ed25519-dalek's SHA-512 and Edwards25519 arithmetic are compiled into the
// blob, which makes this the one example that pulls in a third-party crate.
interface Ed25519 {
    function check(
        bytes32 publicKey,
        bytes32 signatureR,
        bytes32 signatureS,
        bytes32 message1,
        bytes9 message2
    ) external pure returns (bool);
}

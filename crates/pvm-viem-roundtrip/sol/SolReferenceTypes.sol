// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// Solidity reference types that are legal in an interface but that
// `type_to_abi_param` currently cannot resolve to an ABI type.
//
// Both shapes below are ordinary Solidity. solc encodes a contract- or
// interface-typed parameter as `address` (recording the original in
// `internalType`), and breaks a self-referential struct the same way any ABI
// consumer must — by expanding it to a fixed depth is impossible, so solc simply
// rejects recursive structs in external signatures. Our parser instead falls
// back to the bare type name, producing a `type` string that is not part of the
// ABI grammar. The suite asserts the resulting ABI is well-formed, so these fail
// until the fallback is replaced.

interface IToken {
    function totalSupply() external view returns (uint256);
}

/// A struct that reaches itself through a dynamic array.
struct Node {
    uint256 value;
    Node[] children;
}

interface SolReferenceTypes {
    /// A contract handle. solc: `{"type":"address","internalType":"contract IToken"}`.
    function setToken(IToken token) external;

    /// A contract handle inside a struct field and inside an array.
    function setTokens(IToken[] tokens) external;

    /// Self-referential struct.
    function addNode(Node n) external;
}

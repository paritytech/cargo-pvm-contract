// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// Solidity reference types that are legal in an interface and resolve to an
// ABI type only through a rule, not through their own declaration.
//
// A contract- or interface-typed parameter is an `address` on the wire. solc
// emits `{"type":"address","internalType":"contract IToken"}`; the ABI encoding
// does not depend on `internalType`, so the parser emits `address` and drops
// the rest. The handle is exercised bare, inside an array, inside a struct and
// as a return value, since each reaches `type_to_abi_param` through a
// different branch.
//
// A self-referential struct (`struct Node { Node[] children; }`) has no ABI
// representation at all and is rejected at build time; that case lives in the
// builder's own unit tests rather than here, because no fixture can be
// generated from it.

interface IToken {
    function totalSupply() external view returns (uint256);
}

struct Pool {
    IToken token;
    uint256 weight;
}

interface SolReferenceTypes {
    /// A bare contract handle.
    function setToken(IToken token) external;

    /// A contract handle inside an array.
    function setTokens(IToken[] calldata tokens) external;

    /// A contract handle inside a struct field.
    function setPool(Pool calldata p) external;

    /// A contract handle as a return value.
    function token() external view returns (IToken);
}

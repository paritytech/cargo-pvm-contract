// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Precompiles {
    function recover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external view returns (address);
    function verifyP256(bytes32 hash, bytes32 r, bytes32 s, bytes32 x, bytes32 y) external view returns (bool);
}

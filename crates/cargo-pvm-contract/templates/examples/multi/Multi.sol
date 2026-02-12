// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

interface Multi {
    function add(uint32 a, uint32 b) external pure returns (uint32);
    function multiply(uint64 a, uint64 b) external pure returns (uint64);
    function isEven(uint32 n) external pure returns (bool);
    function negate(uint256 value) external pure returns (uint256);
    function max(uint256 a, uint256 b) external pure returns (uint256);
    function hash(address account) external view returns (uint256);
    function sum3(uint32 a, uint32 b, uint32 c) external pure returns (uint32);
    function bitAnd(uint256 a, uint256 b) external pure returns (uint256);
    function isZero(uint256 value) external pure returns (bool);
    function increment(uint32 n) external pure returns (uint32);
}

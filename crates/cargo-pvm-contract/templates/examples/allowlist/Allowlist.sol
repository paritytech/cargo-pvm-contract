// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

interface Allowlist {
    function add(address a) external;
    function remove(uint64 index) external;
    function contains(address a) external view returns (bool);
    function count() external view returns (uint64);
    function at(uint64 index) external view returns (address);
}

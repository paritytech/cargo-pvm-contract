// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

interface Packed {
    function feeBps() external view returns (uint128);
    function maxSupply() external view returns (uint128);
    function setFeeBps(uint128 v) external;
    function setMaxSupply(uint128 v) external;
}

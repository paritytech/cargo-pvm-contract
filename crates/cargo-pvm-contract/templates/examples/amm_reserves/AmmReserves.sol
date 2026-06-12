// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

// Simplified Uniswap V2-style packed pool reserves. `reserve0` and
// `reserve1` are stored together so a single SLOAD reads both — the
// canonical Solidity pattern for sub-256-bit values that change atomically.
interface AmmReserves {
    function getReserves() external view returns (uint128, uint128);
    function sync(uint128 reserve0, uint128 reserve1) external;
}

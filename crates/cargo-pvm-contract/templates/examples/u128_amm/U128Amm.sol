// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

// Uniswap V2-style constant-product quoting over 128-bit reserves. Both
// reserves share a single storage slot, and the quote math (multiply, add,
// divide) stays inside 128-bit words — the shape a wide-integer instruction
// set is meant to serve.
interface U128Amm {
    error InsufficientLiquidity();

    function getReserves() external view returns (uint128, uint128);
    function sync(uint128 reserve0, uint128 reserve1) external;
    function getAmountOut(uint128 amountIn, uint128 reserveIn, uint128 reserveOut) external pure returns (uint128);
    function getAmountIn(uint128 amountOut, uint128 reserveIn, uint128 reserveOut) external pure returns (uint128);
    function swapExactIn(uint128 amountIn, bool zeroForOne) external returns (uint128);
    function quoteCumulative(uint128 amountIn, uint32 hops) external view returns (uint128);
}

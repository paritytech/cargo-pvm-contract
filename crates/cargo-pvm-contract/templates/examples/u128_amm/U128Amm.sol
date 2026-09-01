// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

// Uniswap V2-style constant-product pool with a `uint256` external ABI over
// 128-bit reserves, which still share a single storage slot. Every product and
// quotient in the quote math is evaluated at 256 bits, and each result is
// checked on the way back down into the pool's `uint128` interior, so no
// intermediate wraps: `amountIn * 997 * reserveOut` needs 266 bits at the top of
// the `uint128` range and reverts with `AmountTooLarge` rather than truncating.
interface U128Amm {
    error InsufficientLiquidity();
    error AmountTooLarge();

    function getReserves() external view returns (uint256, uint256);
    function sync(uint256 reserve0, uint256 reserve1) external;
    function getAmountOut(uint256 amountIn, uint256 reserveIn, uint256 reserveOut) external pure returns (uint256);
    function getAmountIn(uint256 amountOut, uint256 reserveIn, uint256 reserveOut) external pure returns (uint256);
    function swapExactIn(uint256 amountIn, bool zeroForOne) external returns (uint256);
    function quoteCumulative(uint256 amountIn, uint32 hops) external view returns (uint256);
}

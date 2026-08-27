// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

// Fixed-point interest accrual on a 128-bit principal. `mulDivDown` and
// `mulDivUp` are the two rounding directions every lending market needs;
// `accrue` compounds them in a loop (multiply, divide, remainder) and
// `compoundQ64` does the same in Q64.64 (multiply, shift).
interface U128Lending {
    function principal() external view returns (uint128);
    function setPrincipal(uint128 amount) external;
    function mulDivDown(uint128 x, uint128 y, uint128 denominator) external pure returns (uint128);
    function mulDivUp(uint128 x, uint128 y, uint128 denominator) external pure returns (uint128);
    function accrue(uint128 ratePerPeriodWad, uint32 periods) external returns (uint128);
    function compoundQ64(uint128 rateQ64, uint32 periods) external view returns (uint128);
    function utilizationWad(uint128 borrows, uint128 supply) external pure returns (uint128);
}

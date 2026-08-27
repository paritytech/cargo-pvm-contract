// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

// ERC20 shaped around 128-bit balances: the same storage and ABI traffic as
// the SDK's 256-bit `mytoken`, with every amount a `uint128`. This is the
// honest counterpart for measuring what the wide-integer extension does to a
// real token contract.
interface U128Erc20 {
    event Transfer(address indexed from, address indexed to, uint128 value);
    event Approval(address indexed owner, address indexed spender, uint128 value);

    error InsufficientBalance();
    error InsufficientAllowance();

    function totalSupply() external view returns (uint128);
    function balanceOf(address account) external view returns (uint128);
    function allowance(address owner, address spender) external view returns (uint128);

    function mint(address to, uint128 amount) external;
    function transfer(address to, uint128 amount) external;
    function approve(address spender, uint128 amount) external;
    function transferFrom(address from, address to, uint128 amount) external;
}

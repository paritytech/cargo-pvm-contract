// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.0;

// ERC20-conformant external ABI over a 128-bit interior: every amount crosses
// the boundary as `uint256`, while storage keeps the total supply, balances and
// allowances as `uint128`. Incoming amounts are checked on the way in, so a
// value above `type(uint128).max` reverts with `AmountTooLarge` — including the
// `type(uint256).max` idiom for unlimited approvals. This is the shape a
// `Balance = uint128` ledger takes when it has to speak ERC20.
interface U128Erc20 {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    error InsufficientBalance();
    error InsufficientAllowance();
    error AmountTooLarge();

    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function allowance(address owner, address spender) external view returns (uint256);

    function mint(address to, uint256 amount) external;
    function transfer(address to, uint256 amount) external;
    function approve(address spender, uint256 amount) external;
    function transferFrom(address from, address to, uint256 amount) external;
}

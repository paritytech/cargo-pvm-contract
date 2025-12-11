// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

interface MyToken {
    event Transfer(address indexed from, address indexed to, uint256 value);
    error InsufficientBalance();

    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);

    function transfer(address to, uint256 amount) external;
    function mint(address to, uint256 amount) external;
}

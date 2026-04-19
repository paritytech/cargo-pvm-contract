// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Payable {
    function deposit() external payable;
    function depositTo(address to) external payable;
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address who) external view returns (uint256);
}

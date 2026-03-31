// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface ConstructorArgs {
    function getOwner() external view returns (address);
    function getInitialSupply() external view returns (uint256);
}

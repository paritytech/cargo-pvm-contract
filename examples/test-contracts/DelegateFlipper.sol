// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface DelegateFlipper {
    function delegateFlipper(address addr) external;
    function get() external view returns (bool);
}

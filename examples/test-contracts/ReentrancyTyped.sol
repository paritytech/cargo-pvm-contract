// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface ReentrancyTyped {
    function onCallback() external returns (bool);
    function lendWithReentry(address borrower) external;
    function lendWithoutReentry(address borrower) external;
}
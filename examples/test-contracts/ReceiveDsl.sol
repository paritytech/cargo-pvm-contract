// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IReceiveDsl {
    receive() external payable;
    function totalReceived() external view returns (uint256);
    function receiveCount() external view returns (uint256);
}

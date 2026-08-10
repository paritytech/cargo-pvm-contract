// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface CallerCheck {
    function getCaller() external view returns (address);
    function getOrigin() external view returns (address);
    function getSelfAddress() external view returns (address);
    function getBlockNumber() external view returns (uint64);
    function getTimestamp() external view returns (uint64);
    function getChainId() external view returns (uint64);
    function getBalance() external view returns (uint256);
    function getBalanceOf(address account) external view returns (uint256);
    function recordCaller() external;
    function recordContext() external;
    function recordContextOn(address target) external;
    function getLastCaller() external view returns (address);
    function getLastOrigin() external view returns (address);
    function getLastSelf() external view returns (address);
}

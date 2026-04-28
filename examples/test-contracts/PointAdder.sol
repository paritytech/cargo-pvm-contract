// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

struct Point {
    uint a;
    uint b;
}

interface PointAdder {
    function add(Point a, Point b) external returns (Point);
}

interface StructParam {
    struct Point {
        uint64 x;
        uint64 y;
    }

    function sum(Point p) external view returns (uint64);
}

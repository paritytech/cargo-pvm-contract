interface StructMismatch {
    struct Point {
        uint64 x;
        uint64 y;
    }

    function echo(Point p) external view returns (Point);
}

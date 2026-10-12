interface CtorStructMismatch {
    struct Point {
        uint64 x;
        uint64 y;
        uint64 z;
    }

    constructor(Point origin);
}

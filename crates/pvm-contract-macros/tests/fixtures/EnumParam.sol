interface EnumParam {
    enum Choice {
        Yes,
        No
    }

    function choose(Choice c) external view returns (uint64);
}

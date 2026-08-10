// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

// Exercises the `.sol` -> ABI type mapping in
// `cargo-pvm-contract-builder/src/abi.rs`. Nothing implements this interface: the
// builder derives an ABI by parsing the file, so a standalone `.sol` is enough,
// and keeping it out of `examples/test-contracts` keeps it off the riscv build.
//
// Every declaration here targets a distinct branch of `type_to_abi_param`.

/// Resolves to `uint8`.
enum Status {
    Pending,
    Active,
    Closed
}

/// A user-defined value type resolves to its underlying elementary type.
type Timestamp is uint64;

/// Fully static struct.
struct Pair {
    uint64 lo;
    uint64 hi;
}

/// Struct containing a struct: nested `tuple` components.
struct Nested {
    Pair inner;
    address owner;
}

/// Struct with dynamic members, so the tuple carries a head/tail split.
struct Profile {
    uint256 id;
    string name;
    uint32[] tags;
}

/// Struct whose fields cover the array shapes.
struct Bundle {
    Pair[] dynamicPairs;
    Pair[2] fixedPairs;
    bytes4[3] tags;
}

interface SolTypeSurface {
    // `uint` and `int` without a width are aliases for the 256-bit forms.
    function bareAliases(uint a, int b) external pure returns (uint, int);

    // Every elementary type the parser has a branch for.
    function elementary(
        address a,
        bool b,
        string calldata c,
        bytes calldata d,
        bytes1 e,
        bytes17 f,
        bytes32 g,
        uint8 h,
        uint128 i,
        int8 j,
        int256 k
    ) external pure returns (bool);

    // Widths solc allows but the SDK has no Rust counterpart for, so no
    // encoding vector can exist. Kept because the parser must still map them.
    function oddWidths(uint24 a, uint40 b, int72 c) external pure returns (uint200);

    // `uint8[N]` is unreachable from Rust: `[u8; N]` maps to `bytesN`, and
    // `Vec<u8>` to `uint8[]`, so a *fixed* array of `uint8` has no spelling.
    function byteArray(uint8[2] a) external pure returns (uint8[2] memory);

    // Enum -> uint8, user-defined value type -> its underlying type.
    function userDefined(Status status, Timestamp at) external pure returns (Status);

    // Named struct -> tuple with named components.
    function takesPair(Pair p) external pure returns (Pair);

    // Struct within a struct.
    function takesNested(Nested n) external pure returns (Nested);

    // Dynamic struct.
    function takesProfile(Profile p) external pure returns (Profile);

    // Arrays of structs, both dynamic and fixed length.
    function takesBundle(Bundle b) external pure returns (Bundle);

    // Inline tuple -> tuple with unnamed components.
    function inlineTuple((uint256, bool) t) external pure returns ((uint256, bool));

    // Nested arrays: the suffix order has to survive the recursion.
    function nestedArrays(uint256[][2] a, string[][] b) external pure returns (uint256[3][] memory);

    // Array of enums and of value types.
    function userDefinedArrays(Status[] s, Timestamp[2] t) external pure returns (Status[] memory);

    // Errors with static, dynamic and composite fields.
    error Unauthorized();
    error InsufficientBalance(address account, uint256 required, uint256 available);
    error DetailedFailure(string reason, uint32 code);
    error CompositeFailure(Pair p, uint256[] values);

    // Events across the indexed-parameter shapes.
    event Simple(address indexed who, uint256 amount);
    event IndexedDynamic(string indexed name, bytes indexed payload, string note);
    event IndexedComposite(Pair indexed p, uint256[] values);
    event ThreeIndexed(address indexed a, uint256 indexed b, bytes32 indexed c, uint64 d);
    event Anonymous(uint256 indexed a, address indexed b, bool c) anonymous;
}

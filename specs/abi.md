# ABI Encoding/Decoding Specification

This document describes how `pvm-contract-macros` encodes and decodes method arguments and return values following the Ethereum ABI specification.

## Overview

The macro generates code that:
1. Decodes input calldata (after the 4-byte selector) into Rust types
2. Encodes return values back to ABI-compliant bytes

All values are encoded as 32-byte words, big-endian, right-aligned (for integers) or left-aligned (for bytes).

## Supported Types

### Type Mapping: Solidity → Rust

| Solidity Type | Rust Type | Notes |
|---------------|-----------|-------|
| `address` | `Address` | Wrapper around `[u8; 20]`, right-aligned in 32-byte word |
| `bool` | `bool` | 0 or 1 in last byte |
| `uint8` | `u8` | |
| `uint16` | `u16` | |
| `uint32` | `u32` | |
| `uint64` | `u64` | |
| `uint128` | `u128` | |
| `uint256` / `uint` | `U256` | |
| `int8` | `i8` | Two's complement |
| `int16` | `i16` | Two's complement |
| `int32` | `i32` | Two's complement |
| `int64` | `i64` | Two's complement |
| `int128` | `i128` | Two's complement |
| `int256` / `int` | `I256` | Two's complement |
| `bytes1`..`bytes32` | `[u8; N]` | Left-aligned, zero-padded |
| `bytes` | `Vec<u8>` (alloc) / `&[u8]` (no_alloc) | Dynamic |
| `string` | `String` (alloc) / `&str` (no_alloc) | Dynamic, UTF-8 |
| `T[]` | `Vec<T>` | Dynamic array (alloc only) |
| `T[N]` | `[T; N]` | Fixed-size array |
| `(T1, T2, ...)` | `(T1, T2, ...)` | Tuple |

## Decoding (Input → Rust)

### Static Types

Static types occupy exactly 32 bytes in the calldata (except for packed fixed arrays/tuples).

#### Address
```text
Calldata: [00 00 00 00 00 00 00 00 00 00 00 00 XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX]
                                              └─────────────────── 20 bytes ───────────────────────────┘
```
Decoded by extracting bytes 12-32:
```rust,ignore
let mut addr = [0u8; 20];
addr.copy_from_slice(&input[offset + 12..offset + 32]);
addr
```

#### Boolean
```text
Calldata: [00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 XX]
                                                                                                      └─ 0 or 1
```
Decoded by checking the last byte:
```rust,ignore
input[offset + 31] != 0
```

#### Unsigned Integers

All unsigned integers are right-aligned in the 32-byte word:

| Type | Bytes Used | Position in 32-byte word |
|------|------------|--------------------------|
| `uint8` | 1 | byte 31 |
| `uint16` | 2 | bytes 30-31 |
| `uint32` | 4 | bytes 28-31 |
| `uint64` | 8 | bytes 24-31 |
| `uint128` | 16 | bytes 16-31 |
| `uint256` | 32 | bytes 0-31 |

Example for `uint32`:
```rust,ignore
u32::from_be_bytes(input[offset + 28..offset + 32].try_into().unwrap())
```

#### Signed Integers

Same layout as unsigned, but interpreted as two's complement:
```rust,ignore
i32::from_be_bytes(input[offset + 28..offset + 32].try_into().unwrap())
```

#### Fixed Bytes (`bytes1` to `bytes32`)

Left-aligned, zero-padded on the right:
```text
Calldata for bytes4: [XX XX XX XX 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00]
                      └─ 4 bytes ─┘
```
```rust,ignore
let mut bytes = [0u8; 4];
bytes.copy_from_slice(&input[offset..offset + 4]);
```

### Dynamic Types

Dynamic types store an offset pointer in the head section, with actual data in the tail.

#### Layout
```text
Head: [offset to data (32 bytes)]
...
Tail: [length (32 bytes)][actual data (length bytes, padded to 32)]
```

#### Dynamic Bytes (`bytes`)
```rust,ignore
let dyn_offset = U256::from_be_slice(&input[offset..offset + 32]).as_limbs()[0] as usize;
let length = U256::from_be_slice(&input[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
// alloc mode:
input[dyn_offset + 32..dyn_offset + 32 + length].to_vec()
// no_alloc mode:
&input[dyn_offset + 32..dyn_offset + 32 + length]
```

#### String
Same as `bytes`, but converted to UTF-8:
```rust,ignore
// alloc mode:
String::from_utf8_lossy(bytes).into_owned()
// no_alloc mode:
core::str::from_utf8(bytes).unwrap_or("")
```

#### Dynamic Arrays (`T[]`)

**Only supported in alloc mode.**

```rust,ignore
let dyn_offset = U256::from_be_slice(&input[offset..offset + 32]).as_limbs()[0] as usize;
let length = U256::from_be_slice(&input[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
let array_data = &input[dyn_offset + 32..];
let mut result = Vec::with_capacity(length);
for i in 0..length {
    let elem_data = &array_data[i * ELEM_SIZE..];
    result.push(decode_element(elem_data));
}
```

### Composite Types

#### Fixed Arrays (`T[N]`)

Elements are concatenated without length prefix:
```text
[element_0 (32 bytes)][element_1 (32 bytes)]...[element_N-1 (32 bytes)]
```

Each element is decoded at its respective offset:
```rust,ignore
[
    decode(&input, offset + 0 * 32),
    decode(&input, offset + 1 * 32),
    // ...
]
```

#### Tuples (`(T1, T2, ...)`)

Static tuples: elements concatenated sequentially.

```rust,ignore
(
    decode_T1(&input, offset),
    decode_T2(&input, offset + T1::head_size()),
    // ...
)
```

## Encoding (Rust → Output)

### Static Types

#### Address
```rust,ignore
let mut out = [0u8; 32];
out[12..32].copy_from_slice(&address);
```

#### Boolean
```rust,ignore
let mut out = [0u8; 32];
out[31] = if value { 1 } else { 0 };
```

#### Unsigned Integers

Right-aligned in 32 bytes:
```rust,ignore
// uint32 example
let mut out = [0u8; 32];
out[28..32].copy_from_slice(&value.to_be_bytes());

// uint256
value.to_be_bytes::<32>()
```

#### Signed Integers

Two's complement, sign-extended:
```rust,ignore
// int32 example
let mut out = if value < 0 { [0xff; 32] } else { [0u8; 32] };
out[28..32].copy_from_slice(&value.to_be_bytes());
```

#### Fixed Bytes

Left-aligned:
```rust,ignore
let mut out = [0u8; 32];
out[..N].copy_from_slice(&value);
```

### Dynamic Types

Dynamic types (`String`, `Vec<T>`) are supported for return values in alloc mode:
```rust,ignore
#[pvm_contract::method]
pub fn get_name() -> String {
    "hello".to_string()
}
```
In no_alloc mode, returning a dynamic type will cause a compile error.

### Composite Types

#### Fixed Arrays

Each element encoded and concatenated:
```rust,ignore
// alloc mode
let mut out = Vec::with_capacity(N * 32);
for elem in array {
    out.extend_from_slice(&encode(elem));
}

// no_alloc mode
let mut out = [0u8; N * 32];
let mut offset = 0;
for elem in array {
    out[offset..offset + 32].copy_from_slice(&encode(elem));
    offset += 32;
}
```

#### Static Tuples

Elements concatenated:
```rust,ignore
let mut out = [0u8; TOTAL_SIZE];
let mut offset = 0;
out[offset..offset + 32].copy_from_slice(&encode(tuple.0));
offset += 32;
out[offset..offset + 32].copy_from_slice(&encode(tuple.1));
// ...
```

## Limitations

### Not Supported

| Feature | Status | Workaround |
|---------|--------|------------|
| Dynamic arrays in `no_alloc` mode | Not supported | Use `alloc` feature or fixed arrays |

### Custom Types with `#[derive(SolType)]`

Custom structs are supported via the `SolType` derive macro. This generates `SolEncode`, `SolDecode`, and (for static-only structs) `StaticEncodedLen` implementations.

```rust,ignore
#[derive(pvm_contract_macros::SolType)]
pub struct Point {
    pub x: U256,
    pub y: U256,
}
```

This generates:
- `SolEncode` impl with `SOL_NAME = "(uint256,uint256)"`, `encode_len`, `encode_to`
- `StaticEncodedLen` impl with `ENCODED_SIZE = 64` (static structs only)
- `SolDecode` impl with `decode`, `decode_at`

Use in contract methods:
```rust,ignore
#[pvm_contract_macros::method]
pub fn set_point(point: Point) {
    // point.x, point.y are available
}

#[pvm_contract_macros::method]
pub fn get_point() -> Point {
    Point { x: U256::from(1), y: U256::from(2) }
}
```

#### Static vs Dynamic Structs

Structs with only static fields generate `StaticEncodedLen` and can be returned in both alloc and no_alloc modes.
Structs with any dynamic field (String, Vec) are dynamic and can only be returned in alloc mode.

```rust,ignore
#[derive(pvm_contract_macros::SolType)]
pub struct User {
    pub name: String,
    pub age: u8,
}

#[pvm_contract::method]
pub fn get_user() -> User {
    User { name: "Alice".into(), age: 30 }
}
```

#### Supported Field Types

- `U256`, `u128`, `u64`, `u32`, `u16`, `u8`
- `i128`, `i64`, `i32`, `i16`, `i8`
- `bool`
- `Address` (address)
- `[u8; N]` (bytesN)
- `[T; N]` (fixed array, T must implement `SolArrayElement`)
- `String` (dynamic, requires alloc)
- `Vec<T>` (dynamic, requires alloc)
- Other `SolType` structs (nested)

Note: `&str` implements `SolEncode` but not `SolDecode` (borrowed types cannot be decoded from a buffer). Use `String` for decode support.

#### Alternative: Tuples

You can also use tuples directly without defining a struct:
```rust,ignore
#[pvm_contract_macros::method]
pub fn set_point(p: (U256, U256)) {
    let (x, y) = p;
}
```

## Input Size Validation

The macro generates size checks before decoding:

```rust,ignore
let min_size = sum of head_size() for all parameters;
if input.len() < min_size {
    return_value(REVERT, b"InvalidCalldata");
}
```

Head sizes:
- Static types: 32 bytes each
- Dynamic types: 32 bytes (offset pointer)
- Fixed arrays of static types: `element_size * count`
- Static tuples: sum of element head sizes

## Examples

### Simple Method

```solidity
function transfer(address to, uint256 amount) external;
```

Input layout (68 bytes total = 4 selector + 64 data):
```text
[selector: 4 bytes]
[to: 32 bytes, address right-aligned]
[amount: 32 bytes, uint256]
```

Generated decode:
```rust,ignore
let to = {
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&input[12..32]);
    addr
};
let amount = U256::from_be_slice(&input[32..64]);
```

### Method with Return Value

```solidity
function balanceOf(address account) external view returns (uint256);
```

Generated encode for return:
```rust,ignore
let encoded = result.to_be_bytes::<32>();
return_value(ReturnFlags::empty(), &encoded);
```

### Fixed Array Parameter

```solidity
function setScores(uint256[3] scores) external;
```

Input layout (100 bytes = 4 + 96):
```text
[selector: 4 bytes]
[scores[0]: 32 bytes]
[scores[1]: 32 bytes]
[scores[2]: 32 bytes]
```

Generated decode:
```rust,ignore
let scores = [
    U256::from_be_slice(&input[0..32]),
    U256::from_be_slice(&input[32..64]),
    U256::from_be_slice(&input[64..96]),
];
```

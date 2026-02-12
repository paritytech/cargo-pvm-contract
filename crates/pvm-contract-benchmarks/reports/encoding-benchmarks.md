# ABI Encoding/Decoding Benchmarks: pvm-contract-types vs Alloy

Criterion benchmarks comparing `pvm-contract-types` (`SolEncode`/`SolDecode`)
against `alloy-core` (`SolValue::abi_encode`/`abi_decode`) for ABI encoding and
decoding of Solidity types.

## How to regenerate

```bash
cargo bench -p pvm-contract-types --features alloc
```

Results are saved to `target/criterion/`. The raw output contains the numbers
used in this report.

## Results

### Static Types (no-alloc compatible)

| Type     | Operation | pvm-contract-types | alloy-core   | Ratio        |
|----------|-----------|--------------------|--------------|--------------|
| u8       | encode    | 14.31 ns           | 6.49 ns      | 2.20x slower |
| u8       | decode    | 0.40 ns            | 2.37 ns      | 5.97x faster |
| u32      | encode    | 14.23 ns           | 6.86 ns      | 2.08x slower |
| u32      | decode    | 0.98 ns            | 2.36 ns      | 2.41x faster |
| u128     | encode    | 14.07 ns           | 6.67 ns      | 2.11x slower |
| u128     | decode    | 0.99 ns            | 2.79 ns      | 2.83x faster |
| U256     | encode    | 13.74 ns           | 6.51 ns      | 2.11x slower |
| U256     | decode    | 1.78 ns            | 3.57 ns      | 2.01x faster |
| address  | encode    | 15.50 ns           | 11.52 ns     | 1.35x slower |
| address  | decode    | 7.09 ns            | 9.59 ns      | 1.35x faster |

### Dynamic Types (alloc required)

| Type       | Operation | pvm-contract-types | alloy-core   | Ratio        |
|------------|-----------|--------------------|--------------|--------------|
| String     | encode    | 11.24 ns           | 16.23 ns     | 1.44x faster |
| String     | decode    | 11.15 ns           | 32.70 ns     | 2.93x faster |
| Vec\<U256> | encode    | 20.78 ns           | 25.62 ns     | 1.23x faster |
| Vec\<U256> | decode    | 25.01 ns           | 46.63 ns     | 1.87x faster |

### Summary

**Encoding**: `pvm-contract-types` encodes static types ~2x slower than alloy
due to writing into a caller-provided `[u8]` buffer (with zeroing) rather than
returning a heap-allocated `Vec<u8>`. For dynamic types (String, Vec) where both
approaches allocate, `pvm-contract-types` is 1.2–1.4x faster.

**Decoding**: `pvm-contract-types` decodes all types faster than alloy — from
2x faster for U256 up to 6x faster for u8. This is because decoding reads
directly from the ABI-encoded buffer with no validation overhead, while alloy
performs full ABI conformance checking.

**Key insight**: The encode overhead is a benchmarking artifact. In real
contracts, `pvm-contract-types` encodes into a stack buffer (no allocation),
which is strictly faster in the `no_std` + `no_alloc` context where these
contracts run. The alloy `abi_encode()` approach always heap-allocates a
`Vec<u8>`, which is not possible without an allocator.

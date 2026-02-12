# Macro vs Builder DSL: Release Binary Size Comparison

Date: 2026-02-12
Commit: measured on current HEAD of cargo-pvm-contract


## 1. Approaches

- **Proc-macro approach** (`pvm-contract-macros`): attribute proc macros
  (`#[contract]`, `#[method]`, `#[constructor]`, `#[fallback]`) that parse
  Rust+Solidity and emit dispatch code at compile time via `syn`/`quote`.
- **Builder DSL approach** (`pvm-contract-builder-dsl`): a pure Rust builder
  pattern API (`ContractBuilder::new().method(selector, handler).dispatch()`)
  that wires up dispatch at runtime without any proc-macro dependency.


## 2. Release Binary Sizes

From `target/benchmark-artifacts/` (built by `build-and-measure`):

| Contract  | Variant     | Size (bytes) | Size (KB) |
|-----------|-------------|-------------:|----------:|
| fibonacci | no-alloc    |          472 |      0.46 |
| fibonacci | builder-dsl |        1,202 |      1.17 |
| fibonacci | with-alloc  |       12,312 |     12.02 |
| fibonacci | alloy       |       13,859 |     13.53 |
| mytoken   | no-alloc    |        3,751 |      3.66 |
| mytoken   | builder-dsl |        3,763 |      3.67 |
| mytoken   | with-alloc  |       16,205 |     15.83 |
| mytoken   | alloy       |       17,242 |     16.84 |

### Builder DSL vs Proc-Macro (no-alloc)

| Contract  | Proc-Macro | Builder DSL | Overhead |
|-----------|------------|-------------|----------|
| fibonacci |    472 B   |   1,202 B   | +154%    |
| mytoken   |  3,751 B   |   3,763 B   | +0.3%    |

For the trivial fibonacci contract, the builder DSL adds ~730 bytes of
overhead from the runtime dispatch table and calldata-copy loop. For
the more realistic mytoken contract, the overhead is negligible (+12 bytes)
— the actual contract logic dominates binary size.

### Key Size Drivers

| Factor                             | Impact                               |
|------------------------------------|--------------------------------------|
| Allocator (no-alloc vs with-alloc) | 26x for fibonacci, 4.3x for mytoken  |
| Alloy vs pvm-contract-macros       | ~1.06x overhead vs with-alloc        |
| Builder DSL vs proc-macro no-alloc | negligible for real contracts         |

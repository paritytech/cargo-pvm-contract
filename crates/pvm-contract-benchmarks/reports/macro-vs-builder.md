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
| multi     | no-alloc    |        3,434 |      3.35 |
| multi     | builder-dsl |        3,575 |      3.49 |

### Builder DSL vs Proc-Macro (no-alloc)

| Contract  | Methods | Proc-Macro | Builder DSL | Overhead |
|-----------|--------:|------------|-------------|----------|
| fibonacci |       1 |    472 B   |   1,202 B   | +154%    |
| mytoken   |       4 |  3,751 B   |   3,763 B   | +0.3%    |
| multi     |      10 |  3,434 B   |   3,575 B   | +4.1%    |

For the trivial fibonacci contract (1 method), the builder DSL adds ~730 bytes
of overhead from the runtime dispatch table and calldata-copy loop. As method
count grows, the fixed overhead is amortized: mytoken (4 methods) shows +0.3%
and multi (10 methods with mixed parameter types) shows +4.1%. The builder DSL
does not become cheaper than the proc-macro with more methods, but the overhead
stays negligible for real contracts.

### Key Size Drivers

| Factor                             | Impact                               |
|------------------------------------|--------------------------------------|
| Allocator (no-alloc vs with-alloc) | 26x for fibonacci, 4.3x for mytoken  |
| Alloy vs pvm-contract-macros       | ~1.06x overhead vs with-alloc        |
| Builder DSL vs proc-macro no-alloc | +4% at 10 methods, negligible at scale |

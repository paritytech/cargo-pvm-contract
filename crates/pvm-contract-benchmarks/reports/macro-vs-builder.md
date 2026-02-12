# Macro vs Builder DSL: Release Binary Size Comparison

Date: 2026-02-10
Commit: measured on current HEAD of cargo-pvm-contract


## 1. Approaches

- **Proc-macro approach** (`pvm-contract-macros`): attribute proc macros
  (`#[contract]`, `#[method]`, `#[constructor]`, `#[fallback]`) that parse
  Rust+Solidity and emit dispatch code at compile time via `syn`/`quote`.
- **Builder DSL approach** (`pvm-contract-builder-dsl`): a pure Rust builder
  pattern API (`ContractBuilder::new().method(selector, handler).dispatch()`)
  that wires up dispatch at runtime without any proc-macro dependency.


## 2. Release Binary Sizes (Proc-Macro Variants)

From `target/benchmark-artifacts/` (built by `build-and-measure`):

| Contract  | Variant    | Size (bytes) | Size (KB) |
|-----------|------------|-------------:|----------:|
| fibonacci | no-alloc   |          472 |      0.46 |
| fibonacci | with-alloc |       12,312 |     12.02 |
| fibonacci | alloy      |       13,859 |     13.53 |
| mytoken   | no-alloc   |        3,751 |      3.66 |
| mytoken   | with-alloc |       16,205 |     15.83 |
| mytoken   | alloy      |       17,242 |     16.84 |

### Key Size Drivers

| Factor                             | Impact                               |
|------------------------------------|--------------------------------------|
| Allocator (no-alloc vs with-alloc) | 26x for fibonacci, 4.3x for mytoken  |
| Alloy vs pvm-contract-macros       | ~1.06x overhead vs with-alloc        |

The dominant binary size factor is allocation strategy, not the macro
system used.

### Builder DSL Binary Size (Not Yet Measured)

The `build-and-measure` tool does not yet build builder-DSL variants.
Both approaches generate equivalent dispatch logic for the same contract,
so binary sizes are expected to be **nearly identical** for equivalent
contracts in the same allocation mode — the final machine code depends
on the expanded Rust, not on how it was generated.


## 3. Limitations

1. **No direct binary-size comparison**: builder DSL variants are not
   built by the benchmark tool. The "expected identical" claim is based
   on code analysis, not measurement.
2. Adding a builder-DSL variant to the benchmark pipeline requires a
   new `Variant::BuilderDsl` enum arm in `build-and-measure.rs` and
   matching Cargo.toml / source templates.

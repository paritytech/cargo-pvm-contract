# Macro vs Builder DSL Comparison Report

Date: 2026-02-10
Commit: measured on current HEAD of cargo-pvm-contract


## 1. Scope and Methodology

This report compares two contract-authoring approaches in cargo-pvm-contract:

- **Proc-macro approach** (`pvm-contract-macros`): attribute proc macros
  (`#[contract]`, `#[method]`, `#[constructor]`, `#[fallback]`) that parse
  Rust+Solidity and emit dispatch code at compile time via `syn`/`quote`.
- **Builder DSL approach** (`pvm-contract-builder-dsl`): declarative
  `macro_rules!` macros (`pvm_contract!`) that expand inline dispatch code
  without any proc-macro dependency.

Metrics gathered:

| Metric             | Method                                             |
|--------------------|----------------------------------------------------|
| Framework LOC      | `wc -l` on source files                            |
| User-facing LOC    | `wc -l` on comparable example contracts            |
| Compile time       | `cargo clean -p <crate>; time cargo check -p ...`  |
| Binary size        | Existing `target/benchmark-artifacts/` from CI tool |
| Dependencies       | `cargo tree -p <crate>` (direct + transitive)      |

All compile-time measurements: 3 runs, median reported. Machine: Linux,
measurements taken after deps are cached (only the target crate rebuilt).

**Limitation**: binary-size data comes from the existing benchmark pipeline
which builds only proc-macro variants (no-alloc/with-alloc/alloy). The
builder DSL examples cannot be built through the same pipeline today because
`build-and-measure` hardcodes proc-macro Cargo.toml templates. A direct
binary-size comparison of proc-macro vs builder DSL for identical contracts
is therefore **not yet available** and is a follow-up item.


## 2. Lines of Code

### 2.1 Framework Implementation (dispatch infrastructure)

```
Command: wc -l crates/pvm-contract-macros/src/**/*.rs
         wc -l crates/pvm-contract-builder-dsl/src/*.rs
```

| Component                | Proc-Macro | Builder DSL |
|--------------------------|------------|-------------|
| codegen/contract.rs      |        538 |         n/a |
| codegen/dispatch.rs      |        230 |         n/a |
| codegen/encode.rs        |        233 |         n/a |
| codegen/decode.rs        |        273 |         n/a |
| codegen/method.rs        |         75 |         n/a |
| codegen/mod.rs           |         10 |         n/a |
| codegen/sol_type.rs      |        512 |         n/a |
| signature/ (3 files)     |        476 |         n/a |
| solidity.rs              |        185 |         n/a |
| lib.rs                   |        560 |         452 |
| selector.rs              |        n/a |           9 |
| **Subtotal (dispatch)**  |  **1,349** |     **461** |
| **Total crate**          |  **3,092** |     **461** |

The proc-macro crate is ~6.7x larger because it contains:
- Full Solidity interface parser and signature inference
- AST manipulation via `syn`/`quote`
- Type-aware encode/decode code generation
- Custom type (SolType derive) support

The builder DSL achieves dispatch with 461 lines of `macro_rules!` by
pushing type resolution to the user (explicit Solidity signatures and
return-type annotations).

### 2.2 User-Facing Contract Code

```
Command: wc -l crates/pvm-contract-builder-dsl/examples/*.rs
         wc -l crates/cargo-pvm-contract/templates/examples/**/*_{no_alloc,with_alloc}.rs
```

| Contract  | Proc-Macro (no-alloc) | Proc-Macro (with-alloc) | Builder DSL |
|-----------|-----------------------|-------------------------|-------------|
| fibonacci |                    40 |                      48 |          39 |
| mytoken   |                   146 |                     155 |         170 |

For the simple fibonacci contract, line counts are nearly identical.
For mytoken, the builder DSL is ~16% longer because:
- Explicit Solidity signatures in `#[method("transfer(address,uint256)")]`
- Explicit `result` / `returns(Type)` annotations
- Mock API stubs for non-RISC-V compilation (25 lines of `mod api`)


## 3. Compile-Time Comparison

### 3.1 Crate-Level Check (deps cached, only target crate rebuilt)

```
Command (3 runs each):
  cargo clean -p pvm-contract-macros; time cargo check -p pvm-contract-macros
  cargo clean -p pvm-contract-builder-dsl; time cargo check -p pvm-contract-builder-dsl
```

| Crate              | Run 1  | Run 2  | Run 3  | Median |
|--------------------|--------|--------|--------|--------|
| pvm-contract-macros| 0.39s  | 0.35s  | 0.37s  | 0.37s  |
| builder-dsl        | 0.10s  | 0.09s  | 0.09s  | 0.09s  |

**Builder DSL compiles ~4x faster** (0.09s vs 0.37s median).

This is expected: `pvm-contract-macros` is a proc-macro crate that
depends on `syn` (full features), `quote`, and `proc-macro2`, all of
which must be compiled for the host target. The builder DSL has no
proc-macro overhead.

### 3.2 Dependency Footprint

```
Command: cargo tree -p <crate> --depth 1
         cargo tree -p <crate> | wc -l  (total transitive)
```

| Metric              | Proc-Macro | Builder DSL |
|---------------------|------------|-------------|
| Direct deps         |          5 |           5 |
| Total transitive    |         17 |          31 |
| Proc-macro deps     | syn, quote, proc-macro2 | none |

Note: the builder DSL has more transitive deps because it directly depends
on `pallet-revive-uapi`, `polkavm-derive`, and `ruint` (runtime deps that
the proc-macro crate does not carry -- those are dependencies of the
*user's* contract, not the macro crate itself). In a full contract build,
both approaches pull in the same runtime dependencies.


## 4. Binary Size Comparison

### 4.1 Available Data (Proc-Macro Variants)

From `target/benchmark-artifacts/` (built by `build-and-measure`):

| Contract  | Variant    | Profile | Size (bytes) | Size (KB) |
|-----------|------------|---------|-------------:|----------:|
| fibonacci | no-alloc   | release |          472 |      0.46 |
| fibonacci | with-alloc | release |       12,312 |     12.02 |
| fibonacci | alloy      | release |       13,859 |     13.53 |
| mytoken   | no-alloc   | release |        3,751 |      3.66 |
| mytoken   | with-alloc | release |       16,205 |     15.83 |
| mytoken   | alloy      | release |       17,242 |     16.84 |

### 4.2 Builder DSL Binary Size (Not Yet Measured)

The `build-and-measure` tool does not yet build builder-DSL variants.
Adding a `builder-dsl` variant to the benchmark pipeline requires:

1. A new `Variant::BuilderDsl` enum arm in `build-and-measure.rs`
2. A Cargo.toml template using `pvm-contract-builder-dsl` instead of
   `pvm-contract-macros`
3. Source files referencing the `pvm_contract!` macro

**Expected outcome**: Both approaches generate equivalent dispatch logic
for the same contract. The builder DSL uses `macro_rules!` expansion
(zero proc-macro overhead at compile time) but produces structurally
identical `call()` and `deploy()` functions. Binary sizes should be
**nearly identical** for equivalent contracts in the same allocation mode,
since the final machine code depends on the expanded Rust, not on how
it was generated.

### 4.3 Key Binary Size Drivers (from existing data)

| Factor              | Impact                                      |
|---------------------|---------------------------------------------|
| Allocator (no-alloc vs with-alloc) | 26x for fibonacci, 4.3x for mytoken |
| Alloy vs pvm-contract-macros       | ~1.06x overhead vs with-alloc       |
| Debug vs release                    | ~13-75x (LTO + opt-level=z)         |

The dominant binary size factor is allocation strategy, not the macro
system used. This supports the hypothesis that builder DSL binary sizes
would match proc-macro sizes for equivalent contracts.


## 5. Ergonomics Comparison

### 5.1 Proc-Macro Approach

Pros:
- **Automatic signature inference**: Rust function types map to Solidity
  types automatically; no manual `"transfer(address,uint256)"` strings
- **Solidity interface validation**: `.sol` file parsed at compile time;
  missing implementations are caught as errors
- **Familiar attribute syntax**: `#[method]`, `#[constructor]` feel
  native to Rust developers
- **SolType derive**: custom struct encode/decode generated automatically
- **IDE support**: proc macros integrate with rust-analyzer (hover,
  go-to-definition on generated items)
- **Dynamic return types**: `dyn_len` attribute enables runtime-sized
  return buffers

Cons:
- **Compile-time cost**: `syn`/`quote` add ~0.3s per rebuild
- **Debugging opacity**: macro expansion errors reference generated code
  that does not exist in source files
- **Framework complexity**: 3,092 lines of implementation to maintain
- **Solidity parser fragility**: hand-rolled `.sol` parser may break on
  edge cases

### 5.2 Builder DSL Approach

Pros:
- **Zero proc-macro dependency**: no `syn`/`quote`; compiles 4x faster
- **Transparent expansion**: `macro_rules!` can be inspected with
  `cargo expand`; errors map directly to source
- **Small implementation**: 461 lines total, easy to audit and maintain
- **Explicit control**: user specifies exact Solidity signatures, no
  inference magic to debug

Cons:
- **Manual signatures required**: user must write
  `#[method("fibonacci(uint32)", returns(u32))]` -- error-prone and
  duplicates information already in the Rust function signature
- **No Solidity validation**: typos in signature strings compile fine
  but produce wrong selectors at runtime
- **No custom type support**: no equivalent to `#[derive(SolType)]`;
  user must manually encode/decode complex types
- **Limited IDE support**: `macro_rules!` expansion is less well
  supported by rust-analyzer than proc macros
- **Combinatorial arm explosion**: 6 dispatch arms x 8 decode-and-call
  arms for all combinations of params/result/encode; adding new
  patterns (e.g., `dyn_len`) requires adding more arms


## 6. Summary Matrix

| Dimension            | Proc-Macro          | Builder DSL         | Winner         |
|----------------------|---------------------|---------------------|----------------|
| Framework LOC        | 3,092               | 461                 | Builder DSL    |
| User contract LOC    | ~similar            | ~similar (+16% tok) | Tie            |
| Compile time (crate) | 0.37s               | 0.09s               | Builder DSL    |
| Binary size          | measured (see above) | expected identical  | Tie (expected) |
| Type safety          | compile-time checks | runtime selector    | Proc-Macro     |
| Solidity validation  | yes (.sol parsing)  | no                  | Proc-Macro     |
| Custom types         | SolType derive      | manual              | Proc-Macro     |
| Debuggability        | opaque expansion    | transparent rules   | Builder DSL    |
| Maintainability      | complex but modular | simple but brittle  | Context-dep.   |


## 7. Limitations

1. **No direct binary-size comparison**: builder DSL variants are not
   built by the benchmark tool. The "expected identical" claim is based
   on code analysis, not measurement.
2. **Compile-time measured without contract build**: we measured
   `cargo check` of the framework crates, not full PolkaVM contract
   builds (which require nightly + `-Zbuild-std`). The 4x speedup
   applies to the framework crate only; full contract build times are
   dominated by LLVM and polkavm-linker.
3. **No macro expansion size comparison**: we did not measure the
   token count of expanded code for identical contracts. This would
   require `cargo expand` on a RISC-V target.
4. **Single machine**: compile times may vary across hardware.


## 8. Recommendations

1. **Add builder-DSL variant to `build-and-measure`**: extend the
   benchmark tool with a `Variant::BuilderDsl` to confirm binary-size
   parity. This is the highest-value follow-up.
2. **Consider hybrid approach**: use `macro_rules!` for dispatch
   scaffolding (fast compile, transparent) with a thin proc-macro
   layer only for Solidity signature inference and validation.
3. **Measure full contract build times**: time `cargo +nightly build`
   for identical contracts using both approaches to capture end-to-end
   impact including LLVM codegen.
4. **Track compile-time in CI**: add framework crate compile-time to
   the benchmark dashboard alongside binary sizes.

# cargo-pvm-contract

Cargo subcommand and toolchain for building Rust smart contracts targeting PolkaVM (used by Polkadot's `pallet-revive`). Scaffolds projects from Solidity `.sol` interfaces, generates ABI encoding/decoding, and compiles to `.polkavm` bytecode.

## Crate Overview

| Crate | Description |
|-------|-------------|
| `cargo-pvm-contract` | CLI tool — scaffolds contract projects from `.sol` files |
| `cargo-pvm-contract-builder` | Build helper — `build.rs` integration that links PolkaVM bytecode and generates ABI JSON |
| `pvm-contract-macros` | Proc macros — `#[contract]`, `#[method]`, `#[constructor]`, `#[fallback]` |
| `pvm-contract-types` | ABI encoding/decoding traits (`SolEncode`, `SolDecode`) — `no_std` compatible |
| `pvm-contract-builder-dsl` | Builder-pattern DSL for contracts without proc macros |
| `pvm-contract-benchmarks` | Binary size comparison tool for CI regression detection |

## Prerequisites

- **Rust 1.92+** (stable) — workspace MSRV
- **Rust nightly** with `rust-src` — needed for `-Zbuild-std` when building contracts and benchmarks
- **solc 0.8.26+** — Solidity compiler for `.sol` interface parsing

```bash
rustup toolchain install nightly --component rust-src --profile minimal
```

## Build

```bash
# Build all workspace crates
cargo build

# Build just the CLI
cargo build -p cargo-pvm-contract
```

## Test

```bash
# All workspace tests
cargo test --workspace

# Unit tests (types + macros)
cargo test -p pvm-contract-types --features alloc
cargo test -p pvm-contract-macros

# Integration tests (scaffolds projects into temp dirs and builds them)
cargo test -p cargo-pvm-contract
```

## Lint & Format

```bash
cargo +nightly fmt
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Benchmarks

### Encoding benchmarks (criterion)

Compare `pvm-contract-types` encoding/decoding against `alloy-core` for primitives, U256, addresses, strings, and Vec<U256>:

```bash
# All benchmarks (with alloc-dependent types like String and Vec)
cargo bench -p pvm-contract-types --features alloc

# Without alloc (primitives + address only)
cargo bench -p pvm-contract-types
```

Results are saved to `target/criterion/`.

### Binary size benchmarks

Build all contract variants (no-alloc, with-alloc, alloy) × (fibonacci, mytoken) × (debug, release) and generate a comparison report:

```bash
cargo +nightly run -p pvm-contract-benchmarks --bin build-and-measure
```

Artifacts: `target/benchmark-artifacts/`
Report: `target/benchmark-results/binary-sizes.md`

In CI (`benchmark.yml`), PR builds are compared against `origin/main` with a 5% regression threshold per artifact.

## Examples

One scaffolded example project lives under `examples/example-mytoken`.

It keeps six MyToken variants as separate binaries in one Cargo project:

- `example-mytoken-macro-pico-alloc` — `pvm_contract_macros` with `allocator = "pico"`
- `example-mytoken-macro-bump-alloc` — `pvm_contract_macros` with `allocator = "bump"`
- `example-mytoken-macro-no-alloc` — `pvm_contract_macros` default stack mode
- `example-mytoken-macro-no-sol` — `pvm_contract_macros` without Solidity interface path
- `example-mytoken-dsl-no-alloc` — `pvm-contract-builder-dsl` variant
- `example-mytoken-alloy-alloc` — alloy-based alloc variant

To build the example variants:

```bash
cd examples/example-mytoken
env -u CARGO -u RUSTUP_TOOLCHAIN cargo build --release
```

The CI `check-examples` job verifies `examples/example-mytoken` builds.

## Editing Rust Code

- Do not add semicolons to existing `return` statements if the original code omits them
- Do not add braces to match arms if the original code uses the braceless form
- Do not introduce formatting-only changes
- Use `cargo +nightly fmt` for formatting
- Prefer `assert_eq!` on full structs over multiple field assertions

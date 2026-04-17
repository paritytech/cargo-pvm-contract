# Architecture & Usage Guide

## What This Project Does

`cargo-pvm-contract` is a toolchain for writing Rust smart contracts that compile to PolkaVM bytecode and run on Polkadot's `pallet-revive`. It provides scaffolding, proc macros, ABI encoding, and a build pipeline that produces `.polkavm` binaries with `.abi.json` metadata.

Contracts use the **Ethereum ABI** (Keccak-256 selectors, Solidity-compatible encoding), so they can be called by the same tooling used for EVM contracts (ethers.js, cast, etc.).

## Workspace Crates

```text
cargo-pvm-contract/
├── cargo-pvm-contract          CLI tool — scaffolds new contract projects
├── cargo-pvm-contract-builder  Build library — links PolkaVM bytecode + emits ABI JSON (used by CLI and optional build.rs)
├── pvm-contract-macros         Proc macros — #[contract], #[method], #[constructor], #[fallback], #[derive(SolType)]
├── pvm-contract-types          ABI traits — SolEncode / SolDecode, no_std compatible
├── pvm-contract-builder-dsl    Builder DSL — non-macro alternative (ContractBuilder)
├── pvm-bump-allocator          Bump allocator — simple no-dealloc heap for contract execution
├── pvm-contract-benchmarks     Benchmarks — binary size comparison tool
├── cargo-pvm-contract-extrinsics  Extrinsics library — Substrate RPC builders for upload, instantiate, call, etc.
└── pvm-contract-e2e-tests      E2E tests — integration tests against revive-dev-node
```

## Two API Styles

### 1. Proc Macro

Annotate a module with `#[contract]` and functions with `#[method]`. The macro generates entry points, calldata dispatch, and ABI encoding automatically.

See [proc-macros.md](proc-macros.md) for full usage, allocator options, error handling, custom types, and ABI generation.

### 2. Builder DSL (explicit control)

No proc macros. You wire up dispatch manually using `ContractBuilder` for full explicit control.

See [builder-dsl.md](builder-dsl.md) for full usage.

## Specifications

- [proc-macros.md](proc-macros.md) — `#[contract]`, `#[method]`, `#[constructor]`, `#[fallback]`, `#[derive(SolType)]`
- [builder-dsl.md](builder-dsl.md) — `ContractBuilder` dispatch
- [abi.md](abi.md) — ABI encoding/decoding, type mapping, wire format
- [build.md](build.md) — scaffolding, build pipeline, generated project structure
- [deployment.md](deployment.md) — deploying `.polkavm` bytecode using Ethereum tooling (cast/anvil-polkadot)
- [cli.md](cli.md) — native Substrate CLI reference (`cargo pvm-contract` subcommands)

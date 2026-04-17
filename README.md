# cargo-pvm-contract

A Cargo subcommand and toolchain for building Rust smart contracts targeting [PolkaVM](https://github.com/nickg/polkavm) (used by Polkadot's [`pallet-revive`](https://docs.rs/pallet-revive-uapi/latest/pallet_revive_uapi/)).

It scaffolds Rust contract projects, provides proc macros and a builder DSL for dispatch/ABI encoding, and compiles to `.polkavm` bytecode. A Solidity `.sol` interface file can optionally be provided to auto-generate method stubs and selectors, but is not required -- the macro can infer ABI information directly from Rust function signatures.

## Features

- **Project scaffolding** -- generate a complete Rust contract project from scratch, or from a `.sol` interface to get typed method stubs with selectors and ABI encoding wired up automatically
- **Two API styles** -- choose between proc macros (`#[contract]`, `#[method]`) for concise code or a builder-pattern DSL (`ContractBuilder::new().method(...)`) for explicit control
- **ABI generation** -- the build system compiles your contract on the host to extract type information, then emits a `.abi.json` alongside the `.polkavm` binary
- **Lightweight ABI encoding** -- `pvm-contract-types` provides `no_std`-compatible `SolEncode`/`SolDecode` traits as an alternative to `alloy-core`, keeping binary sizes small
- **Allocator options** -- contracts can run in stack-only mode (no allocator), or with a bump allocator (`pvm-bump-allocator`) or `picoalloc` for dynamic types like `String` and `Vec<T>`

## Installation

```bash
cargo install cargo-pvm-contract
```

Requires **Rust nightly** with `rust-src` for contract compilation (the scaffolded project handles this via `rust-toolchain.toml`):

```bash
rustup toolchain install nightly --component rust-src --profile minimal
```

Optionally install **solc 0.8.26+** if you want to scaffold from `.sol` files:

```bash
# macOS
brew install solidity

# Linux
SOLC_VERSION=0.8.26
curl -L https://github.com/ethereum/solidity/releases/download/v${SOLC_VERSION}/solc-static-linux -o solc
chmod +x solc && sudo mv solc /usr/local/bin/solc
```

## Quick Start

```bash
cargo pvm-contract
```

This launches an interactive prompt that walks you through:

1. **Init type** -- start from scratch or from a bundled example (Fibonacci, MyToken, Multi)
2. **API style** -- Macro (proc-macro attributes) or DSL (builder pattern)
3. **Allocator** -- Bump allocator (for dynamic types) or no allocator (stack-only, smaller binary)
4. **Solidity interface** -- optionally point to a `.sol` file to auto-generate method stubs

Build the generated project with the CLI:

```bash
cargo pvm-contract build
```

The PolkaVM bytecode and ABI JSON are written to:

```text
target/<profile>/<binary-name>.polkavm
target/<profile>/<binary-name>.abi.json
```

## API Styles

### Proc Macro

Annotate a module with `#[contract]` and mark functions with `#[method]`, `#[constructor]`, or `#[fallback]`. The macro generates the PolkaVM entry points, calldata dispatch, and ABI encoding:

```rust
#[pvm_contract_macros::contract("MyToken.sol", allocator = "bump")]
mod my_token {
    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> { Ok(()) }

    #[pvm_contract_macros::method]
    pub fn total_supply() -> U256 { U256::ZERO }

    #[pvm_contract_macros::method]
    pub fn transfer(to: Address, amount: U256) -> bool { todo!() }
}
```

### Builder DSL

Wire up dispatch explicitly using `ContractBuilder`. No proc macros needed -- all logic is visible in your source:

```rust
use pvm_contract_builder_dsl::{ContractBuilder, solidity_selector};

const TOTAL_SUPPLY: [u8; 4] = solidity_selector("totalSupply()");
const TRANSFER: [u8; 4] = solidity_selector("transfer(address,uint256)");

pub extern "C" fn call() {
    ContractBuilder::new()
        .method(TOTAL_SUPPLY, total_supply_handler)
        .method(TRANSFER, transfer_handler)
        .dispatch::<HostFnImpl, 256>()
}
```

## Workspace Crates

| Crate | Description |
|-------|-------------|
| `cargo-pvm-contract` | CLI -- scaffolds contract projects from `.sol` files |
| `cargo-pvm-contract-builder` | Build library -- links PolkaVM bytecode and generates ABI JSON (used by CLI and optional `build.rs`) |
| `pvm-contract-macros` | Proc macros -- `#[contract]`, `#[method]`, `#[constructor]`, `#[fallback]`, `#[derive(SolType)]` |
| `pvm-contract-types` | ABI encoding/decoding traits (`SolEncode`, `SolDecode`) -- `no_std` compatible |
| `pvm-contract-builder-dsl` | Builder-pattern DSL for contracts without proc macros |
| `pvm-bump-allocator` | Simple bump allocator for short-lived contract executions |
| `pvm-contract-benchmarks` | Binary size comparison tool for CI regression detection |

## Examples

The `examples/example-mytoken` project contains several MyToken variants as separate binaries:

| Binary | Style | Allocator |
|--------|-------|-----------|
| `example-mytoken-macro-pico-alloc` | Proc macro | picoalloc |
| `example-mytoken-macro-bump-alloc` | Proc macro | pvm-bump-allocator |
| `example-mytoken-macro-no-alloc` | Proc macro | None (stack-only) |
| `example-mytoken-macro-no-sol` | Proc macro | None (no `.sol` file) |
| `example-mytoken-dsl-no-alloc` | Builder DSL | None (stack-only) |
| `example-mytoken-alloy-alloc` | alloy-core | pvm-bump-allocator |

Build all variants:

```bash
cd examples/example-mytoken
cargo pvm-contract build
```

## License

Licensed under Apache 2.0
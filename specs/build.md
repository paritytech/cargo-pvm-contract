# Building Contracts

## Prerequisites

- **Rust 1.92+** (stable) — workspace MSRV

- **Rust nightly** with `rust-src` — needed for `-Zbuild-std` when compiling to PolkaVM

  ```bash
  rustup toolchain install nightly --component rust-src --profile minimal
  ```

- **solc 0.8.26+** — only if your contract references a `.sol` interface file

## Scaffold a New Project

```bash
cargo install cargo-pvm-contract
cargo pvm-contract init
```

Interactive prompts depend on the init type chosen:

**New project:**

1. **Contract name**
2. **API style** — Macro or DSL
3. **Allocator** — Bump or none
4. **Solidity interface** — optional `.sol` file for selector generation

**Bundled example** (Fibonacci, MyToken, Multi):

1. **Example** — which example to use
2. **API style** — Macro or DSL
3. **Contract name** — defaults to the example name

This generates a ready-to-build project:

```text
my_contract/
├── Cargo.toml            Dependencies + optimized release profile
├── rust-toolchain.toml   Pinned nightly (nightly-2026-02-01)
├── MyToken.sol           (only if .sol file was provided)
├── .cargo/
│   └── config.toml       RISC-V target + build-std configuration
├── src/
│   └── my_contract.rs    Contract source (macro or DSL)
└── .gitignore
```

## Build

Both options below are supported side by side. An existing project with a `build.rs` keeps
working unchanged, and `cargo pvm-contract build` works on any project regardless of whether
it has a `build.rs`.

### Option 1: CLI (recommended)

```bash
cd my_contract
cargo pvm-contract build
```

Output:

```text
target/release/my_contract.polkavm    — deployable bytecode
target/release/my_contract.abi.json   — Ethereum-compatible ABI (macro style only)
```

### Option 2: build.rs

Projects can also use `cargo-pvm-contract-builder` as a build dependency with a `build.rs` file:

```rust,ignore
// build.rs
fn main() {
    cargo_pvm_contract_builder::PvmBuilder::new().build();
}
```

```bash
cd my_contract
cargo build --release
```

Output:

```text
target/release/my_contract.polkavm    — deployable bytecode
target/release/my_contract.abi.json   — Ethereum-compatible ABI (macro style only)
```

## What Happens Under the Hood

```text
Rust Source (.rs)
    │  #[contract], #[method] macros expand at compile time
    │  → selector computation, dispatch logic, ABI encode/decode
    ▼
cargo build --target riscv64emac-unknown-none-polkavm
    │  -Zbuild-std=core,alloc
    │  profile: lto=true, opt-level="z", codegen-units=1, panic=abort
    ▼
RISC-V ELF Binary
    │
    ▼
polkavm-linker 0.31.0 (strip + optimize)
    │
    ▼
target/<profile>/<name>.polkavm   — deployable bytecode
target/<profile>/<name>.abi.json  — ABI metadata
```

The build is orchestrated by `cargo-pvm-contract-builder`, invoked either from the CLI (`cargo pvm-contract build`) or from a `build.rs` file.

## Debug vs Release

Both profiles produce `.polkavm` output:

```bash
cargo pvm-contract build                  # release (default)
cargo pvm-contract build --profile dev    # debug
```

Release builds are significantly smaller due to size optimization. Always use release for deployment.

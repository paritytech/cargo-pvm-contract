# cargo-pvm-contract

Cargo subcommand and toolchain for building Rust smart contracts targeting PolkaVM (used by Polkadot's `pallet-revive`). Scaffolds projects from Solidity `.sol` interfaces, generates ABI encoding/decoding, and compiles to `.polkavm` bytecode.

## Crate Overview

| Crate | Description |
|-------|-------------|
| `cargo-pvm-contract` | CLI tool — scaffolds contract projects from `.sol` files |
| `cargo-pvm-contract-builder` | Build helper — `build.rs` integration that links PolkaVM bytecode and generates ABI JSON |
| `pvm-contract-macros` | Proc macros — `#[contract]`, `#[method]`, `#[constructor]`, `#[fallback]`, `#[derive(SolType)]`, `#[derive(SolError)]` |
| `pvm-contract-types` | ABI encoding/decoding traits (`SolEncode`, `SolDecode`), error traits (`SolError`, `SolRevert`) — `no_std` compatible |
| `pvm-contract-builder-dsl` | Builder-pattern DSL for contracts without proc macros |
| `pvm-contract-benchmarks` | Binary size comparison tool for CI regression detection |

## How It Works

### End-to-End Pipeline

```
cargo pvm-contract (CLI)
    |
    v
Scaffold project from .sol interface or template
    |
    v
cargo build --release  (user runs this in the scaffolded project)
    |
    v
[build.rs] PvmBuilder::new().build()
    |
    +-- cargo build --target riscv64emac-unknown-none-polkavm -Zbuild-std=core,alloc
    |     |
    |     +-- #[contract] macro expands: dispatch + selectors + encode/decode
    |     +-- SolEncode/SolDecode trait impls handle ABI wire format
    |     +-- Output: ELF binary
    |
    +-- polkavm_linker (strip + optimize, TargetInstructionSet::ReviveV1)
    |     Output: target/{binary}.{profile}.polkavm
    |
    +-- ABI generation (parse .sol or run with --features abi-gen)
          Output: target/{binary}.{profile}.abi.json
```

### Two API Styles

**Macro API** (declarative, auto-ABI):
```rust
#[pvm_contract_macros::contract("MyToken.sol", buffer = 256)]
mod my_token {
    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> { Ok(()) }

    #[pvm_contract_macros::method]
    pub fn balance_of(account: Address) -> U256 { /* ... */ }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> { Ok(()) }
}
```

**DSL API** (explicit, manual dispatch):
```rust
ContractBuilder::new()
    .method(BALANCE_OF_SELECTOR, balance_of_handler)
    .method(TRANSFER_SELECTOR, transfer_handler)
    .dispatch::<HostFnImpl, 256>()
```

### Macro-Generated Code

The `#[contract]` macro generates two PolkaVM entry points:

- **`deploy()`** — calls the `#[constructor]` function
- **`call()`** — reads calldata, extracts 4-byte selector, dispatches to matching `#[method]`

Each method dispatch arm: validates input size -> decodes parameters via `SolDecode` -> calls user function -> encodes return via `SolEncode` -> returns to host. If the user function returns `Err(e)`, the error is encoded via `SolRevert::revert_data` and returned with `REVERT` flags.

Selectors are Keccak-256 of the canonical Solidity signature (first 4 bytes), computed at compile time.

### Contract Attribute Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `"path.sol"` | none | Solidity interface file (validates all functions are implemented) |
| `buffer = N` | 256 | Stack calldata buffer size (no-alloc mode) |
| `allocator = "pico"` | none | Use picoalloc heap allocator (enables dynamic types in returns) |
| `allocator = "bump"` | none | Use bump allocator |
| `allocator_size = N` | 1024 | Heap size in bytes for allocator modes |

### Method Attribute

- `#[method]` — marks a public function as a contract method
- `#[method(rename = "name")]` — overrides the Solidity function name (default: snake_case to camelCase)

## Type System

### Encoding Architecture

The SDK uses Solidity ABI encoding (Ethereum-compatible):

- All values are 32-byte words, big-endian
- Static types are right-aligned (integers) or left-aligned (fixed bytes)
- Dynamic types use head (offset pointer) + tail (length-prefixed data)
- Selectors are Keccak-256 of canonical signatures

### Core Traits (`pvm-contract-types`)

```rust
pub trait SolEncode {
    const IS_DYNAMIC: bool;        // true for String, Vec, bytes
    const SOL_NAME: &'static str;  // "uint256", "address", "(uint64,uint64)", etc.
    const HEAD_SIZE: usize;        // 32 for primitives, sum of fields for structs
    const SLOT_SIZE: usize;        // HEAD_SIZE for static, 32 for dynamic (default)
    const IS_TUPLE: bool;          // true only for Rust tuples (T1, T2, ...)
    fn encode_body_len(&self) -> usize;  // field body size
    fn encode_body_to(&self, buf: &mut [u8]);  // field body encoding
    fn encode_len(&self) -> usize;   // top-level size (default, IS_TUPLE/IS_DYNAMIC aware)
    fn encode_to(&self, buf: &mut [u8]);  // top-level encoding (default, smart wrapping)
}

pub trait SolDecode: SolEncode + Sized {
    fn decode(input: &[u8]) -> Self;
    fn decode_at(input: &[u8], offset: usize) -> Self;
    fn decode_tail(input: &[u8], offset: usize) -> Self;
}

pub trait StaticEncodedLen: SolEncode {
    const ENCODED_SIZE: usize;  // compile-time known size, used for stack buffers
}
```

### Error Traits (`pvm-contract-types`)

```rust
pub trait SolError {
    const SELECTOR: [u8; 4];       // keccak256 of canonical signature, first 4 bytes
    const SIGNATURE: &'static str; // e.g. "InsufficientBalance(address,uint256,uint256)"
    fn encode_params(&self, buf: &mut [u8]) -> usize;  // ABI-encode fields after selector
    fn encoded_size(&self) -> usize;                    // 4 + encoded params size
}

pub trait SolRevert {
    fn revert_data(&self, buf: &mut [u8]) -> usize;    // selector + encode_params
    fn revert_data_len(&self) -> usize;                 // total revert data size
    fn error_signatures() -> &'static [&'static str];   // for ABI JSON generation
}
```

- `SolError` — implemented per error struct (single selector). Use `#[derive(SolError)]`.
- `SolRevert` — dispatch boundary trait. Blanket impl for `T: SolError`. Manual impl for error enums via `sol_revert_enum!`.
- `RevertString` — encodes `Error(string)` with truncation for buffer safety.
- `Panic` — encodes `Panic(uint256)` for overflow/division-by-zero.
- `EmptyError` — zero-cost uninhabited type for contracts with no error paths.
- `sol_revert_enum!` — generates error enum + `SolRevert` impl + `From` conversions, auto-injects `RevertString` and `Panic` variants.

### Type Support Matrix

| Solidity Type | Rust Type | SolEncode | SolDecode | Trait Impl | Notes |
|---------------|-----------|-----------|-----------|------------|-------|
| `uint8`..`uint128` | `u8`..`u128` | yes | yes | `impl_static_type!` | |
| `uint256` | `U256` (ruint) | yes | yes | `impl_static_type!` | |
| `int8`..`int128` | `i8`..`i128` | yes | yes | `impl_static_type!` | Sign-extended encoding |
| `int256` | `I256` | yes | yes | `impl_static_type!` | Newtype around `U256` with two's-complement signed ops |
| `bool` | `bool` | yes | yes | `impl_static_type!` | |
| `address` | `Address` | yes | yes | `impl_static_type!` | Wrapper around `[u8; 20]` |
| `bytesN` | `[u8; N]` | yes | yes | blanket impl | SOL_NAME = `"bytesN"`, left-aligned encoding |
| `string` | `String` | yes | yes | alloc feature | |
| `string` (encode only) | `&str` | yes | no | core | Can't decode into a borrow |
| `bytes` | `Vec<u8>` | yes | yes | alloc feature | |
| `T[]` | `Vec<T>` | yes | yes | alloc feature, blanket impl | |
| `T[N]` (fixed array) | `[T; N]` | yes | yes | blanket impl, requires `T: SolArrayElement` | SOL_NAME = `"T[N]"` via `ConstStr` |
| `(T1,T2,...)` (tuple) | `(T, U, ...)` | yes | yes | macro-generated, arities 1-12 | SOL_NAME = `"(T1,T2,...)"` via `ConstStr` |
| custom struct | `#[derive(SolType)]` | yes | yes | proc macro generated | Also emits `SolArrayElement` |

### Wrapper Type: `Address`

`Address` wraps `[u8; 20]` and maps to Solidity `address`. This wrapper is needed because `[u8; N]` maps to `bytesN` (matching alloy's behavior), not `address`.

### `SolArrayElement` Marker Trait

The `SolArrayElement` marker trait controls which types can be used as elements in `[T; N]` fixed arrays. All types except `u8` implement `SolArrayElement`. This design (similar to alloy) ensures that `[u8; N]` always encodes as Solidity `bytesN` (left-aligned), while `[u32; N]` encodes as `uint32[N]` (array of 32-byte words). Without this marker, `[u8; N]` would have conflicting impls from both the `bytesN` path and the `[T; N]` blanket.

### Known Gaps

- **`&[u8]`**: No trait impl for byte slices. The macro compensates with inline codegen for no-alloc `bytes` decoding.

### Custom Types via `#[derive(SolType)]`

```rust
#[derive(SolType)]
struct Point {
    x: u64,
    y: u64,
}
// Generated: SOL_NAME = "(uint64,uint64)", IS_DYNAMIC = false, ENCODED_SIZE = 64
```

The derive macro detects whether a struct is static or dynamic:
- **Static** (all fields have compile-time known sizes): generates `StaticEncodedLen`, fixed-size encode/decode
- **Dynamic** (contains String, Vec, or custom types that might be dynamic): runtime offset tracking, head+tail separation

## Storage

Contracts use `pallet_revive_uapi::HostFnImpl` for persistent storage:

```rust
api::get_storage(StorageFlags::empty(), &key, &mut output)  // read 32 bytes
api::set_storage(StorageFlags::empty(), &key, &data)         // write 32 bytes
```

Keys are `[u8; 32]` arrays. Common patterns:
- **Single slot**: `const KEY: [u8; 32] = [0u8; 32]` with slot index at byte 31
- **Mappings**: Keccak-256 hash of (address + salt) to derive storage keys

## Host APIs

Contracts interact with the runtime through `pallet_revive_uapi::HostFnImpl`:

- `api::call_data_size()` / `api::call_data_copy()` — read calldata
- `api::return_value(flags, &data)` — return data or revert
- `api::caller(&mut output)` — get transaction sender (20 bytes)
- `api::get_storage()` / `api::set_storage()` — persistent storage
- `api::deposit_event(&topics, &data)` — emit events
- `api::hash_keccak_256(&input, &mut output)` — Keccak-256 hashing

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

Build all contract variants (no-alloc, with-alloc, alloy) x (fibonacci, mytoken) x (debug, release) and generate a comparison report:

```bash
cargo +nightly run -p pvm-contract-benchmarks --bin build-and-measure
```

Artifacts: `target/benchmark-artifacts/`
Report: `target/benchmark-results/binary-sizes.md`

In CI (`benchmark.yml`), PR builds are compared against `origin/main` with a 5% regression threshold per artifact.

## Examples

### example-mytoken

Six MyToken variants as separate binaries:

- `example-mytoken-macro-pico-alloc` — `pvm_contract_macros` with `allocator = "pico"`
- `example-mytoken-macro-bump-alloc` — `pvm_contract_macros` with `allocator = "bump"`
- `example-mytoken-macro-no-alloc` — `pvm_contract_macros` default stack mode
- `example-mytoken-macro-no-sol` — `pvm_contract_macros` without Solidity interface path
- `example-mytoken-dsl-no-alloc` — `pvm-contract-builder-dsl` variant
- `example-mytoken-alloy-alloc` — alloy-based alloc variant

### test-contracts

Multi-binary project with 9+ contracts for E2E integration tests:

- `Flipper` — boolean toggle
- `StorageTypes` — all primitive type storage roundtrips
- `MultiMethod` — multiple view + state methods
- `ReturnValues` — tuple returns
- `Events` — event emission with indexed params
- `DynamicTypes` — String, Vec<u8>, Vec<U256>
- `CompositeTypes` — fixed arrays, tuples
- `ConstructorArgs` — constructor with parameters
- `CallerCheck` — `api::caller()` access
- `ErrorHandling` — `SolError` + `sol_revert_enum!` ABI-encoded revert flow

### Building examples

```bash
cd examples/example-mytoken
env -u CARGO -u RUSTUP_TOOLCHAIN cargo build --release
```

The CI `check-examples` job verifies `examples/example-mytoken` builds.

## Project Structure

```
crates/
  cargo-pvm-contract/          CLI scaffolding tool
    src/scaffold.rs             Project generation from .sol or templates
    templates/                  Embedded project templates
  cargo-pvm-contract-builder/   build.rs helper
    src/lib.rs                  PvmBuilder: ELF build -> polkavm link -> ABI gen
    src/abi.rs                  ABI JSON generation (from .sol or abi-gen feature)
  pvm-contract-macros/          Proc macros
    src/codegen/contract.rs     #[contract] attribute parsing + module generation
    src/codegen/dispatch.rs     Selector computation + dispatch match arms
    src/codegen/encode.rs       (removed — encoding now handled directly in dispatch.rs)
    src/codegen/decode.rs       Parameter decoding codegen
    src/codegen/sol_type.rs     #[derive(SolType)] expansion
    src/codegen/sol_error.rs    #[derive(SolError)] expansion
    src/signature/types.rs      Rust-to-Solidity type mapping
    src/signature/parser.rs     Solidity signature parsing
    src/signature/selector.rs   Keccak-256 selector computation
    src/solidity.rs             .sol interface file parsing
  pvm-contract-types/           ABI encoding/decoding traits
    src/lib.rs                  SolEncode, SolDecode, StaticEncodedLen + primitive impls
    src/alloc_types.rs          String, Vec<T> impls (alloc feature)
  pvm-contract-builder-dsl/     Runtime dispatch DSL
  pvm-contract-benchmarks/      Binary size CI regression tool
  pvm-contract-e2e-tests/       E2E + integration test harness
examples/
  example-mytoken/              6 MyToken variants
  test-contracts/               9+ test contracts with .sol interfaces
specs/
  abi.md                        ABI encoding specification (includes error encoding)
  builder-dsl.md                Builder DSL specification (includes RevertBuffer)
```

## Editing Rust Code

- Do not add semicolons to existing `return` statements if the original code omits them
- Do not add braces to match arms if the original code uses the braceless form
- Do not introduce formatting-only changes
- Use `cargo +nightly fmt` for formatting
- Prefer `assert_eq!` on full structs over multiple field assertions
- Prefer direct value assertions (`assert_eq!` / `assert_ne!`) over substring checks when expected output is deterministic

## Documentation

- The proc macro doc comments in `crates/pvm-contract-macros/src/lib.rs` include `# Generated Code` sections showing what the macros expand to. When changing codegen, always update these examples to match the actual generated output.

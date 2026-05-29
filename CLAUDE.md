# cargo-pvm-contract

Cargo subcommand and toolchain for building Rust smart contracts targeting PolkaVM (used by Polkadot's `pallet-revive`). Scaffolds projects from Solidity `.sol` interfaces, generates ABI encoding/decoding, and compiles to `.polkavm` bytecode.

## Crate Overview

| Crate | Description |
|-------|-------------|
| `cargo-pvm-contract` | CLI tool — scaffolds contract projects from `.sol` files |
| `cargo-pvm-contract-builder` | Build library — links PolkaVM bytecode and generates ABI JSON (used by CLI and optional `build.rs`) |
| `pvm-contract-sdk` | Primary user-facing SDK crate — re-exports macros, types, and polkavm-derive for contract development |
| `pvm-contract-core` | Core structures for the PVM smart contracts SDK |
| `pvm-contract-macros` | Proc macros — `#[contract]`, `#[method]`, `#[constructor]`, `#[fallback]`, `#[receive]`, `#[derive(SolType)]`, `#[derive(SolError)]` |
| `pvm-contract-types` | ABI encoding/decoding traits (`SolEncode`, `SolDecode`), error traits (`SolError`, `SolRevert`) — `no_std` compatible |
| `pvm-storage` | Typed storage helpers — `Lazy<T>`, `Mapping<K, V>`, Solidity-compatible slot layout |
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
#[pvm_contract_sdk::contract("MyToken.sol", buffer = 256)]
mod my_token {
    pub struct MyToken;

    impl MyToken {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> { Ok(()) }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            self.host().get_storage(/* ... */);
            /* ... */
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), Error> { Ok(()) }
    }
}
```

The macro injects a `pub host: Host` field on the storage struct and a `fn host(&self) -> &Host` accessor. `Host` is a cfg-gated wrapper: zero-sized type over `PolkaVmHost` on riscv64; `Rc<dyn HostApi>` on host-target builds so it can be cheaply cloned into helpers like `Lazy`/`Mapping`, and tests can construct the contract with a `MockHost`.

**DSL API** (explicit, manual dispatch):
```rust
let host = Host::new();
ContractBuilder::new()
    .method(BALANCE_OF_SELECTOR, balance_of_handler)
    .method(TRANSFER_SELECTOR, transfer_handler)
    .dispatch_impl::<256>(&host)
```

DSL handlers take a concrete `&Host` (same type the macro path injects on the storage struct). For typed cross-contract calls, handlers wrap a cloned host in `Context::new(host.clone())` — `Context` impls `ContractContext` so it can be passed to `.call(&mut cx)` / `.delegate_call(&mut cx)`. `Host::clone()` is `Copy` on riscv64 (ZST) and a single `Rc::clone` on host targets. Because the wrapper carries only the host handle (no storage state), the borrow checker cannot enforce view-vs-mutating in DSL; use the `#[contract]` macro path if you need that static guarantee. The same `Context` type is used in unit tests, where it owns a `Host` backed by a `MockHost`.

### Macro-Generated Code

The `#[contract]` macro generates two PolkaVM entry points:

- **`deploy()`** — calls the `#[constructor]` function
- **`call()`** — reads calldata, extracts 4-byte selector, dispatches to matching `#[method]`. When `call_data_len == 0` and a `#[receive]` handler is present, the receive arm fires before the selector dispatch. When the selector matches no method (or calldata is 1..=3 bytes), control falls through to `#[fallback]` if present, else reverts.

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
- `#[payable]` — marks the method as `payable` (must be combined with `&mut self`)

### Mutability Inference

Solidity `stateMutability` is inferred from the Rust receiver. No explicit `#[view]` or `#[pure]` attribute — receiver shape is the source of truth.

| Receiver | `#[payable]` | ABI emits |
|---|---|---|
| none (`fn foo(args)`) | — | `pure` |
| `&self` | — | `view` |
| `&mut self` | — | `nonpayable` |
| `&mut self` | yes | `payable` |
| `&self` | yes | **compile error** |
| no receiver | yes | **compile error** |

**Constructor:** must take `&mut self`; `pure`/`view` constructors are rejected (they cannot initialize storage). `#[payable]` is allowed.

**Fallback:** follows the same inference table as regular methods.

**`.sol` consistency check:** when a `.sol` interface is provided, the macro errors if the Rust-inferred mutability disagrees with the `.sol` declaration (e.g., `.sol` says `view` but Rust uses `&mut self`).

### Mutability Enforcement

Three layers, in increasing strength:

1. **Compile-time (typed-API)** — `#[contract]` auto-implements `ContractContext` on the storage struct (and forbids `#[derive(Clone)]` on it). Cross-contract call builders take `&impl ContractContext` for `view`/`pure` callees and `&mut impl ContractContext` for `nonpayable`/`payable` callees, so a `&self` (view) method *cannot* initiate a state-mutating call through the typed `abi_import!`-generated SDK. `delegate_call` and `instantiate` always require `&mut`. Storage helpers (`Lazy`, `Mapping`) similarly gate `set`/`insert` on `&mut self`.

2. **Runtime (contract-side)** — non-payable methods (`pure`/`view`/`nonpayable`) get an injected `__pvm_assert_non_payable` / `__pvm_assert_value_zero` guard at the dispatch entry; the contract reverts if `msg.value > 0`.

3. **Runtime (host-side)** — `pallet-revive` enforces the STATICCALL boundary: state-mutating host calls revert when invoked inside a static frame. This is what backstops `view`/`pure` for cross-contract callers.

**Honest caveat:** the typed-API gate covers cross-contract calls made through `abi_import!`-generated wrappers and storage operations through `pvm-storage`. Raw `pallet_revive_uapi` calls (e.g., `api::set_storage`) bypass the type-level check — only the host's STATICCALL enforcement and the runtime payable guard apply there. Use the typed APIs as the primary surface; reach for raw uAPI only when the typed surface lacks coverage.

**Pure semantics (matches Solidity, by design):** a pure method has no receiver and therefore no `host` accessor. By construction it cannot:
- make cross-contract calls (no `&impl ContractContext` to pass to `CallBuilder::call`),
- read block/chain/tx context (`block.number`, `chain.id`, etc.),
- call host-routed helpers (`keccak256`, event emission, storage),
- emit events.

This matches Solidity's `pure` rules — solc rejects the same operations in a `pure` function. If a method needs `keccak256`, block context, or any host call, mark it `view` (`&self`) rather than pure. The restriction isn't a SDK limitation; it's the same semantic boundary Solidity callers expect when they see `pure` in the ABI.

**Reentrancy non-protection:** `&mut self` enforces single-threaded mutation within a frame, but persistent storage is shared across reentrant frames (each callee gets a fresh contract struct, so the borrow checker offers no cross-frame guarantee). A reentrancy-sensitive method needs an explicit guard (not provided by the SDK yet).

### Fallback and Receive Handlers

- `#[fallback]` — invoked when no method selector matches (or calldata is 1..=3 bytes). Non-payable by default; add `#[payable]` to accept value.
- `#[receive]` — invoked on plain value transfers (empty calldata). Must take `&mut self` and no other arguments. Implicitly payable (mirrors Solidity's `receive() external payable`); `#[payable]` is rejected as redundant. Return type must be `()` or `Result<(), E>`.

When both are present, receive fires first on empty calldata; fallback handles non-empty calldata that doesn't match a selector. Contracts without `#[receive]` pay zero bytecode cost — the empty-calldata branch is omitted entirely.

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
    fn error_signatures() -> impl Iterator<Item = &'static &'static str>;   // for ABI JSON generation
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

The `pvm-storage` crate provides typed storage helpers with Solidity-compatible slot layout.

### Storage Types

| Type | Description |
|------|-------------|
| `Lazy<T>` | Single value at a fixed slot. `get(&self) -> T`, `set(&mut self, &T)`, `try_get(&self) -> Option<T>`, `clear(&mut self)` |
| `Mapping<K, V>` | Key-value mapping. `get(&self, &K) -> V`, `insert(&mut self, &K, &V)`, `entry(&mut self, &K) -> Lazy<V>`, `remove(&mut self, &K)` |

- Currently supports 32-byte types only (U256, Address, bool, `[u8; 32]`); variable-size values are future work
- Solidity-compatible key derivation: `keccak256(pad32(key) ++ pad32(slot))`
- `set(&mut self)` / `insert(&mut self)` / `entry(&mut self)` take `&mut self` for future view enforcement
- `Mapping::entry()` returns a `Lazy<V>` handle for the derived slot, allowing read-then-write on the same key with a single keccak derivation instead of two
- Nested mappings via chaining: `self.allowances.get(&owner).get(&spender)`

### Storage on the contract struct

Declare `#[slot(N)]` fields directly on the contract struct. The `#[contract]` macro constructs each field with a `StorageKey` and a clone of the host handle. Methods access storage via `self`:

```rust
#[contract("MyToken.sol")]
mod my_token {
    pub struct MyToken {
        #[slot(0)]
        total_supply: Lazy<U256>,
        #[slot(1)]
        balances: Mapping<Address, U256>,
        #[slot(2)]
        allowances: Mapping<Address, Mapping<Address, U256>>,
    }

    impl MyToken {
        #[method]
        pub fn balance_of(&self, account: Address) -> U256 {
            self.balances.get(&account)
        }

        #[method]
        pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
            let caller = self.caller();
            let mut cell = self.balances.entry(&caller);
            let bal = cell.get();
            if bal < amount {
                return Err(InsufficientBalance { required: amount, available: bal }.into());
            }
            cell.set(&(bal - amount));
            self.balances.insert(&to, &(self.balances.get(&to) + amount));
            Ok(())
        }
    }
}
```

View enforcement comes from Rust's borrow checker: `&self` methods can only call `get()` on storage fields, while `&mut self` methods can also call `set()`, `insert()`, and `entry()`.

The `host` field name is reserved. The macro injects it automatically.

### Bytecode Optimization

Storage uses type-erased inner functions that operate on raw `[u8; 32]` arrays so the host-call logic is shared across all `Lazy`/`Mapping` instantiations. Benchmarked with/without `#[inline(never)]`: letting the compiler decide produced smaller `.polkavm` output (4,523 vs 4,978 bytes for mytoken), so `#[inline(never)]` is omitted. Contracts that don't use `pvm-storage` pay zero bytes.

### Raw Host Calls

For advanced use cases, raw host calls are still available through `PolkaVmHost`:

```rust
PolkaVmHost::get_storage_or_zero(StorageFlags::empty(), &key, &mut output)
PolkaVmHost::set_storage_or_clear(StorageFlags::empty(), &key, &data)
```

## Reentrancy Protection

pallet-revive rejects reentrant calls by default. When contract A calls contract B, B (and its callees) cannot call back into A. The runtime returns `ReentranceDenied` if reentrancy is attempted.

Two modes are available to contracts:
- **Default** (`CallFlags::empty()`): callee and its recursive callees cannot re-enter the caller.
- **AllowReentry** (`CallFlags::ALLOW_REENTRY`): no restriction, callee can call back freely.

### Macro path (abi_import / CallBuilder)

```rust
// Default: reentrancy denied
let result = foo.bar().call(self.host())?;

// Opt in to reentrancy
let result = foo.bar().allow_reentry().call(self.host())?;
```

### DSL path (raw host calls)

```rust
host.call_evm(
    CallFlags::ALLOW_REENTRY,
    &callee_address,
    gas_limit,
    &value,
    &calldata,
    Some(&mut output),
)?;
```

**Security: do not enable `ALLOW_REENTRY` unless the contract is specifically designed to handle reentrant callbacks** (e.g., flash loans, ERC-777 hooks). Reentrancy is one of the most exploited vulnerability classes in smart contracts. The default protection exists to prevent the classic attack where a callee re-enters the caller before state updates are complete. PVM creates fresh memory per call, so in-memory state is not shared across reentrant invocations. On-chain storage is shared.

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

Seven MyToken variants as separate binaries:

- `example-mytoken-macro-pico-alloc` — `pvm_contract_macros` with `allocator = "pico"`
- `example-mytoken-macro-bump-alloc` — `pvm_contract_macros` with `allocator = "bump"`
- `example-mytoken-macro-no-alloc` — `pvm_contract_macros` default stack mode
- `example-mytoken-macro-no-sol` — `pvm_contract_macros` without Solidity interface path
- `example-mytoken-macro-storage` — `pvm_contract_macros` with `pvm-storage` (`Lazy`, `Mapping`, `#[slot(N)]` on contract struct)
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
cargo pvm-contract build
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
  pvm-contract-sdk/              Primary user-facing SDK crate (re-exports macros, types, polkavm-derive)
  pvm-contract-core/             Core structures for the PVM smart contracts SDK
  pvm-contract-types/           ABI encoding/decoding traits
    src/lib.rs                  SolEncode, SolDecode, StaticEncodedLen + primitive impls
    src/alloc_types.rs          String, Vec<T> impls (alloc feature)
  pvm-storage/                  Typed storage helpers (Lazy, Mapping)
  pvm-contract-builder-dsl/     Runtime dispatch DSL
  pvm-contract-benchmarks/      Binary size CI regression tool
  pvm-contract-e2e-tests/       E2E + integration test harness
examples/
  example-mytoken/              6 MyToken variants
  test-contracts/               9+ test contracts with .sol interfaces
specs/
  abi.md                        ABI encoding specification (includes error encoding)
  builder-dsl.md                Builder DSL specification
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

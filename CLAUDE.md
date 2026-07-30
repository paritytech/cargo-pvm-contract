# cargo-pvm-contract

Cargo subcommand and toolchain for building Rust smart contracts targeting PolkaVM (used by Polkadot's `pallet-revive`). Scaffolds projects from Solidity `.sol` interfaces, generates ABI encoding/decoding, and compiles to `.polkavm` bytecode.

## Crate Overview

| Crate | Description |
|-------|-------------|
| `cargo-pvm-contract` | CLI tool — scaffolds contract projects from `.sol` files |
| `cargo-pvm-contract-builder` | Build library — links PolkaVM bytecode and generates ABI JSON (used by CLI and optional `build.rs`) |
| `pvm-contract-sdk` | Primary user-facing SDK crate — re-exports macros, types, and polkavm-derive for contract development |
| `pvm-contract-core` | Core structures for the PVM smart contracts SDK |
| `pvm-contract-macros` | Proc macros — `#[contract]`, `#[method]`, `#[selector]`, `#[payable]`, `#[constructor]`, `#[fallback]`, `#[receive]`, `#[storage]`, `#[interface_id]`, `abi_import!`, `#[derive(SolType)]`, `#[derive(SolStorage)]`, `#[derive(SolError)]`, `#[derive(SolEvent)]` |
| `pvm-contract-types` | ABI encoding/decoding traits (`SolEncode`, `SolDecode`), error trait (`SolError`) — `no_std` compatible |
| `pvm-storage` | Typed storage helpers — `Lazy<T>`, `Mapping<K, V>`, Solidity-compatible slot layout |
| `pvm-contract-builder-dsl` | Builder-pattern DSL for contracts without proc macros |
| `cargo-pvm-contract-extrinsics` | Library defining extrinsics for PVM smart contracts on pallet-revive |
| `pvm-bump-allocator` | Simple bump allocator for PVM smart contracts (backs `allocator = "bump"`) |
| `pvm-contract-benchmarks` | Binary size comparison tool for CI regression detection |
| `pvm-contract-e2e-tests` | End-to-end + integration test harness |
| `pvm-solc-differential` | Differential tests of on-chain storage representation vs real solc (executed on `revm`); `solc-tests` feature |

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
    |     Output: target/{profile}/{binary}.polkavm
    |
    +-- ABI generation (parse .sol or run with --features abi-gen)
          Output: target/{profile}/{binary}.abi.json
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

The macro injects a `pub host: Host` field on the storage struct and a `fn host(&self) -> &Host` accessor. `Host` is a cfg-gated wrapper: zero-sized type over `PolkaVmHost` on riscv64; `Rc<dyn HostApi>` on host-target builds so it can be cheaply cloned into helpers like `Lazy`/`Mapping`, and tests can construct the contract with a `MockHost`. On host targets the macro also emits a `Foo::with_host(backend: impl HostApi)` test constructor that wires up the storage fields against the backend without running the `#[constructor]` (seed state on the backend directly).

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
- **`call()`** — reads calldata, extracts 4-byte selector, dispatches via `route()` to the matching `#[method]`, then lowers `route()`'s returned `Outcome` to the host through a single `finalize_outcome` exit. When `call_data_len == 0` and a `#[receive]` handler is present, the receive arm fires before the selector dispatch. When the selector matches no method (`Outcome::Unhandled`, or calldata is 1..=3 bytes), control falls through to `#[fallback]` if present, else reverts.

Dispatch uses an "Outcome-in-route" model: arms don't call the host on the success path. `route(this, selector, input, out) -> Outcome` (generic over the `OutSink` output buffer) has each arm validate input size -> decode parameters via `SolDecode` -> call the user function -> encode the return via `SolEncode` into the caller-owned buffer `out`, and return `Outcome::Return(len)`; unmatched selectors return `Outcome::Unhandled`. The single `finalize_outcome` call maps `Return` to the `return_value` success door (`Unhandled` falls through to fallback/revert). **Reverts never flow through `Outcome`** — a method's own `Err(e)` (encoded via `SolError::encode_to` into `out`) and every framework abort (input size check, malformed-calldata decode, payable guard, storage `panic_revert`) all diverge directly via `Host::revert` (`-> !`, `REVERT` flags). So `Outcome` has just two variants (`Return`, `Unhandled`), there's one revert path, and one test idiom (`expect_revert`/`assert_reverts!`) for every revert. The no-alloc `OutSink` is a fixed `&mut [u8]` sized to `MAX_RETURN_LEN`; the alloc one is a stack buffer with a `Vec` spill for dynamic returns.

Selectors are Keccak-256 of the canonical Solidity signature (first 4 bytes), computed at compile time.

### Contract Attribute Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `"path.sol"` | none | Solidity interface file (validates all functions are implemented) |
| `buffer = N` | 256 | Stack calldata buffer size (no-alloc mode) |
| `allocator = "pico"` | none | Use picoalloc heap allocator (enables dynamic types in returns) |
| `allocator = "bump"` | none | Use bump allocator |
| `allocator_size = N` | 1024 | Heap size in bytes for allocator modes |
| `implements(ITrait, ...)` | none | Fold the methods of each in-module `impl ITrait for Contract` block into dispatch as real entry points (see [Interface Composition](#interface-composition)). Optional per-trait `<Error = Ty>` binds the trait's associated error. |

### Method Attribute

- `#[method]` — marks a public function as a contract method
- `#[selector(name = "name")]` — overrides the Solidity function name (default: snake_case to camelCase); `#[method(rename = "name")]` is a supported alias
- `#[payable]` — marks the method as `payable` (must be combined with `&mut self`)
- `#[non_reentrant]` — emits an OpenZeppelin-compatible reentrancy guard (see [Reentrancy Protection](#reentrancy-protection)). Only valid on a `&self` / `&mut self` `#[method]` (not pure, constructor, fallback, or receive)

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

1. **Compile-time (typed-API)** — `#[contract]` auto-implements `ContractContext` on the storage struct (and forbids `#[derive(Clone)]` on it). Cross-contract call builders take `&impl ContractContext` for `view`/`pure` callees and `&mut impl ContractContext` for `nonpayable`/`payable` callees, so a `&self` (view) method *cannot* initiate a state-mutating call through the typed `abi_import!`-generated SDK. `delegate_call` and `instantiate` always require `&mut`. Storage helpers (`Lazy`, `Mapping`) similarly gate `set`/`insert` on `&mut self` against the macro-injected fields. To prevent a view method from sidestepping that by reconstructing a fresh writable handle from `self.host().clone()` and a derived `StorageKey`, `Lazy::new` and `Mapping::new` are marked `unsafe fn` — the macro-driven `StorageComponent::new_at` path stays safe, but direct user construction must opt in to `unsafe` (and is therefore refused by `#![forbid(unsafe_code)]`).

2. **Runtime (contract-side)** — non-payable methods (`pure`/`view`/`nonpayable`) get an injected `__pvm_assert_non_payable` / `__pvm_assert_value_zero` guard at the dispatch entry; the contract reverts if `msg.value > 0`.

3. **Runtime (host-side)** — `pallet-revive` enforces the STATICCALL boundary: state-mutating host calls revert when invoked inside a static frame. This is what backstops `view`/`pure` for cross-contract callers.

**For belt-and-braces enforcement:** add `#![forbid(unsafe_code)]` at your contract crate root. That closes the `unsafe { Lazy::new(...) }` / `unsafe { Mapping::new(...) }` reconstruction bypass at compile time — the macro's own `unsafe` blocks live inside `pvm-storage`'s `StorageComponent::new_at` impls (a separate crate), so the gate doesn't break macro expansion.

**Honest caveat:** the typed-API gate covers cross-contract calls made through `abi_import!`-generated wrappers and storage operations through `pvm-storage`. Raw `pallet_revive_uapi` calls (e.g., `api::set_storage`, `host.set_storage(...)`, `Host::new()`) bypass the type-level check — only the host's STATICCALL enforcement and the runtime payable guard apply there. Use the typed APIs as the primary surface; reach for raw uAPI (or the DSL) only when the typed surface lacks coverage. `forbid(unsafe_code)` does not gate raw uAPI because those calls are themselves safe.

**Pure semantics (matches Solidity, by design):** a pure method has no receiver and therefore no `host` accessor. By construction it cannot:
- make cross-contract calls (no `&impl ContractContext` to pass to `CallBuilder::call`),
- read block/chain/tx context (`block.number`, `chain.id`, etc.),
- call host-routed helpers (`keccak256`, event emission, storage),
- emit events.

This matches Solidity's `pure` rules — solc rejects the same operations in a `pure` function. If a method needs `keccak256`, block context, or any host call, mark it `view` (`&self`) rather than pure. The restriction isn't a SDK limitation; it's the same semantic boundary Solidity callers expect when they see `pure` in the ABI.

**Reentrancy non-protection:** `&mut self` enforces single-threaded mutation within a frame, but persistent storage is shared across reentrant frames (each callee gets a fresh contract struct, so the borrow checker offers no cross-frame guarantee). pallet-revive rejects reentrancy by default; a contract that opts into `ALLOW_REENTRY` and needs an explicit guard can use the `#[non_reentrant]` modifier (see [Reentrancy Protection](#reentrancy-protection)).

### Fallback and Receive Handlers

- `#[fallback]` — invoked when no method selector matches (or calldata is 1..=3 bytes). Non-payable by default; add `#[payable]` to accept value.
- `#[receive]` — invoked on plain value transfers (empty calldata). Must take `&mut self` and no other arguments. Implicitly payable (mirrors Solidity's `receive() external payable`); `#[payable]` is rejected as redundant. Return type must be `()` or `Result<(), E>`.

When both are present, receive fires first on empty calldata; fallback handles non-empty calldata that doesn't match a selector. Contracts without `#[receive]` pay zero bytecode cost — the empty-calldata branch is omitted entirely.

## Interface Composition

`#[contract(implements(ITrait, ...))]` folds the methods of each in-module `impl ITrait for Contract` block into the dispatch table as real entry points. This replaces the pattern of hand-writing one inherent `#[method]` forwarder per interface function: the forwarders move into a compiler-checked `impl ITrait for Contract`, and an override is just a different impl body. Modeled on the interface composition in OpenZeppelin's Contracts for Stylus (`#[public]` + `#[implements(...)]`).

```rust
#[contract(implements(IErc20, IErc165))]
mod my_token {
    pub struct MyToken { pub erc20: Erc20State }
    impl MyToken { #[constructor] pub fn new(&mut self) {} }   // only genuinely-custom methods

    impl IErc20 for MyToken {
        fn total_supply(&self) -> U256 { self.erc20._total_supply() }
        fn transfer(&mut self, to: Address, v: U256) -> Result<bool, Error> {
            self.erc20._transfer(self.caller(), to, v)          // an override is just a different body
        }
        // ... rest of the interface
    }
    impl IErc165 for MyToken { /* the ERC-165 3-liner below */ }
}
```

Mechanics:

- **How folding works.** Each folded method becomes one dispatch arm that calls the method through a fully-qualified trait call `<MyToken as IErc20>::transfer(this, ...)`. This runs the contract's own trait-impl body, so an override just works without any `virtual`/`override` keyword. An inherent method of the same name cannot shadow it. All methods share one flat `match`, keeping bytecode small. Dispatch is static, so interface traits are never used as `dyn` trait objects. This is what lets `#[interface_id]` traits serve as interfaces at all, since their associated constant makes them non-object-safe.
- **Only methods the impl writes are folded.** A folded entry point exists only for a method the `impl` block spells out. A trait method that has a **default body** but isn't restated in the impl is *not* a dispatch entry point: a `.sol` interface flags it through the missing-implementation check, while **without** a `.sol` it is silently absent from both dispatch and the ABI, so restate it in the impl to expose it. Only the traits named in `implements(...)` are folded; an `impl OtherTrait for Contract` you don't list stays an ordinary trait impl. Lifecycle attributes (`#[constructor]` / `#[fallback]` / `#[receive]`) are rejected on a folded method (it always dispatches as an ordinary method) — declare those as inherent `impl` methods on the contract struct.
- **Matched by the path you write.** The match compares the segments in `implements(...)` against the tail of each impl's trait path: a bare `implements(IErc20)` matches `impl IErc20`, `impl crate::x::IErc20`, or `impl super::IErc20`, while a qualified `implements(a::IErc20)` matches only a path ending in `a::IErc20` — never a different `b::IErc20`. (An associated `Error` type is bound in the impl body as `type Error = E;`, not on the impl header.) If two distinct traits in the module share the matched suffix, the match is ambiguous and rejected. A same-trait `impl` targeting a *different* struct in the module is skipped, so the contract's own impl wins regardless of declaration order; if no impl targets the contract struct, that's an error. The impl must be non-generic and carry no `where` clause. Folded methods must take a receiver and have concrete (non-`Self::_`) parameter types.
- **Mutability** is inferred from the receiver exactly as for inherent methods. Per-method attributes go on the **impl fn** and behave as on the inherent path: `#[payable]` (payability isn't part of the Rust signature, so the trait can't carry it), `#[non_reentrant]` (folded methods always have a receiver, so the reentrancy guard applies just as it does to a `#[method]`), and `#[selector(name = "...")]`. With a `.sol` interface, a folded method's payability is cross-checked against the `.sol` declaration.
- **`<Error = Ty>` binding.** For a folded method returning `Result<_, Self::Error>`, the macro can't resolve `Self::Error` from the impl block, so `implements(IErc20<Error = MyError>)` names the concrete error for the ABI. A method that writes its error type concretely needs no binding. The binding is checked against the impl's actual `type Error` by a zero-cost const assertion, so a binding that disagrees with the impl is a compile error rather than an ABI that advertises one error type while the runtime encodes another.
- **Selector-collision detection.** A const-eval guard rejects any two dispatched methods (inherent vs folded, or two folded) sharing a 4-byte selector with a hard compile error. Rename one with `#[selector(name = "...")]` to fix this.
- **Order.** Inherent `#[method]`s dispatch and appear in the ABI first, then folded methods in `implements(...)` order then declaration order — so `abi.json` is stable.

### ERC-165 pattern (opt-in, no generated code)

ERC-165 falls out of `#[interface_id]` + `implements(...)`. Define an `IErc165` interface, list it in `implements(...)`, and hand-write the 3-liner using the `INTERFACE_ID` consts. A contract that doesn't add this pays zero bytes.

```rust
impl IErc165 for MyToken {
    fn supports_interface(&self, id: [u8; 4]) -> bool {
        id == [0x01, 0xff, 0xc9, 0xa7]                // ERC-165 itself
            || id == <Self as IErc20>::INTERFACE_ID
    }
}
```

### DSL path (builder-DSL)

The builder-DSL composes via **router chaining** (its dispatch table is runtime, so chaining is the analog of the macro fold). An extension exposes its handlers as a `ContractBuilder`; the contract calls `ContractBuilder::dispatch_composed::<BUF>(&host, &[&extension, ...])`, which tries its own methods first, then each extension's non-terminating `try_route` in order (first match wins). Selector uniqueness within one builder is enforced at registration — `method`/`payable_method` **panic on a duplicate selector** (the DSL's runtime analog of the macro's compile-time collision check).

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
    const HEAD_SIZE: usize = 32;   // 32 for primitives, sum of fields for structs
    const SLOT_SIZE: usize;        // HEAD_SIZE for static, 32 for dynamic (default)
    const IS_TUPLE: bool = false;  // true only for Rust tuples (T1, T2, ...)
    fn encode_body_len(&self) -> usize;  // field body size
    fn encode_body_to(&self, buf: &mut [u8]);  // field body encoding
    fn encode_len(&self) -> usize;   // top-level size (default, IS_TUPLE/IS_DYNAMIC aware)
    fn encode_to(&self, buf: &mut [u8]);  // top-level encoding (default, smart wrapping)
    fn indexed_topic(&self) -> [u8; 32];  // default: event-topic encoding for indexed params
    // #[cfg(feature = "abi-gen")]
    fn abi_param(name: &str) -> AbiParam;  // ABI JSON parameter descriptor
}

pub trait SolDecode: SolEncode + Sized {
    fn decode(input: &[u8]) -> Result<Self, DecodeError>;             // default
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError>;  // required
    fn decode_tail(input: &[u8], offset: usize) -> Result<Self, DecodeError>;  // default
}

pub trait StaticEncodedLen: SolEncode + Sized {
    const ENCODED_SIZE: usize;  // compile-time known size, used for stack buffers
}

// No-alloc fast-path decoder used by the dispatch codegen for static types.
pub trait StaticDecode: SolDecode + SolEncode + StaticEncodedLen + Sized {
    unsafe fn decode_unchecked(input: &[u8], offset: usize) -> Self;
}
```

### Error Traits (`pvm-contract-types`)

```rust
pub trait SolError: Sized {
    const SELECTOR: [u8; 4];       // keccak256 of canonical signature, first 4 bytes (zeroed for enums)
    const SIGNATURE: &'static str; // e.g. "InsufficientBalance(address,uint256,uint256)" (empty for enums)
    fn encoded_size(&self) -> usize;                   // 4 + encoded params size
    fn encode_to(&self, buf: &mut [u8]) -> usize;      // selector + ABI-encoded fields; returns bytes written
    fn decode_at(input: &[u8], offset: usize) -> Result<Option<Self>, DecodeError>; // symmetric decoder
    // #[cfg(feature = "abi-gen")]
    fn error_signatures() -> impl Iterator<Item = &'static &'static str>; // all signatures, for ABI JSON
}
```

- `SolError` — one unified trait, derived with `#[derive(SolError)]` on both error structs and error enums.
  - On a **struct**: single selector, `encode_to` writes selector + fields, `decode_at` is the inverse.
  - On an **enum** whose variants each wrap one `SolError` struct: the derive emits a `From<Inner>` impl for each variant's inner error type (so `Err(InsufficientBalance { .. }.into())` works), and dispatches `encode_to`/`decode_at`/`error_signatures` to whichever variant the value currently holds. The enum's own `SELECTOR` is zeroed and `SIGNATURE` empty — the wire selector is always the held inner error's. To surface require-style messages or arithmetic panics, add `RevertString` / `Panic` as explicit variants of your enum.
- `RevertString` — encodes `Error(string)` with truncation for buffer safety.
- `Panic` — encodes `Panic(uint256)` for overflow/division-by-zero.
- `EmptyError` — zero-cost uninhabited type for contracts with no error paths.

### Scaffolder type mapping

The scaffolder (`cargo pvm-contract init --init-type new --sol-file Foo.sol`) maps Solidity ABI types to SDK types via `solidity_to_rust_type` in `crates/cargo-pvm-contract/src/scaffold.rs`. Unrecognized or unsupported Solidity types (tuples, non-canonical numeric widths, malformed type names) are rejected at scaffold time with `error: unsupported Solidity type: "X"` rather than silently substituting a default. If you hit this, the type isn't yet supported — file an issue, edit the generated file manually, or use a non-tuple parameter shape.

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
| `bytes` | `Bytes` | yes | yes | alloc feature | Newtype around `Vec<u8>`. `Vec<u8>` is reserved for `uint8[]` — same Rust shape, different on-chain layout. |
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

`#[derive(SolType)]` covers ABI-only use (function-parameter / event-field structs). A struct used as a **storage value** (`Lazy<S>`, `Mapping<_, S>`) must *additionally* derive `#[derive(SolStorage)]`, which provides `StorageEncode`/`StorageDecode` and the abi-gen `StorageTypeName` layout-naming trait. See the Storage section.

## Storage

The `pvm-storage` crate provides typed storage helpers with Solidity-compatible slot layout.

### Storage Types

| Type | Description |
|------|-------------|
| `Lazy<T>` | Single value at a fixed slot. `get(&self) -> T`, `set(&mut self, &T)`, `try_get(&self) -> Option<T>`, `clear(&mut self)` |
| `Mapping<K, V>` | Key-value mapping. `get(&self, &K) -> V`, `insert(&mut self, &K, &V)`, `entry(&mut self, &K) -> Lazy<V>`, `remove(&mut self, &K)` |
| `StorageVec<T>` | Dynamic array (Solidity `T[]`). Read: `len`, `is_empty`, `get(i) -> T` (panics OOB) / `try_get(i) -> Option<T>`, `first`/`last`, `iter`. Write: `push(&T)`, `pop() -> Option<T>`, `set(i, &T)`, `clear`. OOB `get`/`set` revert with solc's ABI-encoded `Panic(0x32)` (array out-of-bounds); use `try_get` to avoid it |

- Supports static values up to `MAX_STATIC_SLOTS` * 32 bytes (single-word and multi-word static structs/tuples) and dynamic values (`String`, `Bytes`, `#[derive(SolType, SolStorage)]` structs with dynamic fields) using solc's inline/spilled `bytes`/`string` layout
- **Custom struct as storage value:** structs that live in `Lazy<S>` / `Mapping<_, S>` must derive **both** `SolType` (for ABI / field-layout signature) **and** `SolStorage` (for `StorageEncode`/`StorageDecode` + the `StaticStorageEncode`/`StaticStorageDecode` refinement when fully static). `SolType` alone is sufficient for ABI-only types (function parameter structs, event field structs). Deriving `SolStorage` on a struct with a non-storage-compatible field (e.g. `Vec<U256>`, nested SolType structs, tuples, fixed arrays of non-`u8`) emits a `compile_error!` at expansion time — visible to `cargo check` and `trybuild`
- Fixed-size arrays `[T; N]` are supported as storage values (Solidity `T[N]`), striped across consecutive slots. The element `T` must be a static, non-dynamic-body type — enforced at compile time via the `StorageArrayElement` marker and a `!T::HAS_DYNAMIC_BODY` const-assert (so e.g. `[String; N]` is rejected; use `StorageVec<T>` for dynamic-length or dynamic-element collections)
- `Vec<u8>` is rejected as a storage value — its ABI name is `"uint8[]"`, a different on-chain layout from Solidity `bytes`; use `Bytes` for `bytes`-shaped storage. `Vec<u8>` is still valid as an ABI parameter and as a mapping key
- Solidity-compatible key derivation: `keccak256(pad32(key) ++ pad32(slot))`
- `set(&mut self)` / `insert(&mut self)` / `entry(&mut self)` take `&mut self` for future view enforcement
- `Mapping::entry()` returns a `Lazy<V>` handle for the derived slot, allowing read-then-write on the same key with a single keccak derivation instead of two
- Nested mappings via chaining: `self.allowances.get(&owner).get(&spender)`
- **Nested / composite collections.** Because an inner collection is a handle (not a `StorageEncode` value), the nested accessors hand out borrow guards rather than values, gating mutation through the parent borrow:
  - `Mapping<K, StorageVec<T>>` (Solidity `mapping(K => T[])`): `get(&K) -> Ref<StorageVec<T>>` (read) / `entry(&K) -> RefMut<StorageVec<T>>` (write), then operate on the inner vec — `self.posts.entry(&author).push(&post)`
  - `StorageVec<StorageVec<T>>` (Solidity `T[][]`): `len`/`get(i) -> Ref<…>`/`try_get`/`first`/`last`/`iter` for reads; `grow() -> RefMut<…>` appends an empty inner row, `entry(i) -> RefMut<…>` mutates an existing one, and `erase_last() -> bool` drops the last row (the inner vec can't be returned by value, so it's destroyed rather than popped)
  - These shapes are supported via dedicated impls today; arbitrary nesting (3+ levels, `StorageVec<Mapping<…>>`) awaits the planned `StorageType` unification
- **Composability:** `#[storage]` structs auto-implement `StorageComponent` (slot reservation) and, under `--features abi-gen`, `StorageLayoutEmit` (so the outer contract flattens their leaves into the `storageLayout` JSON with dotted labels like `erc20.total_supply`). Layout emission is **uniform with no syntactic type-name special-casing**: every storage field — `Lazy<T>`, `Mapping<K, V>`, `StorageVec<T>`, and embedded `#[storage]` sub-structs — dispatches through `<#ty as StorageLayoutEmit>::emit_entries(base, offset, name_prefix, entries)`, the single source of truth for both a leaf's layout and its `type` string. The built-in leaves implement **both** `StorageComponent` and `StorageLayoutEmit`, plus `StorageTypeName` for the `type` name (`Lazy<T>` → `T`'s name, `Mapping<K, V>` → `mapping(K => V)`, `StorageVec<T>` → `T[]`, `[T; N]` → `T[N]`); there is no `is_layout_leaf`/`sol_storage_type_name` resolver. A hand-rolled composite component just implements the same two traits to flatten its leaves. `StorageComponent::PACKED_BYTES = 32` declares "always start a fresh slot" — mappings, multi-slot composites, and embedded `#[storage]` sub-structs all set this; sub-32-byte primitives propagate `T::PACKED_BYTES`
- **Field-level packing:** adjacent sub-32-byte contract fields share a slot byte-for-byte with solc's layout. For `Lazy<u128> a; Lazy<u128> b;`, `a` occupies the low-order 16 bytes and `b` the high-order 16 bytes (solc's lower-order-first packing). The macro walker `layout_step` (defined in `pvm-contract-types`, re-exported by `pvm-storage`) is the const-fn computing each placement; it tracks an internal big-endian offset (`a` → 16, `b` → 0) used as the read-modify-write window in `Lazy::set/get`. The `storageLayout` JSON converts this to solc's convention — `offset` counted from the least-significant byte — so the emitted layout matches solc exactly (`a` → offset 0, `b` → offset 16). Packed writes are read-modify-write (one SLOAD + one SSTORE), matching solc's gas profile
- **`try_get` is full-slot only:** `Lazy::<T>::try_get` is rejected at compile time for sub-32-byte `T` (e.g. `Lazy<u128>`) with a const-assert message — a neighbour's write to the same slot would make `try_get` indistinguishable from `get`. For packed fields, use `.get()` and compare to the zero value of `T`
- **Test-harness contract modules:** `#[contract(no_main)]` suppresses the abi-gen `fn main()` emission so a `#[contract]` can sit inside a `tests/` integration test or library crate without colliding with the test harness's own `main`. `__abi_json()` / `__storage_layout_json()` accessors are still emitted on the module

### Storage on the contract struct

Declare storage fields directly on the contract struct. Three modes:

- **Auto-numbering (default).** Drop the `#[slot]` attribute and let the macro assign slots in declaration order via `layout_step`. Sub-word siblings pack into the same slot solc-style (`Lazy<u32>` at byte 28; adjacent `Lazy<bool>` at byte 27, sharing slot 0). Accepts every storage type.
- **Explicit pinning (`#[slot(N)]`).** Restricted to full-slot types — `Mapping`, `Lazy<U256>`, `Lazy<String>`, `Lazy<Bytes>`, multi-slot composites like `Lazy<(U256, U256)>`, and `#[storage]` sub-structs (anything with `StorageComponent::PACKED_BYTES == 32`). Sub-word types are rejected at compile time (explicit mode would place them at byte 0 of the slot while solc places them right-aligned). Use auto-numbering for sub-word packing or wrap the field in a `#[storage]` sub-struct if you need to pin the group at a specific slot. The primary reason to reach for `#[slot(N)]` over auto-numbering is `#[cfg(...)]`-gated storage variants — auto-numbered fields can't carry `#[cfg]` because that would shift later slot indices based on the active feature set.
- **Raw external slots (`#[slot(raw = KEY)]`).** Bind a typed field to a fixed, externally-known 32-byte slot (`KEY: [u8; 32]`) that lives *outside* the compiler-assigned sequential range — e.g. an [EIP-1967](https://eips.ethereum.org/EIPS/eip-1967) proxy slot (`keccak256("eip1967.proxy.implementation") - 1`). Because it's a real typed field, `get`/`set` stay gated by the borrow checker's `&self`/`&mut self` (view-vs-mutating) rule — no raw `host.get_storage` call and no `unsafe`. Sub-word types are **accepted** here (unlike `#[slot(N)]`) and placed right-aligned at `offset = 32 - PACKED_BYTES`, matching solc, so a slot written by a real Solidity proxy decodes identically. The field binds via `StorageComponent::new_at(StorageKey::from_raw(KEY), …)` with `alone = true` (an external pseudo-random slot has no packing neighbours). Raw slots are omitted from the abi-gen `storageLayout` (solc doesn't emit them either). Define the `KEY` constant in the contract/example, not the SDK. Example:
  ```rust
  const IMPLEMENTATION_SLOT: [u8; 32] = /* keccak256("eip1967.proxy.implementation") - 1 */;

  pub struct Proxy {
      #[slot(raw = IMPLEMENTATION_SLOT)]
      impl_addr: Lazy<Address>,
  }
  ```

**Mixing modes.** Numeric `#[slot(N)]` and auto-numbering are mutually exclusive — either all non-raw fields pin a numeric slot or all are auto-numbered. **`#[slot(raw = KEY)]` is exempt** and may coexist with either mode: its key is pseudo-random and outside the sequential range, so it can't collide with a numeric or auto slot, and auto-numbered siblings keep their solc-style sub-word packing. (The caller owns raw-key correctness — a deliberately colliding `KEY` isn't checked.)

The `#[contract]` macro constructs each field with a `StorageKey` and a clone of the host handle. Methods access storage via `self`:

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

Storage uses type-erased inner functions that operate on raw `[u8; 32]` arrays so the host-call logic is shared across all `Lazy`/`Mapping` instantiations. Benchmarked with/without `#[inline(never)]`: letting the compiler decide produced smaller `.polkavm` output, so `#[inline(never)]` is omitted. Contracts that don't use `pvm-storage` pay zero bytes.

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

### `#[non_reentrant]` modifier

For contracts that opt into `ALLOW_REENTRY` (and thus lose the default runtime reject), `#[non_reentrant]` re-adds an explicit OpenZeppelin-style guard on a method. The mode is inferred from the receiver:

- **`&mut self` — full guard** (OZ `nonReentrant`): reverts if a guarded section is already in progress, otherwise sets a lock for the duration of the call and clears it on return.
- **`&self` — read-only check** (OZ `nonReentrantView`): reverts if a guarded section is in progress; never writes.

```rust
#[method]
#[non_reentrant]
pub fn flash_loan(&mut self, ...) -> Result<(), Error> { /* ... */ }
```

On re-entry a guarded method reverts with the OZ-compatible `ReentrancyGuardReentrantCall` error (selector matches OZ v5), so Foundry/Etherscan decode it. The error is registered in the contract's ABI for guarded methods.

Implementation notes (for maintainers):

- **Opt-in only.** On a contract that never sets `ALLOW_REENTRY`, the guard is redundant with pallet-revive's default reject (pure overhead). Use it only on the `ALLOW_REENTRY` paths. This can't be enforced at compile time: `ALLOW_REENTRY` is a runtime call flag (not a static contract property), and the guarded method often makes no calls itself (a sibling makes the `ALLOW_REENTRY` call while the guarded method is the one re-entered via the shared lock), so the macro can't tell whether the guard is warranted. The redundant case is wasted gas, not a correctness issue.
- **Two different errors.** A reentrant call on a *default* contract reverts with pallet-revive's `ReentranceDenied` (a runtime trap, not ABI-decodable); only the `#[non_reentrant]` path produces the ABI-decodable `ReentrancyGuardReentrantCall`. This differs from OZ, which has a single mechanism. The SDK can't unify them because the default-path error belongs to the runtime.
- **Guard every reachable mutating entry point** during an `ALLOW_REENTRY` window: the guard only protects methods that carry the attribute; an unguarded sibling mutator is a hole. (Internal Rust calls bypass the guard; it protects external dispatch entry points only.)
- **Delegatecall/proxy:** the lock lives in the *current* storage context, so under `delegatecall` it guards the caller's storage. That's the correct EVM behavior, but call it out since proxy + reentrancy is a common confusion.
- **The lock lives in transient storage** (EIP-1153, `StorageFlags::TRANSIENT`) at a fixed namespaced key (`keccak256("pvm.guards.reentrancy")`), outside the declared layout, so it doesn't perturb slot numbering or the `storageLayout` ABI (`pvm-contract-types::reentrancy`). Transient is the right fit: shared across the call stack within a transaction (the re-entrant frame sees the lock), cheaper than a persistent `SSTORE`, and auto-cleared at transaction end, so a stuck lock can't brick the contract across transactions.
- **The lock is cleared explicitly before `return_value`, not via `Drop`.** On-chain `return_value` diverges without unwinding, so a `Drop` guard would never run (it would on host targets, hiding the bug). The explicit clear is still needed with transient storage, which persists across *sequential* calls within a transaction, otherwise a later guarded call in the same tx would revert spuriously (as in OZ's `ReentrancyGuardTransient`).

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

Multi-binary project (19 contracts) for E2E integration tests:

- `flipper` — boolean toggle
- `storage-types` — all primitive type storage roundtrips
- `multi-method` — multiple view + state methods
- `return-values` — tuple returns
- `events` — event emission with indexed params
- `dynamic-types` — String, Vec<u8>, Vec<U256>
- `composite-types` — fixed arrays, tuples
- `constructor-args` — constructor with parameters
- `caller-check` — `api::caller()` access
- `error-handling` — `#[derive(SolError)]` (struct + enum) ABI-encoded revert flow
- `payable` / `receive` / `receive_dsl` — `#[payable]`, `#[receive]`, and DSL receive handlers
- Cross-contract: `flipper_call`, `flipper_delegate`, `flipper_instantiate` (`call`/`delegate_call`/`instantiate` via `abi_import!`), `point_adder` + `point_adder_call` (struct args across a call), `error_caller` (decoding a callee's `SolError` revert)

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
  cargo-pvm-contract-extrinsics/ pallet-revive extrinsic definitions
  pvm-contract-macros/          Proc macros
    src/codegen/contract.rs     #[contract] attribute parsing + module generation
    src/codegen/dispatch.rs     Selector computation + dispatch match arms
    src/codegen/decode.rs       Parameter decoding codegen
    src/codegen/method.rs       #[method]/#[constructor]/#[fallback]/#[receive] parsing
    src/codegen/sol_type.rs     #[derive(SolType)] expansion
    src/codegen/sol_error.rs    #[derive(SolError)] expansion
    src/codegen/sol_event.rs    #[derive(SolEvent)] expansion
    src/codegen/sol_storage.rs  #[storage] / #[derive(SolStorage)] expansion
    src/codegen/storage_layout.rs  storageLayout JSON emit (slots, packing, type names)
    src/codegen/abi_gen.rs      abi-gen main() + __abi_json/__storage_layout_json accessors
    src/abi_import/             abi_import! parsing (parse.rs, ctxt.rs)
    src/signature/types.rs      Rust-to-Solidity type mapping
    src/signature/selector.rs   Keccak-256 selector computation
  pvm-contract-sdk/              Primary user-facing SDK crate (re-exports macros, types, polkavm-derive)
  pvm-contract-core/             Core structures for the PVM smart contracts SDK
  pvm-contract-types/           ABI encoding/decoding traits
    src/lib.rs                  SolEncode, SolDecode, StaticEncodedLen + primitive impls
    src/alloc_types.rs          String, Vec<T> impls (alloc feature)
    src/storage_codec.rs        Storage encode/decode (static slots + dynamic bytes/string)
    src/layout.rs               layout_step / MAX_STATIC_SLOTS field-packing walker
    src/host.rs                 HostApi trait + Host / Context wrappers
    src/i256.rs                 I256 signed 256-bit integer
  pvm-storage/                  Typed storage helpers (Lazy, Mapping, StorageVec)
  pvm-contract-builder-dsl/     Runtime dispatch DSL
  pvm-bump-allocator/           Bump allocator (allocator = "bump")
  pvm-contract-benchmarks/      Binary size CI regression tool
  pvm-contract-e2e-tests/       E2E + integration test harness
examples/
  example-mytoken/              7 MyToken variants
  test-contracts/               19 test contracts with .sol interfaces
specs/
  abi.md                        ABI encoding specification (includes error encoding)
  architecture.md               Architecture overview
  build.md                      Build pipeline
  builder-dsl.md                Builder DSL specification
  cli.md                        CLI reference
  deployment.md                 Deployment
  proc-macros.md                Proc-macro reference
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

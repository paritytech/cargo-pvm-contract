# Proc Macros for PVM Smart Contracts

Annotate a module with `#[contract]` and impl methods with `#[method]`, `#[constructor]`, `#[fallback]`, or `#[receive]`. The macro generates entry points, calldata dispatch, ABI encoding, and (under `--features abi-gen`) the ABI JSON and storage layout JSON.

> The user-facing crate is `pvm_contract_sdk`, which re-exports the macros from `pvm_contract_macros` together with the runtime types (`Lazy`, `Mapping`, `Address`, ABI traits, etc.). The two attribute paths (`#[pvm_contract_sdk::contract]` and `#[pvm_contract_macros::contract]`) are equivalent; the SDK path is preferred in user code.

## Basic Usage

```rust,ignore
#![no_main]
#![no_std]

use pvm_contract_sdk::{Address, Lazy, Mapping};
use ruint::aliases::U256;

#[pvm_contract_sdk::contract("MyToken.sol", allocator = "bump")]
mod my_token {
    use super::*;

    pub struct MyToken {
        total_supply: Lazy<U256>,
        balances: Mapping<Address, U256>,
    }

    impl MyToken {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self, initial: U256) {
            self.total_supply.set(&initial);
            let caller = self.caller();
            self.balances.insert(&caller, &initial);
        }

        #[pvm_contract_sdk::method]
        pub fn total_supply(&self) -> U256 {
            self.total_supply.get()
        }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            self.balances.get(&account)
        }
    }
}
```

The macro reads the `.sol` interface (if provided) to validate that every declared function is implemented and to compute Keccak-256 selectors. The Solidity file is only an interface — no implementation:

```solidity
// MyToken.sol
interface MyToken {
    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
}
```

**Without a `.sol` file** — selectors are inferred from Rust function signatures. Rust `snake_case` is converted to `camelCase` for the Solidity signature.

## Contract Attribute Arguments

| Argument             | Default | Description                                                                                                                       |
| -------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `"path.sol"`         | none    | Solidity interface file (validates every function is implemented and that `stateMutability` agrees with the Rust receiver shape) |
| `buffer = N`         | 256     | Stack calldata buffer size (no-alloc mode)                                                                                        |
| `allocator = "pico"` | none    | Use picoalloc heap allocator (required to *return* dynamic types like `String` / `Vec`)                                           |
| `allocator = "bump"` | none    | Use bump allocator (no free, smaller than picoalloc)                                                                              |
| `allocator_size = N` | 1024    | Heap size in bytes for allocator modes                                                                                            |
| `no_main`            | off     | Suppress the `fn main()` emission so a `#[contract]` can sit inside an integration test or library crate                          |

## Allocator Options

Contracts run in `no_std`. If you need heap allocation (`Vec`, `String`), choose an allocator.

- **No allocator (default).** Stack-only. Calldata is read into a fixed-size buffer. Only static return types allowed. Smallest binary.
- **Bump.** Simple bump allocator from `pvm-bump-allocator`. Never frees. Fine for short-lived contract calls.
- **Pico.** Third-party allocator with actual `free` support. Slightly larger binary.

```rust,ignore
#[pvm_contract_sdk::contract("MyToken.sol", allocator = "pico", allocator_size = 2048)]
mod my_token { ... }
```

## Contract Anatomy

Every PVM contract has two entry points:

```text
deploy()  — called once during contract instantiation (constructor)
call()    — called on every subsequent interaction
```

`deploy()` reads constructor calldata, decodes via `SolDecode`, calls `#[constructor]`, and returns to the caller.

`call()`:

1. Reads calldata via `HostFnImpl::call_data_copy`
2. If calldata is empty and a `#[receive]` handler is present, dispatches there
3. Otherwise extracts the 4-byte selector from `calldata[0..4]`, matches a registered method, decodes parameters via `SolDecode`, calls the user function, and encodes the return via `SolEncode`
4. If the selector matches no method (or calldata is 1..=3 bytes), falls through to `#[fallback]` if present, else reverts
5. If the user function returns `Err(e)`, the error is encoded via `SolError::encode_to` and returned with `REVERT` flags

## Method, Constructor, Fallback, Receive

- `#[method]` — public contract method. Optional `#[method(rename = "name")]` overrides the Solidity name (default: `snake_case` → `camelCase`).
- `#[constructor]` — runs once at deployment. Must take `&mut self`; pure/view constructors are rejected because they cannot initialize storage.
- `#[fallback]` — invoked when no method selector matches (or calldata is 1..=3 bytes).
- `#[receive]` — invoked on plain value transfers (empty calldata). Must take `&mut self` and no other arguments. Implicitly payable; `#[payable]` is rejected as redundant.
- `#[payable]` — marks a method as payable. Must be combined with `&mut self`. Adding it to a no-receiver or `&self` method is a compile error.
- `#[non_reentrant]` — emits an OpenZeppelin-compatible reentrancy guard on a `#[method]`. Mode is inferred from the receiver: `&mut self` gives a full guard (OZ `nonReentrant`), `&self` a read-only check (OZ `nonReentrantView`). On re-entry the method reverts with the `ReentrancyGuardReentrantCall` error (OZ v5 selector). Only valid on a `#[method]` with a receiver — applying it to a pure method, constructor, fallback, or receive handler is a compile error. Only meaningful for contracts that opt into `CallFlags::ALLOW_REENTRY` (pallet-revive rejects reentrancy by default).

When both `#[receive]` and `#[fallback]` are present, `receive` fires first on empty calldata.

### Mutability Inference

Solidity `stateMutability` is inferred from the Rust receiver shape. There is no explicit `#[view]` or `#[pure]` attribute.

| Receiver              | `#[payable]` | ABI emits           |
| --------------------- | ------------ | ------------------- |
| none (`fn foo(args)`) | —            | `pure`              |
| `&self`               | —            | `view`              |
| `&mut self`           | —            | `nonpayable`        |
| `&mut self`           | yes          | `payable`           |
| `&self`               | yes          | **compile error**   |
| no receiver           | yes          | **compile error**   |

If a `.sol` interface is provided, the macro rejects any mismatch between the Rust-inferred mutability and the `.sol` declaration.

## Storage

Storage helpers live in `pvm-storage` (re-exported from `pvm-contract-sdk`). The primary types are `Lazy<T>` (single value at a fixed slot), `Mapping<K, V>` (key-value), and `StorageVec<T>` (dynamic array, Solidity `T[]`). Fixed-size arrays `[T; N]` (Solidity `T[N]`, static element) are supported as values inside any of these.

Declare fields directly on the contract struct. Two layout modes:

- **Auto-numbered (default).** Omit `#[slot]` and the macro assigns slots in declaration order. Sub-word siblings pack into a shared slot solc-style (`Lazy<u32>` at byte 28; adjacent `Lazy<bool>` at byte 27, both in slot 0).
- **Explicit `#[slot(N)]`.** Pins a field at slot `N`. Restricted to full-slot types (`Mapping`, `StorageVec`, `Lazy<U256>`, `Lazy<String>`, multi-slot composites, `#[storage]` sub-structs). Sub-word types are rejected because solc would place them at byte `32 - sizeof(T)`, while explicit-slot mode would place them at byte 0. Mixing the two modes within one struct is not supported.

`#[slot(N)]` is mainly useful when fields need `#[cfg(...)]` gating — auto-numbered fields can't carry `#[cfg]` because that would shift later slot indices based on the active feature set.

```rust,ignore
#[pvm_contract_sdk::contract("MyToken.sol")]
mod my_token {
    use super::*;

    pub struct MyToken {
        total_supply: Lazy<U256>,
        balances: Mapping<Address, U256>,
        allowances: Mapping<Address, Mapping<Address, U256>>,
    }

    impl MyToken {
        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            self.balances.get(&account)
        }

        #[pvm_contract_sdk::method]
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

Mutability gating comes from the borrow checker: `&self` methods can only call read accessors (`get` / `try_get`, plus `len` / `first` / `last` / `iter` on `StorageVec`); `&mut self` can also call mutators (`set`, `insert`, `entry`, `remove`, and `push` / `pop` / `set` / `grow` / `erase_last` on `StorageVec`). To prevent a view method from reconstructing a writable handle from `self.host().clone()` plus a derived `StorageKey`, `Lazy::new`, `Mapping::new`, and `StorageVec::new` are `unsafe fn` — the macro path (`StorageComponent::new_at`) stays safe, and `#![forbid(unsafe_code)]` at the contract crate root closes the reconstruction bypass entirely.

### `#[storage]` Sub-Structs

A `#[storage]`-derived struct is itself a storage component and can be embedded in a contract struct. The auto-numbering walker reserves a contiguous slot range for it:

```rust,ignore
#[pvm_contract_sdk::storage]
pub struct Erc20State {
    pub total_supply: Lazy<U256>,
    pub balances: Mapping<Address, U256>,
    pub allowances: Mapping<Address, Mapping<Address, U256>>,
}

#[pvm_contract_sdk::contract("MyToken.sol")]
mod my_token {
    use super::*;

    pub struct MyToken {
        erc20: Erc20State,     // 3 slots, auto-numbered starting at slot 0
        paused: Lazy<bool>,    // slot 3
    }
    // ...
}
```

Under `--features abi-gen`, embedded `#[storage]` sub-structs flatten into the `storageLayout` JSON with dotted labels (`erc20.total_supply`, `erc20.balances`, …) so `cast storage` and other Solidity tooling can navigate the layout.

`#[storage]` structs may not derive `Clone` (it would let a view method clone the component and obtain a fresh `&mut`), and per-field `#[cfg]` is rejected for the same slot-shifting reason as on the contract struct.

### Dynamic Values

`Lazy<String>`, `Lazy<Bytes>`, and `Mapping<K, V>` with `V = String` / `Bytes` / a `#[derive(SolType)]` struct containing dynamic fields all use solc's inline/spilled `bytes`/`string` storage layout. `Vec<u8>` is rejected as a storage value (its ABI name is `"uint8[]"`, a different on-chain layout) — use `Bytes` for `bytes`-shaped storage; `Vec<u8>` remains valid as an ABI parameter type and as a mapping key.

### Dynamic Arrays (`StorageVec`)

`StorageVec<T>` is a dynamic array with Solidity's `T[]` slot layout (length at the field's slot; elements at `keccak256(slot) + i`). Reads take `&self`, writes `&mut self`:

```rust,ignore
pub struct Registry {
    entries: StorageVec<U256>,                 // T[]
}

impl Registry {
    #[pvm_contract_sdk::method]
    pub fn len(&self) -> u64 {
        self.entries.len()                     // read: len / is_empty / get / try_get / first / last / iter
    }

    #[pvm_contract_sdk::method]
    pub fn add(&mut self, v: U256) {
        self.entries.push(&v);                 // write: push / pop / set(i, &v) / clear
    }
}
```

Out-of-bounds `get` / `set` revert via a plain trap (not solc's ABI-encoded `Panic(0x32)`); use `try_get` for a non-panicking read.

**Nested and composite shapes.** Because an inner collection is a *handle* (not a `StorageEncode` value), the nested accessors return borrow guards (`Ref` / `RefMut`) rather than the inner collection by value — which also enforces the view gate (a `&self` outer can only hand out a read-only `Ref`):

- `Mapping<K, StorageVec<T>>` (`mapping(K => T[])`): `get(&K) -> Ref<StorageVec<T>>` (read) / `entry(&K) -> RefMut<StorageVec<T>>` (write), then operate on the inner vec — `self.posts.entry(&author).push(&post)`.
- `StorageVec<StorageVec<T>>` (`T[][]`): `len` / `get(i) -> Ref<…>` / `try_get` / `first` / `last` / `iter` for reads; `grow() -> RefMut<…>` appends an empty inner row, `entry(i) -> RefMut<…>` mutates an existing one, and `erase_last() -> bool` drops the last row (the inner vec can't be returned by value, so it is destroyed rather than popped).

These shapes are provided by dedicated impls today; arbitrary deeper nesting (3+ levels, `StorageVec<Mapping<…>>`) awaits the planned `StorageType` unification. Under `--features abi-gen`, `StorageVec<T>` is recognized by the macro's layout resolver and named as `T[]` (recursively, so `T[][]` and `mapping(K => T[])` resolve correctly) — it participates through `StorageComponent` alone and does not implement `StorageLayoutEmit`.

### Raw Host Calls

For advanced cases, raw uAPI calls remain available through `PolkaVmHost`:

```rust,ignore
use pvm_contract_sdk::{PolkaVmHost, StorageFlags};

PolkaVmHost::get_storage_or_zero(StorageFlags::empty(), &key, &mut output);
PolkaVmHost::set_storage_or_clear(StorageFlags::empty(), &key, &data);
```

These bypass the typed-storage view enforcement; the host's STATICCALL boundary and the runtime payable guard still apply.

## Error Handling

Error encoding is handled by a single trait, `SolError`, derived with `#[derive(SolError)]`:

- On a **struct**, the derive computes the 4-byte selector (`keccak256` of the canonical signature derived from the struct name and fields) and emits `encode_to` (selector + ABI-encoded fields), `encoded_size`, and `decode_at`.
- On an **enum** whose variants each wrap a single `SolError` struct, the derive emits `From` conversions plus an `encode_to` / `decode_at` / `error_signatures` impl that dispatches to the active variant's inner error. The enum's own `SELECTOR` is zeroed and `SIGNATURE` is empty — the wire selector is always the inner error's. Add explicit `RevertString` / `Panic` variants if you want `require`-style messages or arithmetic panics in the same enum.

```rust,ignore
use pvm_contract_sdk::SolError;

#[derive(SolError)]
pub struct InsufficientBalance {
    pub required: U256,
    pub available: U256,
}

#[derive(SolError)]
pub enum TokenError {
    InsufficientBalance(InsufficientBalance),
}

#[pvm_contract_sdk::method]
pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
    // returning `Err(InsufficientBalance { .. }.into())` reverts with the
    // ABI-encoded `InsufficientBalance(uint256,uint256)` payload that solc and
    // viem decode automatically.
}
```

Infallible methods (return type `T`, not `Result<T, E>`) cannot revert by returning an error. They can still trigger a `Panic(uint256)` revert via overflow / division-by-zero, or use a plain `revert("reason")` macro.

## Custom Types

`#[derive(SolType)]` makes a struct usable as method parameter, return type, or storage value:

```rust,ignore
#[derive(pvm_contract_sdk::SolType)]
pub struct Point {
    pub x: U256,
    pub y: U256,
}

#[pvm_contract_sdk::method]
pub fn set_point(&mut self, point: Point) { /* ... */ }

#[pvm_contract_sdk::method]
pub fn get_point(&self) -> Point {
    Point { x: U256::from(1), y: U256::from(2) }
}
```

The derive emits `SolEncode`, `SolDecode`, `SolArrayElement`, and (under `--features abi-gen`) the storage-layout walker. Static structs (all fields have compile-time-known sizes) implement `StaticEncodedLen`; structs with dynamic fields (`String`, `Vec`, nested dynamic structs) use head + tail offset encoding.

See [specs/abi.md](abi.md) for the full encoding specification.

## Events

`#[derive(SolEvent)]` generates the event signature constant and emit helper:

```rust,ignore
#[derive(pvm_contract_sdk::SolEvent)]
pub struct Transfer {
    #[indexed]
    pub from: Address,
    #[indexed]
    pub to: Address,
    pub value: U256,
}

#[pvm_contract_sdk::method]
pub fn transfer(&mut self, to: Address, value: U256) {
    // ... state updates ...
    Transfer { from: self.caller(), to, value }.emit(self.host());
}
```

`#[indexed]` fields become topics (max 3 after the signature topic); the rest are ABI-encoded into the data payload.

## ABI Generation

When the contract is built with `cargo pvm-contract build` (or `PvmBuilder::new().build()` in a `build.rs`), the build system runs the contract under `--features abi-gen` and emits:

```text
target/<profile>/<binary-name>.polkavm      — deployable bytecode
target/<profile>/<binary-name>.abi.json     — Ethereum-compatible ABI JSON (functions, events, errors, storageLayout)
```

The ABI JSON follows the standard Ethereum ABI format and can be consumed by viem, ethers.js, alloy, `cast`, or any tool that reads Solidity ABIs. The `storageLayout` section follows solc's shape so `cast storage <addr> <name>` resolves slot addresses correctly.

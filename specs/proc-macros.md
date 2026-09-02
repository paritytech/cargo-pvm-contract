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
            let caller = self.env().caller();
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

**Without a** `.sol` **file** — selectors are inferred from Rust function signatures. Rust `snake_case` is converted to `camelCase` for the Solidity signature.

## Contract Attribute Arguments


| Argument             | Default | Description                                                                                                                      |
| -------------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `"path.sol"`         | none    | Solidity interface file (validates every function is implemented and that `stateMutability` agrees with the Rust receiver shape) |
| `buffer = N`         | 256     | Stack calldata buffer size (no-alloc mode)                                                                                       |
| `allocator = "pico"` | none    | Use picoalloc heap allocator (required to *return* dynamic types like `String` / `Vec`)                                          |
| `allocator = "bump"` | none    | Use bump allocator (no free, smaller than picoalloc)                                                                             |
| `allocator_size = N` | 1024    | Heap size in bytes for allocator modes                                                                                           |
| `no_main`            | off     | Suppress the `fn main()` emission so a `#[contract]` can sit inside an integration test or library crate                         |




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

- `#[method]` — public contract method. `#[selector(name = "name")]` overrides the Solidity name (default: `snake_case` to `camelCase`); `#[method(rename = "name")]` is a supported alias.
- `#[constructor]` — runs once at deployment. Must take `&mut self`; pure/view constructors are rejected because they cannot initialize storage.
- `#[fallback]` — invoked when no method selector matches (or calldata is 1..=3 bytes).
- `#[receive]` — invoked on plain value transfers (empty calldata). Must take `&mut self` and no other arguments. Implicitly payable; `#[payable]` is rejected as redundant.
- `#[payable]` — marks a method as payable. Must be combined with `&mut self`. Adding it to a no-receiver or `&self` method is a compile error.
- `#[non_reentrant]` — emits an OpenZeppelin-compatible reentrancy guard on a `#[method]`. Mode is inferred from the receiver: `&mut self` gives a full guard (OZ `nonReentrant`), `&self` a read-only check (OZ `nonReentrantView`). On re-entry the method reverts with the `ReentrancyGuardReentrantCall` error (OZ v5 selector). Only valid on a `#[method]` with a receiver — applying it to a pure method, constructor, fallback, or receive handler is a compile error. Only meaningful for contracts that opt into `CallFlags::ALLOW_REENTRY` (pallet-revive rejects reentrancy by default).

When both `#[receive]` and `#[fallback]` are present, `receive` fires first on empty calldata.

### Mutability Inference

Solidity `stateMutability` is inferred from the Rust receiver shape. There is no explicit `#[view]` or `#[pure]` attribute.


| Receiver              | `#[payable]` | ABI emits         |
| --------------------- | ------------ | ----------------- |
| none (`fn foo(args)`) | —            | `pure`            |
| `&self`               | —            | `view`            |
| `&mut self`           | —            | `nonpayable`      |
| `&mut self`           | yes          | `payable`         |
| `&self`               | yes          | **compile error** |
| no receiver           | yes          | **compile error** |


If a `.sol` interface is provided, the macro rejects any mismatch between the Rust-inferred mutability and the `.sol` declaration.

## Environment Access

The macro injects a `pub host: Host` field on the contract struct (the field name `host` is reserved) and two accessors:

```rust,ignore
pub fn host(&self) -> &Host;   // raw HostApi surface
pub fn env(&self) -> Env;      // read-only transaction/block context
```

`env()` is the typed equivalent of Solidity's `msg.*` / `block.*` globals:

| Accessor                    | Solidity                | Returns   |
| --------------------------- | ----------------------- | --------- |
| `self.env().caller()`       | `msg.sender`            | `Address` |
| `self.env().origin()`       | `tx.origin`             | `Address` |
| `self.env().address()`      | `address(this)`         | `Address` |
| `self.env().value()`        | `msg.value`             | `U256`    |
| `self.env().balance()`      | `address(this).balance` | `U256`    |
| `self.env().base_fee()`     | `block.basefee`         | `U256`    |
| `self.env().block_number()` | `block.number`          | `u64`     |
| `self.env().timestamp()`    | `block.timestamp`       | `u64`     |
| `self.env().chain_id()`     | `block.chainid`         | `u64`     |

Plus the `<address>` members that read chain state, parameterized by the account being asked about:

| Accessor                      | Solidity                | Returns |
| ----------------------------- | ----------------------- | ------- |
| `self.env().balance_of(addr)` | `addr.balance`          | `U256`  |
| `self.env().has_code(addr)`   | `addr.code.length != 0` | `bool`  |

```rust,ignore
#[pvm_contract_sdk::method]
pub fn owner_is_caller(&self) -> bool {
    self.owner.get() == self.env().caller()
}
```

Both accessors take `&self`, so they are available to `view` methods. A `pure` method has no receiver and therefore has neither — the same boundary solc enforces (see [Mutability Inference](#mutability-inference)). A method that needs `caller`, block context, or any other host call must be `view` (`&self`) or stronger.

`Env` holds only a cloned `Host` handle — a ZST on riscv64, one `Rc` bump on host targets — so constructing one per use costs nothing. `value()` is always zero in a non-payable method reached through external dispatch, because the dispatch prelude reverts before the body runs if value was attached (an internal Rust call from a payable method skips that prelude).

**`caller()` vs `origin()`.** `caller()` is the immediate sender and changes at every call boundary; `origin()` is the transaction signer and is the same at every depth. Authorize on `caller()` — an `origin() == owner` check passes for *any* contract the owner is tricked into calling, which is the classic phishing-via-intermediary hole. `origin()`'s legitimate uses are narrow, mainly the top-level-frame test `caller() == origin()`. `address()` is `address(this)`, and under `delegatecall` it is the *delegating* contract's address (the storage context the code executes against), matching EVM semantics.

**Byte order.** The host reports numeric 32-byte values (`value`, `chain_id`, balances, block number, timestamp) **little-endian**; identifiers (`caller`, `origin`, `address`, `block_author`, `code_hash`, `block_hash`) are opaque byte strings that are not byte-swapped. `env()` decodes the numeric ones for you, which is the main reason to prefer it over reading raw buffers through `self.host()`. Note this differs from storage slots and ABI encoding, which are big-endian to match solc. `block_number()`, `timestamp()` and `chain_id()` return `u64` — the width pallet-revive actually holds (`BlockNumberFor<T>`, a millisecond moment, and `ChainId: Get<u64>`); the 32-byte host width is EVM-compatibility packaging. They read the buffer's low 8 bytes and ignore the high 24, with no range check. That's a guarantee for the timestamp and chain ID; for the block number it's runtime convention, since frame bounds `BlockNumberFor<T>` only by `AtLeast32Bit` and it is `u32` in every real runtime. The balance-shaped reads stay `U256`: `value()` and the two balances are genuinely 256-bit (pallet-revive reports balances in EVM units, scaling the native balance by `NativeToEthRatio`), and `base_fee()` is `uint256` in Solidity with no narrower pallet-guaranteed width.

**DSL handlers** get the same accessor from the `Host` they are handed:

```rust,ignore
fn transfer_handler(host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let caller: [u8; 20] = host.env().caller().into();
    /* ... */
}
```

`env()` is also a provided method on `ContractContext`, so a handler that already wrapped its host for typed cross-contract calls reads context off the wrapper (`cx.env().caller()`), and so does any helper written against the `&impl ContractContext` bound those call builders impose. The macro-generated inherent `env()` on the storage struct takes precedence over the trait method, so contract bodies never need the trait in scope.

**Testing.** `MockHostBuilder`'s numeric setters take typed values and encode little-endian, so seeded state reads back through `env()` unchanged:

```rust,ignore
let mock = MockHostBuilder::new().caller([0xAA; 20]).block_number(258).build();
let contract = MyToken::with_host(mock);
assert_eq!(contract.env().block_number(), 258);
```

The `*_raw` setters store 32 bytes verbatim; use them only when a test asserts byte layout.

## Storage

Storage helpers live in `pvm-storage` (re-exported from `pvm-contract-sdk`). The primary types are `Lazy<T>` (single value at a fixed slot), `Mapping<K, V>` (key-value), and `StorageVec<T>` (dynamic array, Solidity `T[]`). Fixed-size arrays `[T; N]` (Solidity `T[N]`, static element) are supported as values inside any of these.

Declare fields directly on the contract struct. Two layout modes:

- **Auto-numbered (default).** Omit `#[slot]` and the macro assigns slots in declaration order. Sub-word siblings pack into a shared slot solc-style (`Lazy<u32>` at byte 28; adjacent `Lazy<bool>` at byte 27, both in slot 0).
- **Explicit** `#[slot(N)]`**.** Pins a field at slot `N`. Restricted to full-slot types (`Mapping`, `StorageVec`, `Lazy<U256>`, `Lazy<String>`, multi-slot composites, `#[storage]` sub-structs). Sub-word types are rejected because solc would place them at byte `32 - sizeof(T)`, while explicit-slot mode would place them at byte 0. Mixing the two modes within one struct is not supported.

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
            let caller = self.env().caller();
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

Out-of-bounds `get` / `set` revert with solc's ABI-encoded `Panic(0x32)` (array out-of-bounds), matching Solidity; use `try_get` for a non-panicking read.

**Nested and composite shapes.** Because an inner collection is a *handle* (not a `StorageEncode` value), the nested accessors return borrow guards (`Ref` / `RefMut`) rather than the inner collection by value — which also enforces the view gate (a `&self` outer can only hand out a read-only `Ref`):

- `Mapping<K, StorageVec<T>>` (`mapping(K => T[])`): `get(&K) -> Ref<StorageVec<T>>` (read) / `entry(&K) -> RefMut<StorageVec<T>>` (write), then operate on the inner vec — `self.posts.entry(&author).push(&post)`.
- `StorageVec<StorageVec<T>>` (`T[][]`): `len` / `get(i) -> Ref<…>` / `try_get` / `first` / `last` / `iter` for reads; `grow() -> RefMut<…>` appends an empty inner row, `entry(i) -> RefMut<…>` mutates an existing one, and `erase_last() -> bool` drops the last row (the inner vec can't be returned by value, so it is destroyed rather than popped).

These compose to arbitrary depth with no per-shape code, via the `StorageType` / `SimpleStorageType` trait pair (issue #108): `StorageVec<S: StorageType>` and `Mapping<K, V: StorageType>` return `S::Get` / `V::Get` (a value for a leaf, a `Ref` / `RefMut` guard for a container), and by-value ops (`push` / `pop` / `insert` / `set`) are gated on `SimpleStorageType` (leaves only). So `StorageVec<Mapping<…>>` (`mapping(…)[]`), `Mapping<K, StorageVec<…>>`, and 3+-level nesting all work through the generic impls. Under `--features abi-gen`, `StorageVec<T>` is named as `T[]` (recursively, so `T[][]` and `mapping(K => T[])` resolve correctly) via its own `StorageLayoutEmit` impl.

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
    Transfer { from: self.env().caller(), to, value }.emit(self.host());
}
```

`#[indexed]` fields become topics (max 3 after the signature topic); the rest are ABI-encoded into the data payload.

## Interfaces (`#[interface_id]`)

`#[interface_id]` on a trait declares it as an on-chain interface and generates its ERC-165 interface ID — the XOR of the 4-byte selectors of its methods — as a defaulted associated constant:

```rust,ignore
#[pvm_contract_sdk::interface_id]
pub trait IErc20 {
    fn total_supply(&self) -> U256;
    fn balance_of(&self, account: Address) -> U256;
    #[selector(name = "transfer")]
    fn transfer(&mut self, to: Address, amount: U256) -> bool;
    // ...
}

// generated:
// const INTERFACE_ID: [u8; 4];
```

`INTERFACE_ID` is a defaulted associated const, so read it through a concrete implementor: `<MyToken as IErc20>::INTERFACE_ID`. This is the value a contract returns from `supportsInterface(bytes4)`.

- Each method's selector is `keccak256` of its canonical Solidity signature. The Solidity name defaults to the `camelCase` of the Rust name and is overridden with `#[selector(name = "...")]` — the same attribute used on `#[method]`.
- Parameter types are resolved through their `SolEncode::SOL_NAME` at const-eval, so custom types (`#[derive(SolType)]` structs) work as parameters.
- Adding the associated const makes the trait non-object-safe (it can no longer be used behind `dyn`).

Compile errors: an empty trait, a generic method (its selector is undefined), a `#[cfg]`-gated method (it would still contribute its selector to the XOR, so the ID wouldn't match the active method set), a method with a default body (it would still be counted in `INTERFACE_ID` but is not folded into dispatch unless the impl restates it, so ERC-165 could advertise a method the contract doesn't serve), a trait that already declares `INTERFACE_ID`, or two methods that produce the same selector (they would silently cancel in the XOR — rename one with `#[selector(name = "...")]`).

## Interface Composition (`implements(...)`)

`#[contract(implements(ITrait, ...))]` folds the methods of each in-module `impl ITrait for Contract` block into the dispatch table as real entry points, letting you keep every interface forwarder in one compiler-checked trait impl, where overriding a method is simply a matter of writing a different impl body.

```rust,ignore
#[pvm_contract_sdk::contract(implements(IErc20, IErc165))]
mod my_token {
    pub struct MyToken { pub erc20: Erc20State }
    impl MyToken { #[constructor] pub fn new(&mut self) {} }   // only genuinely-custom methods

    impl IErc20 for MyToken {
        fn total_supply(&self) -> U256 { self.erc20._total_supply() }
        fn transfer(&mut self, to: Address, v: U256) -> Result<bool, Error> {
            self.erc20._transfer(self.caller(), to, v)          // override = a different body
        }
        // ... rest of the interface, no `#[method]` attributes
    }

    impl IErc165 for MyToken {                                  // ERC-165, opt-in, no codegen
        fn supports_interface(&self, id: [u8; 4]) -> bool {
            id == [0x01, 0xff, 0xc9, 0xa7] || id == <Self as IErc20>::INTERFACE_ID
        }
    }
}
```

- A folded method dispatches through `<MyToken as IErc20>::method(this, ...)`, so it runs the contract's own body (overrides work) and can't be shadowed by an inherent method of the same name.
- Interfaces are matched on the path segments written in `implements(...)`: a bare name matches any impl path ending in it, a qualified name only one agreeing on that suffix. A same-trait `impl` for a different struct in the module is skipped (the contract's own impl wins regardless of order); the `impl` folded must target the contract struct, be non-generic, and carry no `where` clause. Folded methods must take a receiver and have concrete (non-`Self::_`) parameters.
- Mutability is inferred from the receiver, exactly as for inherent methods. Folded methods carry no `#[method]` attribute; the same per-method behavior attributes still apply, written on the method in the `impl ITrait for Contract` block (not on the trait): `#[payable]`, `#[non_reentrant]`, and `#[selector(name = "...")]`.
- `implements(IErc20<Error = MyError>)` binds the trait's associated `Error` so a folded method returning `Result<_, Self::Error>` registers `MyError` in the ABI. The binding is verified against the impl's actual `type Error` by a const-eval assertion, so a mismatch is a compile error.
- Two dispatched methods (inherent or folded) that share a 4-byte selector are a **compile error** — rename one with `#[selector(name = "...")]`. Inherent methods are ordered before folded ones in dispatch and the ABI.

Compile errors: `implements()` empty or listing a duplicate trait; an interface with no matching `impl` in the module; an ambiguous match, where two distinct traits sharing the matched suffix both have an `impl` for the contract; an `impl` of the interface that targets no struct but the contract (all same-trait impls point elsewhere), or one that is generic or carries a `where` clause; a generic folded method; a no-receiver folded method; a folded method with a `Self::_`-rooted parameter *or return type* (e.g. `Self::Value`); a folded error type that nests `Self` (e.g. `Wrapper<Self::Error>` — write it concretely instead); a folded method returning `Result<_, Self::Error>` with no `<Error = Ty>` binding; a `<Error = Ty>` binding that disagrees with the impl's `type Error`; two dispatched methods (inherent or folded) that share a 4-byte selector; a lifecycle attribute (`#[constructor]`/`#[fallback]`/`#[receive]`) on a folded method (it always dispatches as an ordinary method — declare lifecycle handlers as inherent `impl` methods); and `#[cfg]`/`#[cfg_attr]` on a folded method *or on the folded* `impl` *block* (use an inherent `#[method]` for feature-gated entry points). Only the traits listed in `implements(...)` are folded — an `impl OtherTrait for Contract` not listed is left as an ordinary trait impl.

Only the methods the `impl` block writes become entry points. A trait method that has a **default body** but isn't restated in the impl is *not* folded: a `.sol` interface flags it through the missing-implementation check, while without a `.sol` it is silently absent from both dispatch and the ABI. Restate the method in the `impl` to expose it. (An `#[interface_id]` trait rejects default bodies outright, since a defaulted method would otherwise be counted in `INTERFACE_ID` while going undispatched.)

## ABI Generation

When the contract is built with `cargo pvm-contract build` (or `PvmBuilder::new().build()` in a `build.rs`), the build system runs the contract under `--features abi-gen` and emits:

```text
target/<profile>/<binary-name>.polkavm      — deployable bytecode
target/<profile>/<binary-name>.abi.json     — Ethereum-compatible ABI JSON (functions, events, errors, storageLayout)
```

The ABI JSON follows the standard Ethereum ABI format and can be consumed by viem, ethers.js, alloy, `cast`, or any tool that reads Solidity ABIs. The `storageLayout` section follows solc's shape so `cast storage <addr> <name>` resolves slot addresses correctly.
#![doc = include_str!("../proc-macros.md")]

extern crate proc_macro2;

mod abi_import;
mod codegen;
mod signature;
mod utils;
use proc_macro::TokenStream;
use syn::{DeriveInput, ItemFn, ItemMod, ItemTrait, parse_macro_input};

/// Marks a module as a PVM smart contract, generating dispatch logic and entry points.
///
/// # Attributes
///
/// - `"path/to/Interface.sol"` - Optional Solidity interface file defining method signatures
/// - `buffer = N` - Sets stack calldata buffer size in default no-alloc mode (default: 256)
/// - `allocator = "pico"` - Enables allocator mode using picoalloc
/// - `allocator = "bump"` - Enables allocator mode using pvm-bump-allocator
/// - `allocator_size = N` - Sets allocator heap size (with `allocator = "pico"` or `allocator = "bump"`, default: 1024)
///
/// # Usage with Solidity Interface
///
/// Create a Solidity interface file defining your contract's ABI:
///
/// ```solidity
/// // MyToken.sol
/// interface MyToken {
///     function totalSupply() external view returns (uint256);
///     function balanceOf(address account) external view returns (uint256);
///     function transfer(address to, uint256 amount) external;
/// }
/// ```
///
/// Then implement the interface in Rust:
///
/// ```ignore
/// use pvm_contract_sdk::{Address, U256};
///
/// #[pvm_contract_sdk::contract("MyToken.sol")]
/// mod my_token {
///     use super::*;
///
///     pub struct MyToken;
///
///     impl MyToken {
///         #[pvm_contract_sdk::constructor]
///         pub fn new(&mut self) -> Result<(), Error> { Ok(()) }
///
///         #[pvm_contract_sdk::method]
///         pub fn total_supply(&self) -> U256 { U256::ZERO }
///
///         #[pvm_contract_sdk::method]
///         pub fn balance_of(&self, _account: Address) -> U256 { U256::ZERO }
///
///         #[pvm_contract_sdk::method]
///         pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), TokenError> { Ok(()) }
///
///         #[pvm_contract_sdk::fallback]
///         pub fn fallback(&mut self) -> Result<(), Error> { Ok(()) }
///     }
/// }
/// ```
///
/// # Usage without Solidity Interface
///
/// You can also define contracts without a `.sol` file. Signatures are inferred from Rust types:
///
/// ```ignore
/// use pvm_contract_sdk::{Address, U256};
///
/// #[pvm_contract_sdk::contract]
/// mod my_token {
///     use super::*;
///
///     pub struct MyToken;
///
///     impl MyToken {
///         #[pvm_contract_sdk::constructor]
///         pub fn new(&mut self) -> Result<(), Error> { Ok(()) }
///
///         #[pvm_contract_sdk::method]
///         pub fn total_supply(&self) -> U256 { U256::ZERO }
///
///         #[pvm_contract_sdk::method]
///         pub fn balance_of(&self, account: Address) -> U256 { U256::ZERO }
///
///         #[pvm_contract_sdk::method]
///         pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), TokenError> { Ok(()) }
///
///         #[pvm_contract_sdk::fallback]
///         pub fn fallback(&mut self) -> Result<(), Error> { Ok(()) }
///     }
/// }
/// ```
///
/// The builder will automatically generate an ABI JSON file alongside the `.polkavm` binary.
///
/// # Name Matching
///
/// Rust function names are converted to camelCase for Solidity compatibility:
/// - `total_supply` → `totalSupply`
/// - `balance_of` → `balanceOf`
///
/// For custom name mapping, use the `rename` attribute:
///
/// ```ignore
/// #[pvm_contract::method(rename = "getBalance")]
/// pub fn balance_of(account: Address) -> U256 { ... }
/// ```
///
/// # Generated Code
///
/// ## Entry Points and Router
///
/// The macro generates the following **inside** the contract module:
///
/// - `pub const MAX_RETURN_LEN: usize` — the size of the caller-owned output
///   buffer (`max` return size; floored at 256 in no-alloc mode for the error
///   path).
/// - `pub fn route<B: OutSink>(this: &mut MyToken, selector: [u8; 4],
///   input: &[u8], out: &mut B) -> Outcome` — selector dispatch. A matched arm
///   that succeeds encodes its result into `out` and returns
///   `Outcome::Return(len)`; an unmatched selector returns `Outcome::Unhandled`.
///   The single `finalize_outcome` exit lowers a `Return` to `return_value`.
///   Reverts never become an `Outcome`: a method's own `Err(e)` and every
///   framework abort (size check, malformed-calldata decode, payable guard,
///   storage `Panic`) diverge via `this.host().revert(...)` (`-> !`) at the point
///   they occur.
/// - `pub extern "C" fn deploy()` — PolkaVM deploy entry point (riscv64-only)
/// - `pub extern "C" fn call()` — PolkaVM call entry point (riscv64-only);
///   reads calldata, drives `route()` with an output buffer, and lowers the
///   returned `Outcome` via `finalize_outcome`. On `Outcome::Unhandled` it falls
///   through to the fallback or the `UNKNOWN_SELECTOR` revert.
///
/// Outside the module, a `Router` trait impl is generated:
///
/// ```ignore
/// impl ::pvm_contract_sdk::Router for my_token::MyToken {
///     fn route<B: ::pvm_contract_sdk::OutSink>(
///         &mut self,
///         selector: [u8; 4],
///         input: &[u8],
///         out: &mut B,
///     ) -> ::pvm_contract_sdk::Outcome {
///         my_token::route(self, selector, input, out)
///     }
/// }
/// ```
///
/// ## Host and environment accessors
///
/// The macro injects a `pub host: Host` field on the contract struct (the field
/// name `host` is reserved) plus two accessors:
///
/// ```ignore
/// #[cfg(not(feature = "abi-gen"))]
/// impl MyToken {
///     #[inline(always)]
///     pub fn host(&self) -> &::pvm_contract_sdk::Host { &self.host }
///     #[inline(always)]
///     pub fn env(&self) -> ::pvm_contract_sdk::Env { self.host.env() }
/// }
/// ```
///
/// `env()` is the read-only view of transaction and block context — the typed
/// equivalent of Solidity's `msg.*` / `block.*` globals and of the `<address>`
/// members that read chain state:
///
/// | Accessor | Solidity | Returns |
/// |---|---|---|
/// | `self.env().caller()` | `msg.sender` | `Address` |
/// | `self.env().origin()` | `tx.origin` | `Address` |
/// | `self.env().address()` | `address(this)` | `Address` |
/// | `self.env().value()` | `msg.value` | `U256` |
/// | `self.env().balance()` | `address(this).balance` | `U256` |
/// | `self.env().base_fee()` | `block.basefee` | `U256` |
/// | `self.env().block_number()` | `block.number` | `u64` |
/// | `self.env().timestamp()` | `block.timestamp` | `u64` |
/// | `self.env().chain_id()` | `block.chainid` | `u64` |
/// | `self.env().balance_of(addr)` | `addr.balance` | `U256` |
/// | `self.env().has_code(addr)` | `addr.code.length != 0` | `bool` |
///
/// Both accessors take `&self`, so they are available to `view` methods. A
/// `pure` method has no receiver and therefore has neither — matching solc,
/// which rejects the same operations in a `pure` function.
///
/// Reach for `self.host()` only for raw `HostApi` calls that `env()` doesn't
/// cover; note the host returns numeric 32-byte values little-endian, which
/// `env()` decodes for you.
///
/// Alongside these, the same `#[cfg(not(feature = "abi-gen"))]` block emits the
/// `ContractContext` impl that gates cross-contract calls on `&self` vs
/// `&mut self`, and (off riscv64) a `with_host(backend)` test constructor.
///
/// The contract holds a concrete `Host` whose internals are cfg-gated:
/// on riscv64 it's a zero-sized type wrapping `PolkaVmHost` (zero overhead), on the
/// host target it wraps `Rc<dyn HostApi>` so tests can inject a `MockHost`.
/// `HostApi::return_value` (the success door) has a cfg-gated signature: `-> !`
/// on `riscv64` (the `pallet_revive_uapi` syscall), `-> ()` on host targets
/// (captures into `MockHost`). `HostApi::revert` (the failure door) is `-> !`
/// on both targets. The generated dispatch code has no `cfg(target_arch)` gate
/// — the same path serves production and native unit tests.
///
/// All generated items are gated behind `#[cfg(not(feature = "abi-gen"))]`.
///
/// ### Composition and inheritance
///
/// `route()` returns an `Outcome`: `Return(len)` when the selector matched and
/// succeeded (the arm encoded `len` bytes into `out`), or `Unhandled` when it
/// did not match (a matched arm that reverts diverges and does not return).
/// Chain multiple routers by trying each in turn and matching `Unhandled`, then
/// lowering the first `Return` once:
///
/// The shared buffer must be sized to the **max** `MAX_RETURN_LEN` across every
/// module in the chain (each const covers only its own module's returns):
///
/// ```ignore
/// pub extern "C" fn call() {
///     let (selector, input) = read_calldata();
///     const CAP: usize = if my_extension::MAX_RETURN_LEN > erc20_base::MAX_RETURN_LEN {
///         my_extension::MAX_RETURN_LEN
///     } else {
///         erc20_base::MAX_RETURN_LEN
///     };
///     let mut storage = [0u8; CAP];
///     let mut out: &mut [u8] = &mut storage;
///     match my_extension::route(&mut this, selector, input, &mut out) {
///         Outcome::Unhandled => {} // fall through to the parent
///         outcome => return finalize_outcome(this.host(), outcome, &out),
///     }
///     match erc20_base::route(&mut this, selector, input, &mut out) {
///         Outcome::Unhandled => this.host().revert(&UNKNOWN_SELECTOR),
///         outcome => finalize_outcome(this.host(), outcome, &out),
///     }
/// }
/// ```
///
/// ### Native unit tests
///
/// Two test layers, both host-agnostic against `MockHost`:
///
/// **Method-level** (recommended for most logic) — call methods directly on
/// the contract struct, observe Rust return values:
///
/// ```ignore
/// let mock = MockHostBuilder::new().build();
/// let mut contract = my_token::MyToken::with_host(mock.clone());
/// let bal = contract.balance_of(account);
/// assert_eq!(bal, U256::from(42));
/// ```
///
/// The macro generates `MyToken::with_host(backend)` — wraps any
/// `HostApi` implementor in `Rc<dyn HostApi>` and initialises `#[slot(N)]`
/// fields. Mirrors the std-lib `Vec::with_capacity` idiom for
/// "constructor with a non-default dependency." The user's
/// `#[constructor]` is NOT run — seed storage on the mock directly if
/// you need initial state.
///
/// **Dispatch-level** (selector routing, ABI revert encoding) — drive
/// `route()` with raw calldata: a success comes back as `Outcome::Return`
/// (no host call, no unwind), while every revert diverges and is caught with
/// `assert_reverts!`:
///
/// ```ignore
/// let mut buf = [0u8; my_token::MAX_RETURN_LEN];
/// let mut out: &mut [u8] = &mut buf;
///
/// // Success: returned as data — no host call, no unwind.
/// match my_token::route(&mut contract, BALANCE_OF_SELECTOR, &input, &mut out) {
///     Outcome::Return(n) => { /* decode and assert on out.view(n) */ }
///     other => panic!("expected Return, got {other:?}"),
/// }
///
/// // Any revert — a method's own `Err`, size check, decode failure, storage
/// // OOB — diverges via host.revert(...); catch it with `assert_reverts!`.
/// assert_reverts!(mock, INVALID_CALLDATA,
///     my_token::route(&mut contract, sel, &short_input, &mut out));
/// ```
///
/// ## Error Handling
///
/// The scaffold uses `EmptyError` for methods that don't produce errors.
/// To add custom errors, define error structs with `#[derive(SolError)]` and use them directly:
///
/// ```ignore
/// mod my_token {
///     #[derive(Debug, pvm_contract_macros::SolError)]
///     pub struct Unauthorized;
///     #[derive(Debug, pvm_contract_macros::SolError)]
///     pub struct InsufficientBalance;
///
///     pub struct MyToken;
///     impl MyToken {
///         // Single error: use the struct directly
///         pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), InsufficientBalance> { ... }
///     }
///
///     #[derive(Debug, pvm_contract_macros::SolError)]
///     pub enum TokenError {
///         InsufficientBalance(InsufficientBalance),
///         Unauthorized(Unauthorized),
///     }
/// }
/// ```
///
/// ## Dispatch Logic
///
/// Stack and allocator modes use the same direct dispatch logic.
/// The only difference is buffer allocation:
///
/// - **allocator mode**: `let mut call_data = vec![0u8; call_data_len];`
/// - **default stack mode**: `let mut call_data = [0u8; BUFFER_SIZE];` with overflow check
///
/// All types are decoded and encoded uniformly via trait dispatch (`SolDecode`, `SolEncode`).
/// The macro never inspects types — it emits trait calls and lets the compiler resolve them.
///
/// ### Default stack generated code example
///
/// ```ignore
/// #[pvm_contract_sdk::contract("MyToken.sol", buffer = 512)]
/// mod my_token {
///     use super::*;
///
///     pub struct MyToken;
///
///     impl MyToken {
///         #[pvm_contract_sdk::method]
///         pub fn balance_of(&self, account: Address) -> U256 { U256::ZERO }
///
///         #[pvm_contract_sdk::method]
///         pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), TokenError> { Ok(()) }
///
///         // `#[payable]` requires `&mut self`; read the attached value with
///         // `self.env().value()`.
///         #[pvm_contract_sdk::method]
///         #[pvm_contract_sdk::payable]
///         pub fn deposit(&mut self, to: Address) { let _ = self.env().value(); }
///
///         #[pvm_contract_sdk::constructor]
///         pub fn new(&mut self) -> Result<(), TokenError> { Ok(()) }
///     }
///
///     // --- Generated inside the module: ---
///
///     // Output buffer size: max static return, floored at 256 (error path).
///     pub const MAX_RETURN_LEN: usize = /* max(256, <U256>::ENCODED_SIZE, …) */;
///
///     pub fn route<B: ::pvm_contract_sdk::OutSink>(
///         this: &mut MyToken,
///         selector: [u8; 4],
///         input: &[u8],
///         out: &mut B,
///     ) -> ::pvm_contract_sdk::Outcome {
///         // Value-transfer hoist — read once, used by all non-payable arms
///         let __has_value =
///             ::pvm_contract_sdk::value_transferred_is_nonzero(this.host());
///
///         // Selector consts — precomputed from .sol, or derived via SOL_NAME
///         const __SEL_balance_of: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
///         const __SEL_transfer: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
///         const __SEL_deposit: [u8; 4] = /* keccak("deposit(address)")[..4] */;
///
///         match selector {
///             // balanceOf(address) -> uint256  (non-payable)
///             __SEL_balance_of => {
///                 // Non-payable guard (shared helper) — reverts if msg.value > 0
///                 __pvm_assert_value_zero(this.host(), __has_value);
///                 if input.len() < <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE {
///                     // Size check is a mid-expression abort: diverges via revert.
///                     <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
///                         this.host(),
///                         &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA);
///                 }
///                 let mut __decode_offset: usize = 0;
///                 let account = /* unsafe StaticDecode::decode_unchecked … */;
///                 let result = this.balance_of(::core::convert::Into::into(account));
///                 const __LEN: usize =
///                     <U256 as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE;
///                 let __buf = out.reserve(__LEN);
///                 <U256 as ::pvm_contract_sdk::SolEncode>::encode_to(&result, __buf);
///                 ::pvm_contract_sdk::Outcome::Return(__LEN)
///             }
///
///             // transfer(address,uint256) — fallible, non-payable
///             __SEL_transfer => {
///                 __pvm_assert_value_zero(this.host(), __has_value);
///                 // ... size check + decode ...
///                 match this.transfer(
///                     ::core::convert::Into::into(to),
///                     ::core::convert::Into::into(amount),
///                 ) {
///                     Ok(()) => ::pvm_contract_sdk::Outcome::Return(0),
///                     Err(e) => {
///                         // A revert diverges: encode into `out`, then revert.
///                         let __n = { let b = out.reserve(256); e.encode_to(b) };
///                         <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
///                             this.host(), out.view(__n))   // -> !
///                     }
///                 }
///             }
///
///             // deposit(address) — payable: no __has_value guard
///             __SEL_deposit => {
///                 // ... size check + decode `to` ...
///                 this.deposit(::core::convert::Into::into(to));
///                 ::pvm_contract_sdk::Outcome::Return(0)
///             }
///
///             _ => ::pvm_contract_sdk::Outcome::Unhandled,
///         }
///     }
///
///     #[polkavm_derive::polkavm_export]
///     pub extern "C" fn deploy() {
///         let host = ::pvm_contract_sdk::Host::new();
///         let mut this = MyToken { /* #[slot(N)] fields, */ host };
///         // Non-payable constructor: reject value (reverts via this.host().revert)
///         __pvm_assert_non_payable(this.host());
///         // ... read constructor calldata, decode, call new() ...
///     }
///
///     #[polkavm_derive::polkavm_export]
///     pub extern "C" fn call() {
///         let host = ::pvm_contract_sdk::Host::new();
///         let mut this = MyToken {
///             // storage fields would be initialised here via the safe door
///             // field: <Type as ::pvm_contract_sdk::StorageComponent>::new_at(
///             //     StorageKey::from_slot(N), offset, alone, host.clone()),
///             host,
///         };
///         let call_data_len = HostFnImpl::call_data_size() as usize;
///         let mut call_data = [0u8; 512];
///         if call_data_len > 512 {
///             this.host().revert(
///                 &::pvm_contract_sdk::framework_errors::CALLDATA_TOO_LARGE);
///         }
///         HostFnImpl::call_data_copy(&mut call_data[..call_data_len], 0);
///
///         if call_data_len < 4 {
///             // With #[receive]: dispatches receive on empty calldata (returns
///             // after). The empty-calldata branch is only emitted when a
///             // #[receive] handler is present — contracts without it pay zero
///             // bytecode cost here.
///             if call_data_len == 0 {
///                 this.receive();
///                 return;
///             }
///             // With #[fallback]: calls fallback. Without: reverts with NoSelector.
///             this.host().revert(
///                 &::pvm_contract_sdk::framework_errors::NO_SELECTOR);
///         }
///
///         let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
///         let input = &call_data[4..call_data_len];
///
///         // Drive route() with an output buffer, then lower the Outcome once.
///         let mut __out_storage = [0u8; MAX_RETURN_LEN];
///         let mut __out: &mut [u8] = &mut __out_storage;
///         match route(&mut this, selector, input, &mut __out) {
///             ::pvm_contract_sdk::Outcome::Unhandled => {
///                 // With #[fallback]: calls fallback. Without: UnknownSelector.
///                 this.host().revert(
///                     &::pvm_contract_sdk::framework_errors::UNKNOWN_SELECTOR);
///             }
///             __outcome => ::pvm_contract_sdk::finalize_outcome(
///                 this.host(), __outcome, &__out),
///         }
///     }
/// }
///
/// // Generated outside the module:
/// impl ::pvm_contract_sdk::Router for my_token::MyToken {
///     fn route<B: ::pvm_contract_sdk::OutSink>(
///         &mut self,
///         selector: [u8; 4],
///         input: &[u8],
///         out: &mut B,
///     ) -> ::pvm_contract_sdk::Outcome {
///         my_token::route(self, selector, input, out)
///     }
/// }
/// ```
///
/// ### Allocator mode
///
/// Two differences in `call()`. The calldata buffer is heap-allocated:
///
/// ```ignore
/// let mut call_data = alloc::vec![0u8; call_data_len];
/// ```
///
/// And the output buffer is a `Vec`-backed [`OutSink`] with an inline stack fast
/// path (sized to `MAX_RETURN_LEN`), so static returns stay on the stack and
/// only large/dynamic returns spill to the heap:
///
/// ```ignore
/// struct __OutBuf { stack: [u8; MAX_RETURN_LEN], spill: alloc::vec::Vec<u8> }
/// impl ::pvm_contract_sdk::OutSink for __OutBuf { /* reserve/view: stack or spill */ }
/// let mut __out = __OutBuf { stack: [0u8; MAX_RETURN_LEN], spill: alloc::vec::Vec::new() };
/// match route(&mut this, selector, input, &mut __out) { /* … finalize_outcome … */ }
/// ```
///
/// The `route()` function and dispatch logic are otherwise identical.
///
/// ## Allocator Setup
///
/// When an allocator is specified, the macro generates a `#[global_allocator]` and
/// brings `alloc::vec` / `alloc::vec::Vec` into scope. All allocator items are gated
/// behind `#[cfg(not(feature = "abi-gen"))]` so they are skipped during ABI generation
/// (which runs on the host).
///
/// ### `allocator = "pico"`
///
/// Uses the `picoalloc` crate with a fixed-size array-backed heap
/// (default 1024 bytes, customisable via `allocator_size`):
///
/// ```ignore
/// extern crate alloc;
/// use alloc::vec;
/// use alloc::vec::Vec;
/// use alloc::string::String;
///
/// #[global_allocator]
/// static mut ALLOC: picoalloc::Mutex<
///     picoalloc::Allocator<picoalloc::ArrayPointer<1024>>
/// > = {
///     static mut ARRAY: picoalloc::Array<1024> =
///         picoalloc::Array([0u8; 1024]);
///
///     picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {
///         picoalloc::ArrayPointer::new(&raw mut ARRAY)
///     }))
/// };
/// ```
///
/// Override the heap size with `allocator_size`:
///
/// ```ignore
/// #[pvm_contract::contract("MyToken.sol", allocator = "pico", allocator_size = 4096)]
/// mod my_token { /* ... */ }
/// ```
///
/// ### `allocator = "bump"`
///
/// Uses the `pvm-bump-allocator` crate, a simple bump allocator for PVM
/// smart contracts (based on the ink! bump allocator). Heap size defaults
/// to 1024 bytes and can be changed with `allocator_size`:
///
/// ```ignore
/// extern crate alloc;
/// use alloc::vec;
/// use alloc::vec::Vec;
/// use alloc::string::String;
///
/// #[global_allocator]
/// static ALLOC: pvm_bump_allocator::BumpAllocator<1024> =
///     pvm_bump_allocator::BumpAllocator::new();
/// ```
///
/// You must add `pvm-bump-allocator` to your `Cargo.toml`:
///
/// ```toml
/// pvm-bump-allocator = { path = "../../crates/pvm-bump-allocator" }
/// ```
///
/// ### No allocator (default)
///
/// No allocator setup is generated. Calldata is read into a stack-allocated
/// `[0u8; BUFFER_SIZE]` array, and only static return types are allowed.
///
/// # Return Type Flexibility
///
/// Methods can return either:
/// - `Result<T, Error>` - For fallible operations that may revert
/// - `T` - For infallible operations (macro wraps in `Ok(...)`)
#[proc_macro_attribute]
pub fn contract(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as codegen::ContractArgs);
    let input = parse_macro_input!(item as ItemMod);

    match codegen::expand_contract(args, input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derives [`StorageComponent`] for a struct so it can be embedded as a field
/// inside another `#[storage]` struct or directly inside the `#[contract]`
/// storage struct. Also derives a container [`StorageType`] (`get`/`entry`
/// hand out a `Ref`/`RefMut<Self>` guard) so the struct can be a value of
/// `Mapping<K, Self>` or an element of `StorageVec<Self>`.
///
/// Field slots are auto-numbered in declaration order; the struct's `SLOTS` is
/// the layout walker's **packed** slot count (`step.next_slot + 1`), so
/// adjacent sub-word fields (e.g. two `Lazy<u128>`) share a slot solc-style
/// rather than each claiming a fresh one. The contract struct's auto-numbering
/// uses this `SLOTS` constant to assign contiguous ranges, so embedding — and
/// using the struct as a `StorageVec` element (where `SLOTS` is the stride) —
/// nests cleanly without manual slot math.
///
/// # Example
///
/// ```ignore
/// #[pvm_contract_sdk::storage]
/// pub struct Erc20 {
///     total_supply: Lazy<U256>,
///     balances: Mapping<Address, U256>,
///     allowances: Mapping<Address, Mapping<Address, U256>>,
/// }
///
/// #[pvm_contract_sdk::contract]
/// mod my_contract {
///     pub struct MyContract {
///         erc20: super::Erc20,           // claims 3 slots
///         additional_state: Lazy<u32>,   // claims slot 3
///     }
/// }
/// ```
///
/// # Constraints
///
/// - Only named-field structs are supported (unit/tuple structs rejected).
/// - All fields must implement `StorageComponent` (which `Lazy`/`Mapping` and
///   other `#[storage]` structs do).
/// - `#[slot(N)]` pinning inside a `#[storage]` struct is *not* supported.
///   Use auto-numbering, or write the leaf fields directly on the contract
///   struct if you need explicit slots.
/// - On the contract struct, `#[slot(N)]` accepts only full-slot types
///   (`PACKED_BYTES == 32`): `Mapping`, `Lazy<U256>`, `Lazy<String>`,
///   `Lazy<Bytes>`, multi-slot composites like `Lazy<(U256, U256)>`, and
///   `#[storage]` sub-structs. Sub-word types (`Lazy<bool>`, `Lazy<u32>`,
///   `Lazy<Address>`, etc.) are rejected at compile time — explicit-mode
///   would place them at byte 0 of the slot while solc places them
///   right-aligned, producing a non-solc layout. Sub-word packing is the
///   auto-numbered walker's job (it packs siblings per solc via
///   `layout_step`); wrap the field in a `#[storage]` sub-struct if you
///   need to pin the group at a specific slot.
#[proc_macro_attribute]
pub fn storage(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::ItemStruct);
    match codegen::expand_storage_struct(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Declares a trait as an on-chain interface and generates its ERC-165 interface
/// ID as an associated constant.
///
/// # Generated Code
///
/// Given
///
/// ```ignore
/// #[pvm_contract_sdk::interface_id]
/// pub trait IErc20 {
///     fn total_supply(&self) -> U256;
///     fn balance_of(&self, account: Address) -> U256;
///     #[selector(name = "transfer")]
///     fn transfer(&mut self, to: Address, amount: U256) -> Result<bool, Error>;
/// }
/// ```
///
/// the macro adds a defaulted associated constant
///
/// ```ignore
/// const INTERFACE_ID: [u8; 4] = /* XOR of every method's 4-byte selector */;
/// ```
///
/// Each method's selector is `keccak256` of its canonical Solidity signature
/// (the camelCase of the Rust name, or the `#[selector(name = "...")]` override).
/// Parameter types are rendered through their `SolEncode::SOL_NAME` at const-eval,
/// so custom types work.
///
/// It is a compile error for the trait to be empty, to have a generic method, to
/// already declare `INTERFACE_ID`, or for two methods to share a selector (which
/// would silently cancel in the XOR).
#[proc_macro_attribute]
pub fn interface_id(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemTrait);
    match codegen::expand_interface_id(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Overrides the Solidity name a method contributes to selector computation:
/// `#[selector(name = "transfer")]`.
///
/// This is an inert helper attribute — it is consumed by `#[interface_id]` (on
/// trait methods) and by `#[contract]` (on `#[method]`s, where it is the
/// canonical spelling of the older `#[method(rename = "...")]`). Applied on its
/// own it expands to the item unchanged.
#[proc_macro_attribute]
pub fn selector(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Marks a function as a contract method. The signature is derived from the Solidity interface file.
///
/// # Attributes
///
/// - `rename = "name"` - Override the Solidity function name to match (default: snake_case conversion)
///
/// # Static vs Dynamic Return Types
///
/// The encoding strategy is determined by contract allocator settings and the return type:
///
/// **Allocator mode (`allocator = "pico"` or `allocator = "bump"`)**:
/// - Static return types (U256, Address, etc.) use compile-time sized buffers
/// - Dynamic return types (String, `Vec<T>`, etc.) automatically use runtime-sized buffers
///
/// ```ignore
/// #[pvm_contract::contract(allocator = "pico")]
/// mod MyContract {
///     // Static return - uses compile-time buffer size
///     #[pvm_contract::method]
///     pub fn balance_of(account: Address) -> U256 { ... }
///
///     // Dynamic return - automatically uses runtime-computed buffer size
///     #[pvm_contract::method]
///     pub fn greeting() -> String { ... }
/// }
/// ```
///
/// **Default stack mode**:
/// - Only static return types are allowed
/// - Returning a dynamic type will produce a compile error:
///   `Return type 'String' is dynamic and requires an explicit allocator. Set allocator = "pico" or allocator = "bump" in #[contract], or use static types.`
///
/// # Name Matching
///
/// By default, the macro converts the Rust function name from snake_case to camelCase
/// to match the Solidity function:
///
/// ```ignore
/// #[pvm_contract::method]
/// pub fn total_supply() -> U256 { ... }  // matches totalSupply()
/// ```
///
/// Use `rename` when the naming convention differs:
///
/// ```ignore
/// #[pvm_contract::method(rename = "getBalance")]
/// pub fn balance_of(account: Address) -> U256 { ... }  // matches getBalance(address)
/// ```
///
/// # Return Types
///
/// Methods support two return patterns:
///
/// ```ignore
/// // Fallible - can revert with error
/// #[pvm_contract::method]
/// pub fn transfer(to: Address, amount: U256) -> Result<(), Error> { ... }
///
/// // Infallible - always succeeds
/// #[pvm_contract::method]
/// pub fn balance_of(account: Address) -> U256 { ... }
/// ```
///
/// # Generated Code
///
/// The `#[method]` attribute is used by `#[contract]` to generate dispatch arms. Here are
/// examples of the generated call handling for static and dynamic return types (alloc mode).
///
/// ## Payable enforcement
///
/// At the top of `route()`, the macro hoists a single `value_transferred()` call.
/// Non-payable arms check `__has_value` and revert; methods marked `#[payable]`
/// skip the guard and are responsible for reading `value_transferred()` themselves
/// if they need the amount:
///
/// ```ignore
/// // Hoisted at the top of route() — shared by all arms
/// let __has_value = ::pvm_contract_sdk::value_transferred_is_nonzero(this.host());
/// ```
///
/// ## Static return (U256) — non-payable
///
/// Types implementing `StaticEncodedLen` use compile-time buffer sizing.
/// Non-payable methods emit a guard that reverts when value is attached:
///
/// ```ignore
/// #[pvm_contract::method]
/// pub fn balance_of(account: Address) -> U256 { ... }
///
/// // Generated dispatch arm (inside the module):
///
/// // 0) Non-payable guard (shared helper) — reverts if value was transferred
/// __pvm_assert_value_zero(this.host(), __has_value);
///
/// // 1) Size check + decode (static params use the no-alloc fast path)
/// if input.len() < <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE {
///     <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
///         this.host(),
///         &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA);
/// }
/// let mut __decode_offset: usize = 0;
/// let account = {
///     let __value = unsafe {
///         <Address as ::pvm_contract_sdk::StaticDecode>::decode_unchecked(
///             &input, __decode_offset)
///     };
///     __decode_offset += <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE;
///     __value
/// };
///
/// // 2) Call the method (no module prefix — generated inside the module)
/// let result = balance_of(::core::convert::Into::into(account));
///
/// // 3) Encode into the caller-owned buffer and return the outcome as data
/// const __LEN: usize = <U256 as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE;
/// let __buf = out.reserve(__LEN);
/// <U256 as ::pvm_contract_sdk::SolEncode>::encode_to(&result, __buf);
/// ::pvm_contract_sdk::Outcome::Return(__LEN)
/// ```
///
/// ## Payable method — `#[payable]` attribute
///
/// Marking a method with `#[payable]` tells the dispatcher to skip the
/// non-payable guard. The user reads `msg.value` themselves inside the body:
///
/// ```ignore
/// #[pvm_contract::method]
/// #[pvm_contract::payable]
/// pub fn deposit(to: Address) {
///     let mut buf = [0u8; 32];
///     pallet_revive_uapi::HostFnImpl::value_transferred(&mut buf);
///     let amount = ruint::aliases::U256::from_le_bytes(buf);
///     // ...
/// }
///
/// // Generated dispatch arm (inside the module):
///
/// // No __has_value guard — this method is payable
///
/// if input.len() < <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE {
///     <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
///         this.host(),
///         &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA);
/// }
/// let mut __decode_offset: usize = 0;
/// let to = {
///     let __value = unsafe {
///         <Address as ::pvm_contract_sdk::StaticDecode>::decode_unchecked(
///             &input, __decode_offset)
///     };
///     __decode_offset += <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE;
///     __value
/// };
///
/// deposit(::core::convert::Into::into(to));
/// ::pvm_contract_sdk::Outcome::Return(0)
/// ```
///
/// ## Return encoding (alloc mode)
///
/// In alloc mode the arm encodes into the caller-owned buffer just like the
/// static case — `out.reserve(encode_len)` returns a slice of exactly the right
/// size (the [`OutSink`] keeps small returns on its inline stack and spills only
/// large/dynamic ones to the heap), and the arm evaluates to `Outcome::Return`:
///
/// ```ignore
/// #[pvm_contract::method]
/// pub fn greeting() -> String { ... }
///
/// // Generated dispatch arm (in alloc mode, inside route()):
///
/// let result = greeting();
/// let __len = <String as ::pvm_contract_sdk::SolEncode>::encode_len(&result);
/// let __buf = out.reserve(__len);
/// <String as ::pvm_contract_sdk::SolEncode>::encode_to(&result, __buf);
/// ::pvm_contract_sdk::Outcome::Return(__len)
/// ```
#[proc_macro_attribute]
pub fn method(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as codegen::MethodArgs);
    let input = parse_macro_input!(item as ItemFn);

    match codegen::expand_method(args, input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Marks a function as the contract constructor, called during deployment.
///
/// # Examples
///
/// A constructor must take `&mut self` — it exists to initialise storage, so
/// `pure` and `view` receivers are rejected. Read the deployer with
/// `self.env().caller()`.
///
/// Constructor that can revert:
///
/// ```ignore
/// #[pvm_contract_sdk::constructor]
/// pub fn new(&mut self) -> Result<(), Error> {
///     self.owner.set(&self.env().caller());
///     Ok(())
/// }
/// ```
///
/// Constructor that never reverts:
///
/// ```ignore
/// #[pvm_contract_sdk::constructor]
/// pub fn new(&mut self) {
///     self.owner.set(&self.env().caller());
/// }
/// ```
///
/// When returning `Result<(), Error>`, returning `Err` reverts the deployment.
#[proc_macro_attribute]
pub fn constructor(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    match codegen::expand_constructor(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Marks a function as the fallback handler for unknown selectors.
///
/// Called when:
/// - Calldata is less than 4 bytes
/// - No method matches the selector
///
/// # Example
///
/// ```ignore
/// #[pvm_contract::fallback]
/// pub fn fallback() -> Result<(), Error> {
///     Err(Error::UnknownSelector)
/// }
/// ```
///
/// Must return `Result<(), Error>`. Commonly used to reject unknown calls.
///
/// # Payable Enforcement
///
/// By default, a fallback is non-payable and the generated code reverts if
/// value is attached to the call:
///
/// ```ignore
/// // Generated for a non-payable fallback (shared helper reads value + reverts):
/// __pvm_assert_non_payable(this.host());
/// ```
///
/// To accept value in the fallback, add `#[payable]`:
///
/// ```ignore
/// #[pvm_contract::fallback]
/// #[pvm_contract::payable]
/// pub fn fallback() -> Result<(), Error> { Ok(()) }
/// ```
#[proc_macro_attribute]
pub fn fallback(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    match codegen::expand_fallback(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Marks a function as the receive handler — invoked when the contract is
/// called with empty calldata (plain value transfers).
///
/// Mirrors Solidity's `receive() external payable`. The receive function is
/// implicitly `payable`: there is no such thing as a non-payable receive,
/// so adding `#[payable]` is a compile error. Must take `&mut self`, take no
/// other arguments, and return either `()` or `Result<(), Error>`.
///
/// Dispatch precedence on empty calldata:
/// 1. `#[receive]` fires if defined.
/// 2. Otherwise, the call falls through to `#[fallback]` (which must be
///    `#[payable]` if value is attached).
/// 3. Otherwise, the call reverts.
///
/// # Example
///
/// ```ignore
/// #[pvm_contract::receive]
/// pub fn receive(&mut self) {
///     // value already credited; record receipt, emit event, etc.
/// }
/// ```
///
/// Fallible form:
///
/// ```ignore
/// #[pvm_contract::receive]
/// pub fn receive(&mut self) -> Result<(), MyError> {
///     Ok(())
/// }
/// ```
#[proc_macro_attribute]
pub fn receive(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    match codegen::expand_receive(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Marks a contract entry point as payable — it accepts non-zero `msg.value`.
///
/// Applies to `#[method]`, `#[constructor]`, and `#[fallback]`. Without
/// `#[payable]`, the generated dispatch rejects any call carrying value with
/// `NonPayableValueReceived`. The attribute is a marker scanned by `#[contract]`
/// and produces no code on its own.
///
/// # Example
///
/// ```ignore
/// #[pvm_contract_macros::method]
/// #[pvm_contract_macros::payable]
/// pub fn deposit() {
///     let mut buf = [0u8; 32];
///     pallet_revive_uapi::HostFnImpl::value_transferred(&mut buf);
///     let amount = ruint::aliases::U256::from_le_bytes(buf);
///     // ...
/// }
/// ```
///
/// When a `.sol` interface is supplied, the Rust attribute must agree with the
/// Solidity `payable` keyword; a mismatch is a compile error.
#[proc_macro_attribute]
pub fn payable(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Marker attribute: `#[contract]` scans for its presence at expansion time
    // and then strips it. Passing the function through unchanged is enough.
    item
}

/// Marks a `#[method]` as reentrancy-guarded (OpenZeppelin `nonReentrant`).
///
/// Mode is inferred from the receiver: `&mut self` emits a full guard
/// (check + set + clear the lock); `&self` emits a read-only check
/// (`nonReentrantView`). A guarded method reverts with the OZ-compatible
/// `ReentrancyGuardReentrantCall` error on re-entry.
///
/// Only relevant for contracts that opt into `CallFlags::ALLOW_REENTRY`;
/// pallet-revive rejects reentrancy by default otherwise.
///
/// The lock lives in transient storage (EIP-1153) and is released on every exit
/// path, including a body that diverges via a raw `self.host().return_value(..)`:
/// the SDK clears it inside `return_value` (the one function every exit routes
/// through) when the current frame holds it. So a guarded method may exit either
/// by returning normally or via a raw `return_value` without leaving the lock set
/// for the rest of the transaction.
#[proc_macro_attribute]
pub fn non_reentrant(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Marker attribute: `#[contract]` scans for its presence at expansion time.
    item
}

/// Derives ABI encoding/decoding methods for a struct, enabling it to be used
/// as a parameter or return type in contract methods.
///
/// # Generated Traits
///
/// This derive macro generates implementations for both:
/// - `SolEncode` - Base trait with `encode_body_len()` and `encode_body_to()` methods
/// - `StaticEncodedLen` - Marker trait with compile-time `ENCODED_SIZE` constant
///
/// Types with only static fields implement `StaticEncodedLen` and can be returned from methods
/// in both allocator and default stack modes since they have a compile-time known size.
///
/// # Generated Code
///
/// For this struct:
///
/// ```ignore
/// use ruint::aliases::U256;
/// #[derive(pvm_contract_macros::SolType)]
/// pub struct Point {
///     pub x: U256,
///     pub y: U256,
/// }
/// ```
///
/// The macro generates implementations for ABI traits:
///
/// ```ignore
/// impl ::pvm_contract_sdk::SolEncode for Point {
///     const IS_DYNAMIC: bool = false;
///     const SOL_NAME: &'static str = "(uint256,uint256)";
///     const HEAD_SIZE: usize = 64;
///
///     fn encode_body_len(&self) -> usize { 64 }
///
///     fn encode_body_to(&self, buf: &mut [u8]) {
///         let mut __offset: usize = 0;
///         ::pvm_contract_sdk::SolEncode::encode_body_to(&self.x, &mut buf[__offset..]);
///         __offset += <U256 as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE;
///         ::pvm_contract_sdk::SolEncode::encode_body_to(&self.y, &mut buf[__offset..]);
///         __offset += <U256 as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE;
///     }
/// }
///
/// impl ::pvm_contract_sdk::StaticEncodedLen for Point {
///     const ENCODED_SIZE: usize = 64;
/// }
///
/// impl ::pvm_contract_sdk::SolDecode for Point {
///     fn decode_at(input: &[u8], offset: usize) -> Self {
///         let mut __offset: usize = 0;
///         let __field_x = {
///             let __val = <U256 as ::pvm_contract_sdk::SolDecode>::decode_at(
///                 input, offset + __offset);
///             __offset += <U256 as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE;
///             __val
///         };
///         let __field_y = {
///             let __val = <U256 as ::pvm_contract_sdk::SolDecode>::decode_at(
///                 input, offset + __offset);
///             __offset += <U256 as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE;
///             __val
///         };
///         Self { x: __field_x, y: __field_y }
///     }
/// }
///
/// impl ::pvm_contract_sdk::SolArrayElement for Point {}
/// ```
///
/// # Usage in Contract Methods
///
/// ```ignore
/// #[pvm_contract_macros::method]
/// pub fn get_point() -> Point {
///     // Macro calls SolEncode::encode_to() automatically
///     Point { x: U256::from(10), y: U256::from(20) }
/// }
/// ```
///
/// # Supported Field Types
///
/// | Rust Type | Solidity Type | Encoded Size |
/// |-----------|---------------|--------------|
/// | `U256` | `uint256` | 32 bytes |
/// | `u128` | `uint128` | 32 bytes |
/// | `u64` | `uint64` | 32 bytes |
/// | `u32` | `uint32` | 32 bytes |
/// | `u16` | `uint16` | 32 bytes |
/// | `u8` | `uint8` | 32 bytes |
/// | `I256` | `int256` | 32 bytes |
/// | `i128` | `int128` | 32 bytes |
/// | `i64` | `int64` | 32 bytes |
/// | `i32` | `int32` | 32 bytes |
/// | `i16` | `int16` | 32 bytes |
/// | `i8` | `int8` | 32 bytes |
/// | `bool` | `bool` | 32 bytes |
/// | `Address` | `address` | 32 bytes |
/// | `[u8; N]` (N <= 32) | `bytesN` | 32 bytes |
/// | `[T; N]` | `T[N]` | N * element size |
/// | `Bytes` | `bytes` | dynamic |
/// | `Vec<T>` | `T[]` | dynamic |
/// | `&[T]` | `T[]` | dynamic |
/// | `String` | `string` | dynamic |
/// | `&str` | `string` | dynamic |
/// | Other `SolType` struct | tuple | sum of field sizes |
///
/// # Static vs Dynamic Structs
///
/// Structs with only static fields implement `SolEncode`, `StaticEncodedLen`, and `SolDecode`.
/// Structs with any dynamic fields (like `String`) implement `SolEncode` and `SolDecode`.
///
/// ```ignore
/// // Static struct - implements both traits
/// #[derive(SolType)]
/// pub struct Point { pub x: U256, pub y: U256 }
///
/// // Dynamic struct - implements only SolEncode
/// #[derive(SolType)]
/// pub struct User { pub name: String, pub age: u8 }
/// ```
///
/// Dynamic structs can only be returned in allocator mode (compile error in default stack mode).
///
/// ## Generated Code for Dynamic Structs
///
/// For a dynamic struct like `User { name: String, age: u8 }`, the macro generates:
///
/// ```ignore
/// impl ::pvm_contract_sdk::SolEncode for User {
///     const IS_DYNAMIC: bool = true;
///     const SOL_NAME: &'static str = "(string,uint8)";
///     const HEAD_SIZE: usize = 64;  // 32 (offset pointer for String) + 32 (u8 slot)
///
///     fn encode_body_len(&self) -> usize {
///         64 + ::pvm_contract_sdk::SolEncode::encode_body_len(&self.name)
///     }
///
///     fn encode_body_to(&self, buf: &mut [u8]) {
///         let __head_size: usize = 64;
///         let mut __tail_offset: usize = __head_size;
///
///         // Field 0 (name: String) — dynamic, write offset pointer
///         buf[0..24].fill(0);
///         buf[24..32].copy_from_slice(&(__tail_offset as u64).to_be_bytes());
///         let __tail_len = ::pvm_contract_sdk::SolEncode::encode_body_len(&self.name);
///         ::pvm_contract_sdk::SolEncode::encode_body_to(
///             &self.name,
///             &mut buf[__tail_offset..__tail_offset + __tail_len]
///         );
///         __tail_offset += __tail_len;
///
///         // Field 1 (age: u8) — static, write inline
///         <u8 as ::pvm_contract_sdk::SolEncode>::encode_body_to(
///             &self.age, &mut buf[32..64]);
///     }
/// }
///
/// impl ::pvm_contract_sdk::SolDecode for User {
///     fn decode_at(input: &[u8], offset: usize) -> Self { /* ... */ }
///     fn decode_tail(input: &[u8], offset: usize) -> Self {
///         Self::decode_at(input, offset)
///     }
/// }
///
/// impl ::pvm_contract_sdk::SolArrayElement for User {}
/// ```
///
#[proc_macro_derive(SolType)]
pub fn sol_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match codegen::expand_sol_type(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derive the storage-layout traits ([`StorageEncode`], [`StorageDecode`],
/// and the `StaticStorageEncode`/`StaticStorageDecode` refinement for static
/// structs) for a struct that can be used as a `Lazy<S>` / `Mapping<_, S>`
/// value. Also derives the leaf `StorageType` + `SimpleStorageType`
/// (`Get<'a> = Self`, `GetMut<'a> = Lazy<Self>`), so the struct composes as a
/// by-value element of `Mapping<K, S>` / `StorageVec<S>` — `get` returns the
/// value, `insert`/`push` take it by value.
///
/// This derive is **separate from `#[derive(SolType)]`** — `SolType` covers
/// ABI encoding (calldata, return values, event fields) and is meaningful
/// for any struct, while `SolStorage` covers the solc-compatible on-chain
/// storage layout and only makes sense for structs that will live in
/// contract storage. Most user structs that go in storage will derive both:
///
/// ```ignore
/// #[derive(SolType, SolStorage)]
/// struct AccountInfo {
///     addr: Address,
///     balance: U256,
/// }
/// ```
///
/// If any field is not yet storage-compatible (nested SolType structs,
/// `Vec<T>` — use `Bytes` for `bytes`-shaped values, fixed arrays of
/// non-`u8`, tuples), the derive
/// emits a `compile_error!` at expansion time — visible to `cargo check`
/// and `trybuild`, unlike the prior `const STORAGE_SLOTS = panic!(...)`
/// stub that only fired during MIR const-eval at `cargo build` time.
///
/// [`StorageEncode`]: pvm_contract_sdk::StorageEncode
/// [`StorageDecode`]: pvm_contract_sdk::StorageDecode
#[proc_macro_derive(SolStorage)]
pub fn sol_storage(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match codegen::expand_sol_storage(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derive the [`SolError`] trait for a struct, enabling Solidity-compatible
/// ABI-encoded revert data.
///
/// Generates `SELECTOR` (compile-time keccak256), `SIGNATURE`, and
/// `encode_params` from the struct fields. Each field must implement
/// [`pvm_contract_types::SolEncode`].
///
/// # Example
///
/// ```ignore
/// #[derive(SolError)]
/// pub struct InsufficientBalance {
///     pub account: Address,
///     pub required: U256,
///     pub available: U256,
/// }
/// ```
///
/// Zero-field errors are valid:
///
/// ```ignore
/// #[derive(SolError)]
/// pub struct Unauthorized;
/// ```
#[proc_macro_derive(SolError)]
pub fn sol_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match codegen::expand_sol_error(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Generates bindings to interact with a contract interface using either a:
/// - solidity literal that has the defined inteface of said contract.
/// - json abi file as a path and a name for said contract.
///
/// # Supported methods
///
/// - delegate call
/// - call
/// - instantiation via `new` function.
///
/// # Supported attributes
///
/// - #[abi_import(alloc = <true/false>)] - higher level bindings and dynamic type support, default value is [false].
///
/// # Support for custom types
///
/// - structs: present
/// - errors: present
/// - udts: present
/// - enums: currently not supported
///
/// # Example of usage
/// - `solidity` literal
///
/// ```ignore
/// pvm_contract_macros::abi_import! {
///     #![abi_import(alloc = true)]
///     // SPDX-License-Identifier: MIT
///     pragma solidity ^0.8.0;
///     interface Flipper {
///         function flip() external;
///         function get() external view returns (bool);
///     }
/// }
/// ```
///
/// - `json` api
/// ```text
/// abi_import! {
///     #![abi_import(alloc = true)]
///     Contract,
///     concat!(env!("CARGO_MANIFEST_DIR"), "/path/to/MyJsonContract.abi.json"))
/// }
/// ```
///
/// # Name Matching
///
/// Solidity function names are converted to snake_case for compatibility:
/// - `totalSupply` → `total_supply`
/// - `balanceOf` → `balance_of`
///
/// # Function overloading inside abi
///
/// in case of function overloading inside abi a-la:
/// ```solidity
///    function flip() external;
///    function flip(bool a) external;
/// ```
/// the folowing methods will be generated:
/// ```text
///    fn flip(&mut self) -> ...
///    fn flip_1(&mut self, a: bool) -> ...
/// ```
///
/// # Alloc enabled api examples
///
/// #![abi_import(alloc = true)] enables a higher level api.
/// example below:
///
/// ```text
/// pvm_contract_macros::abi_import! {
///     #![abi_import(alloc = true)]
///     // SPDX-License-Identifier: MIT
///     pragma solidity ^0.8.0;
///     interface Flipper {
///         constructor();
///         function flip() payable external;
///         function get() external view returns (bool);
///     }
/// }
///
/// ...
///
/// // The call builders take the contract itself (`&impl ContractContext` for a
/// // `view`/`pure` callee, `&mut impl ContractContext` otherwise) — pass `self`,
/// // not `self.host()`. That borrow is what stops a `&self` method from
/// // initiating a state-mutating call.
/// fn example(&mut self) {
///     use flipper::*;
///     // call a contract
///     let bool: bool = Flipper::from_address(<addr>).get().call(self)?;
///     // set a `value` this method is only present if the method is `payable`.
///     // also its possible to set a limit for the call.
///     let _ = Flipper::from_address(<addr>).set_value(5).set_call_limits(CallLimits::GasLimit(u64::MAX)).flip().call(self)?;
///
///     // instantiate a contract
///     let (address, <return_value>): (Address, ()) = Flipper::new().instantiate(self, <code_hash>, <value>, <limits>, <optional salt>)?;
/// }
/// ```
///
/// # Further Documentation
/// Please refer to:
/// - [`pvm_contract_core::call::CallError`] for errors
/// - [`pvm_contract_core::call::CallLimits`] for call limits
#[proc_macro]
pub fn abi_import(input: TokenStream) -> TokenStream {
    let (file, alloc, sol_path) = parse_macro_input!(input with abi_import::parse::parse_macro);

    // Conversion boundary (whole output): failures that invalidate the entire
    // invocation (`import` statements, duplicate type names). Per-function and
    // per-type-item failures are converted inside `expand_to_module` instead,
    // so sibling items keep expanding.
    abi_import::expand_to_module(&file, alloc, sol_path.as_deref())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Derive the [`SolEvent`] trait for a struct, enabling Solidity-compatible
/// event emission with automatic topic hashing and indexed field packing.
/// No allocator required.
///
/// Fields marked with `#[indexed]` become log topics (max 3, or 4 for anonymous
/// events). Remaining fields are ABI-encoded as the log data blob. The event
/// signature hash is computed at compile time as topic0 (skipped for `#[anonymous]`).
///
/// Indexed static arrays, fixed arrays, and tuples use `keccak256(abi.encode(value))`.
/// Indexed dynamic composites and dynamic arrays (`Vec<T>`) are rejected at
/// compile time. Custom and alias types are not supported as indexed fields.
///
/// For events where all non-indexed fields are known-static primitive types,
/// the derive generates an `emit(host)` convenience method with a stack buffer.
/// For events with dynamic fields (e.g. `String`), add `#[alloc]` to generate
/// an alloc-backed `emit()`, or use `data_len()` + `data_to()` manually.
///
/// # Example
///
/// ```ignore
/// // Static event: emit() generated automatically.
/// #[derive(SolEvent)]
/// struct Transfer {
///     #[indexed]
///     from: Address,
///     #[indexed]
///     to: Address,
///     value: U256,
/// }
/// Transfer { from, to, value }.emit(self.host());
///
/// // Dynamic event with #[alloc]: emit() uses heap allocation.
/// #[derive(SolEvent)]
/// #[alloc]
/// struct Log {
///     message: String,
/// }
/// Log { message }.emit(self.host());
/// ```
#[proc_macro_derive(SolEvent, attributes(indexed, anonymous, alloc))]
pub fn sol_event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match codegen::expand_sol_event(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

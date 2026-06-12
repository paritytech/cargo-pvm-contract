#![doc = include_str!("../../../specs/proc-macros.md")]

extern crate proc_macro2;

mod abi_import;
mod codegen;
mod signature;
mod utils;
use proc_macro::TokenStream;
use syn::{DeriveInput, ItemFn, ItemMod, parse_macro_input};

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
/// - `pub fn route(this: &mut Contract, selector: [u8; 4], input: &[u8])
///   -> Option<()>` — selector dispatch. Each matched arm encodes the
///   result and calls `this.host().return_value(flags, data)` directly:
///   `-> !` (diverging syscall) on `riscv64`, captured into `MockHost`
///   on host targets. Returns `Some(())` on a matched selector and `None`
///   on no match (caller can chain or revert).
/// - `pub extern "C" fn deploy()` — PolkaVM deploy entry point (riscv64-only)
/// - `pub extern "C" fn call()` — PolkaVM call entry point (riscv64-only);
///   reads calldata, calls `route()`, falls through to fallback or
///   `return_value(REVERT, UNKNOWN_SELECTOR)` when `route()` returns `None`.
///
/// Outside the module, a `Router` trait impl is generated:
///
/// ```ignore
/// impl ::pvm_contract_sdk::Router for my_token::Contract {
///     fn route(
///         &mut self,
///         selector: [u8; 4],
///         input: &[u8],
///     ) -> ::core::option::Option<()> {
///         my_token::route(self, selector, input)
///     }
/// }
/// ```
///
/// The contract holds a concrete `Host` whose internals are cfg-gated:
/// on riscv64 it's a zero-sized type wrapping `PolkaVmHost` (zero overhead), on the
/// host target it wraps `Rc<dyn HostApi>` so tests can inject a `MockHost`.
/// `HostApi::return_value` itself has a cfg-gated signature: `-> !` on
/// `riscv64` (the `pallet_revive_uapi` syscall), `-> ()` on host targets
/// (captures into `MockHost`). The generated dispatch code has no
/// `cfg(target_arch)` gate — the same path serves production and native
/// unit tests.
///
/// All generated items are gated behind `#[cfg(not(feature = "abi-gen"))]`.
///
/// ### Composition and inheritance
///
/// `route()` returns `Option<()>` — `Some(())` if the selector matched (and
/// the arm has already called `return_value`, which on `riscv64` means
/// execution has terminated); `None` if the selector did not match. Chain
/// multiple routers via `Option::or_else`:
///
/// ```ignore
/// pub extern "C" fn call() {
///     let (selector, input) = read_calldata();
///     if my_extension::route(&mut this, selector, input).is_some() { return; }
///     if erc20_base::route(&mut this, selector, input).is_some() { return; }
///     // fallback or revert
///     HostFnImpl::return_value(ReturnFlags::REVERT, &UNKNOWN_SELECTOR);
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
/// let mut contract = my_token::Contract::with_host(mock.clone());
/// let bal = contract.balance_of(account);
/// assert_eq!(bal, U256::from(42));
/// ```
///
/// The macro generates `Contract::with_host(backend)` — wraps any
/// `HostApi` implementor in `Rc<dyn HostApi>` and initialises `#[slot(N)]`
/// fields. Mirrors the std-lib `Vec::with_capacity` idiom for
/// "constructor with a non-default dependency." The user's
/// `#[constructor]` is NOT run — seed storage on the mock directly if
/// you need initial state.
///
/// **Dispatch-level** (selector routing, ABI revert encoding) — drive
/// `route()` with raw calldata and read the captured `ReturnValue`:
///
/// ```ignore
/// let outcome = my_token::route(&mut contract, BALANCE_OF_SELECTOR, &input);
/// assert_eq!(outcome, Some(())); // selector matched
/// let rv = mock.take_return_value().expect("contract called return_value");
/// assert_eq!(rv.flags, ReturnFlags::empty());
/// // decode and assert on rv.data
/// ```
///
/// `take_return_value` consumes the capture so each `route()` call must be
/// followed by exactly one `take_return_value()` — stale state cannot leak
/// across calls on the same mock.
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
///     }
///
///     #[pvm_contract::method]
///     #[pvm_contract::payable]
///     pub fn deposit(to: Address) { /* read value via api::value_transferred */ }
///
///     #[pvm_contract::constructor]
///     pub fn new() -> Result<(), Error> { Ok(()) }
///
///     // --- Generated inside the module: ---
///
///     pub fn route(
///         this: &mut Contract,
///         selector: [u8; 4],
///         input: &[u8],
///     ) -> ::core::option::Option<()> {
///         // Value-transfer hoist — read once, used by all non-payable arms
///         let mut __value_buf = [0u8; 32];
///         this.host().value_transferred(&mut __value_buf);
///         let __has_value = __value_buf != [0u8; 32];
///
///         // Selector consts — precomputed from .sol, or derived via SOL_NAME
///         const __SEL_balance_of: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
///         const __SEL_transfer: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
///         const __SEL_deposit: [u8; 4] = /* keccak("deposit(address)")[..4] */;
///
///         match selector {
///             // balanceOf(address) -> uint256  (non-payable)
///             __SEL_balance_of => {
///                 if __has_value {
///                     this.host().return_value(
///                         ::pvm_contract_sdk::ReturnFlags::REVERT,
///                         &::pvm_contract_sdk::framework_errors::NON_PAYABLE_VALUE_RECEIVED);
///                 }
///                 if input.len() < <Address as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE {
///                     this.host().return_value(
///                         ::pvm_contract_sdk::ReturnFlags::REVERT,
///                         &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA);
///                     return ::core::option::Option::Some(());
///                 }
///                 let mut __decode_offset: usize = 0;
///                 let account = /* decode … */;
///                 let result = this.balance_of(::core::convert::Into::into(account));
///                 const __LEN: usize =
///                     <U256 as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE;
///                 let mut __buf = [0u8; __LEN];
///                 <U256 as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf);
///                 this.host().return_value(
///                     ::pvm_contract_sdk::ReturnFlags::empty(), &__buf);
///                 return ::core::option::Option::Some(());
///             }
///
///             // transfer(address,uint256) — fallible, non-payable
///             __SEL_transfer => {
///                 if __has_value {
///                     this.host().return_value(
///                         ::pvm_contract_sdk::ReturnFlags::REVERT,
///                         &::pvm_contract_sdk::framework_errors::NON_PAYABLE_VALUE_RECEIVED);
///                 }
///                 // ... size check + decode ...
///                 match this.transfer(
///                     ::core::convert::Into::into(to),
///                     ::core::convert::Into::into(amount),
///                 ) {
///                     Ok(()) => {
///                         this.host().return_value(
///                             ::pvm_contract_sdk::ReturnFlags::empty(), &[]);
///                         return ::core::option::Option::Some(());
///                     }
///                     Err(e) => {
///                         let mut __revert_buf = [0u8; 256];
///                         let __revert_len =
///                             e.encode_to(&mut __revert_buf);
///                         this.host().return_value(
///                             ::pvm_contract_sdk::ReturnFlags::REVERT,
///                             &__revert_buf[..__revert_len]);
///                         return ::core::option::Option::Some(());
///                     }
///                 }
///             }
///
///             // deposit(address) — payable: no __has_value guard
///             __SEL_deposit => {
///                 // ... size check + decode `to` ...
///                 this.deposit(::core::convert::Into::into(to));
///                 return ::core::option::Option::Some(());
///             }
///
///             _ => ::core::option::Option::None,
///         }
///     }
///
///     #[polkavm_derive::polkavm_export]
///     pub extern "C" fn deploy() {
///         // Non-payable constructor: reject value
///         let mut __value_buf = [0u8; 32];
///         pallet_revive_uapi::HostFnImpl::value_transferred(&mut __value_buf);
///         let __has_value = __value_buf != [0u8; 32];
///         if __has_value {
///             pallet_revive_uapi::HostFnImpl::return_value(
///                 pallet_revive_uapi::ReturnFlags::REVERT,
///                 &::pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED);
///         }
///         // ... read constructor calldata, decode, call new() ...
///     }
///
///     #[polkavm_derive::polkavm_export]
///     pub extern "C" fn call() {
///         let host = ::pvm_contract_sdk::Host::new();
///         let mut this = Contract {
///             // #[slot(N)] fields would be initialised here with
///             // field: <Type>::new(StorageKey::from_slot(N), host.clone()),
///             host,
///         };
///         let call_data_len = HostFnImpl::call_data_size() as usize;
///         let mut call_data = [0u8; 512];
///         if call_data_len > 512 {
///             HostFnImpl::return_value(ReturnFlags::REVERT,
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
///             HostFnImpl::return_value(ReturnFlags::REVERT,
///                 &::pvm_contract_sdk::framework_errors::NO_SELECTOR);
///         }
///
///         let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
///         let input = &call_data[4..call_data_len];
///
///         // route() either calls return_value (diverges on riscv64) or returns
///         // None for an unmatched selector. Falling through means: unmatched.
///         if route(&mut this, selector, input).is_none() {
///             // With #[fallback]: calls fallback. Without: UnknownSelector.
///             HostFnImpl::return_value(ReturnFlags::REVERT,
///                 &::pvm_contract_sdk::framework_errors::UNKNOWN_SELECTOR);
///         }
///     }
/// }
///
/// // Generated outside the module:
/// impl ::pvm_contract_sdk::Router for my_token::Contract {
///     fn route(
///         &mut self,
///         selector: [u8; 4],
///         input: &[u8],
///     ) -> ::core::option::Option<()> {
///         my_token::route(self, selector, input)
///     }
/// }
/// ```
///
/// ### Allocator mode
///
/// The only difference is buffer allocation in `call()`:
///
/// ```ignore
/// let mut call_data = alloc::vec![0u8; call_data_len];
/// ```
///
/// The `route()` function and dispatch logic are identical.
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
/// storage struct.
///
/// Field slots are auto-numbered in declaration order; the embedded struct's
/// `SLOTS` is the sum of its fields' `SLOTS`. The contract struct's
/// auto-numbering uses these `SLOTS` constants to assign contiguous ranges,
/// so embedding nests cleanly without manual slot math.
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
/// let mut __value_buf = [0u8; 32];
/// pallet_revive_uapi::HostFnImpl::value_transferred(&mut __value_buf);
/// let __has_value = __value_buf != [0u8; 32];
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
/// // 0) Non-payable guard — revert if value was transferred
/// if __has_value {
///     pallet_revive_uapi::HostFnImpl::return_value(
///         pallet_revive_uapi::ReturnFlags::REVERT,
///         &::pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED);
/// }
///
/// // 1) Decode input parameters (uniform trait dispatch)
/// let mut __decode_offset: usize = 0;
/// let account = {
///     let __value = <Address as ::pvm_contract_sdk::SolDecode>::decode_at(
///         &input, __decode_offset);
///     __decode_offset += <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE;
///     __value
/// };
///
/// // 2) Call the method (no module prefix — generated inside the module)
/// let result = balance_of(::core::convert::Into::into(account));
///
/// // 3) Encode and return via encode_to (smart top-level encoding)
/// let mut __buf = [0u8; <U256 as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE];
/// <U256 as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf);
/// ::pvm_contract_sdk::PolkaVmHost::return_value(
///     ::pvm_contract_sdk::ReturnFlags::empty(), &__buf);
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
/// if input.len() < <Address as ::pvm_contract_types::SolEncode>::HEAD_SIZE {
///     pallet_revive_uapi::HostFnImpl::return_value(
///         pallet_revive_uapi::ReturnFlags::REVERT,
///         &::pvm_contract_types::framework_errors::INVALID_CALLDATA);
/// }
/// let mut __decode_offset: usize = 0;
/// let to = {
///     let __value = <Address as ::pvm_contract_types::SolDecode>::decode_at(
///         &input, __decode_offset);
///     __decode_offset += <Address as ::pvm_contract_types::SolEncode>::HEAD_SIZE;
///     __value
/// };
///
/// deposit(::core::convert::Into::into(to));
/// ```
///
/// ## Return encoding (alloc mode)
///
/// In alloc mode, the generated code uses a compile-time `IS_DYNAMIC` branch.
/// Static types use a stack buffer; dynamic types (String, `Vec<T>`, `Bytes`)
/// use heap allocation. The compiler eliminates the dead branch at compile time:
///
/// ```ignore
/// #[pvm_contract::method]
/// pub fn greeting() -> String { ... }
///
/// // Generated dispatch arm (in alloc mode, inside route()):
///
/// let result = greeting();
///
/// let __len = <String as ::pvm_contract_sdk::SolEncode>::encode_len(&result);
/// if <String as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC {
///     let mut __buf = alloc::vec![0u8; __len];
///     <String as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf);
///     ::pvm_contract_sdk::PolkaVmHost::return_value(
///         ::pvm_contract_sdk::ReturnFlags::empty(), &__buf);
/// } else {
///     let mut __buf = [0u8; <String as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE];
///     <String as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf[..__len]);
///     ::pvm_contract_sdk::PolkaVmHost::return_value(
///         ::pvm_contract_sdk::ReturnFlags::empty(), &__buf[..__len]);
/// }
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
/// Constructor that can revert:
///
/// ```ignore
/// #[pvm_contract::constructor]
/// pub fn new() -> Result<(), Error> {
///     set_owner(pvm_contract::caller());
///     Ok(())
/// }
/// ```
///
/// Constructor that never reverts:
///
/// ```ignore
/// #[pvm_contract::constructor]
/// pub fn new() {
///     set_owner(pvm_contract::caller());
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
/// // Generated for a non-payable fallback:
/// let mut __value_buf = [0u8; 32];
/// pallet_revive_uapi::HostFnImpl::value_transferred(&mut __value_buf);
/// let __has_value = __value_buf != [0u8; 32];
/// if __has_value {
///     pallet_revive_uapi::HostFnImpl::return_value(
///         pallet_revive_uapi::ReturnFlags::REVERT,
///         &::pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED);
/// }
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
/// fn example() {
///     use flipper::*;
///     // call a contract
///     let bool: bool = Flipper::from_address(<addr>).get().call(self.host())?;
///     // set a `value` this method is only present if the method is `payable`.
///     // also its possible to set a limit for the call.
///     let _ = Flipper::from_address(<addr>).set_value(5).set_call_limits(CallLimits::GasLimit(u64::MAX)).flip().call(self.host())?;
///
///     // instantiate a contract
///     let (address, <return_value>): (Address, ()) = Flipper::new().instantiate(self.host(), <code_hash>, <value>, <limits>, <optional salt>)?;
/// }
/// ```
///
/// # Further Documentation
/// Please refer to:
/// - [`pvm_contract_core::call::CallError`] for errors
/// - [`pvm_contract_core::call::CallLimits`] for call limits
#[proc_macro]
pub fn abi_import(input: TokenStream) -> TokenStream {
    let (file, alloc) = parse_macro_input!(input with abi_import::parse::parse_macro);

    abi_import::expand_to_module(&file, alloc).into()
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

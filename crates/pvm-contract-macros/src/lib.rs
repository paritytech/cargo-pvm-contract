#![doc = include_str!("../../../specs/proc-macros.md")]

extern crate proc_macro;

mod codegen;
mod signature;
mod solidity;

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
/// use pvm_contract::{Address, U256};
///
/// #[pvm_contract::contract("MyToken.sol")]
/// mod my_token {
///     use super::*;
///
///     #[pvm_contract::constructor]
///     pub fn new() -> Result<(), Error> { Ok(()) }
///
///     #[pvm_contract::method]
///     pub fn total_supply() -> U256 { U256::ZERO }
///
///     #[pvm_contract::method]
///     pub fn balance_of(_account: Address) -> U256 { U256::ZERO }
///
///     #[pvm_contract::method]
///     pub fn transfer(to: Address, amount: U256) -> Result<(), Error> { Ok(()) }
///
///     #[pvm_contract::fallback]
///     pub fn fallback() -> Result<(), Error> { Err(Error::UnknownSelector) }
/// }
/// ```
///
/// # Usage without Solidity Interface
///
/// You can also define contracts without a `.sol` file. Signatures are inferred from Rust types:
///
/// ```ignore
/// use pvm_contract::{Address, U256};
///
/// #[pvm_contract::contract]
/// mod my_token {
///     use super::*;
///
///     #[pvm_contract::constructor]
///     pub fn new() -> Result<(), Error> { Ok(()) }
///
///     #[pvm_contract::method]
///     pub fn total_supply() -> U256 { U256::ZERO }
///
///     #[pvm_contract::method]
///     pub fn balance_of(account: Address) -> U256 { U256::ZERO }
///
///     #[pvm_contract::method]
///     pub fn transfer(to: Address, amount: U256) -> Result<(), Error> { Ok(()) }
///
///     #[pvm_contract::fallback]
///     pub fn fallback() -> Result<(), Error> { Err(Error::UnknownSelector) }
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
/// ## Entry Points
///
/// The macro generates two PolkaVM entry points:
///
/// ```ignore
/// #[no_mangle]
/// #[polkavm_derive::polkavm_export]
/// pub extern "C" fn deploy() { /* constructor logic */ }
///
/// #[no_mangle]
/// #[polkavm_derive::polkavm_export]
/// pub extern "C" fn call() { /* dispatch logic */ }
/// ```
///
/// ## Error Type
///
/// The scaffold generates an empty `Error` enum inside the contract module.
/// You are expected to add your own error variants as needed:
///
/// ```ignore
/// mod my_token {
///     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
///     pub enum Error {
///         // Add your errors here:
///         InsufficientBalance,
///         Unauthorized,
///     }
///
///     impl AsRef<[u8]> for Error {
///         fn as_ref(&self) -> &[u8] {
///             match self {
///                 Self::InsufficientBalance => b"InsufficientBalance",
///                 Self::Unauthorized => b"Unauthorized",
///             }
///         }
///     }
///     // ... methods
/// }
/// ```
///
/// ## Dispatch Logic
///
/// stack and allocator modes use the same direct dispatch logic.
/// The only difference is buffer allocation:
///
/// - **allocator mode**: `let mut call_data = vec![0u8; call_data_len];`
/// - **default stack mode**: `let mut call_data = [0u8; BUFFER_SIZE];` with overflow check
///
/// ### default stack generated `call()` example
///
/// ```ignore
/// #[pvm_contract_macros::contract("MyToken.sol", buffer = 512)]
/// mod my_token {
///     // Infallible method (no Result wrapper)
///     #[pvm_contract::method]
///     pub fn balance_of(account: Address) -> U256 { U256::ZERO }
///
///     // Fallible method (returns Result)
///     #[pvm_contract::method]
///     pub fn transfer(to: Address, amount: U256) -> Result<(), Error> { Ok(()) }
/// }
///
/// // Generates:
/// #[polkavm_derive::polkavm_export]
/// pub extern "C" fn call() {
///     let call_data_len = pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
///     let mut call_data = [0u8; 512];
///
///     if call_data_len > 512 {
///         pallet_revive_uapi::HostFnImpl::return_value(
///             pallet_revive_uapi::ReturnFlags::REVERT, b"CalldataTooLarge");
///     }
///     pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data[..call_data_len], 0);
///
///     if call_data_len < 4 { /* fallback handling */ }
///
///     let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
///     let input = &call_data[4..call_data_len];
///
///     // Selector consts (computed at compile time)
///     const __SEL_balance_of: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
///     const __SEL_transfer: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
///
///     match selector {
///         // balanceOf(address) -> uint256 - Infallible method
///         __SEL_balance_of => {
///             // Decode parameters
///             let account = <::pvm_contract_types::Address as ::pvm_contract_types::SolDecode>::decode(&input);
///
///             // Call the method (args wrapped with Into::into for type coercion)
///             let result = my_token::balance_of(::core::convert::Into::into(account));
///
///             // Encode return value (compile-time buffer via StaticEncodedLen)
///             let encoded = {
///                 let mut __buf = [0u8; <ruint::aliases::U256 as ::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE];
///                 <ruint::aliases::U256 as ::pvm_contract_types::SolEncode>::encode_to(
///                     &::core::convert::Into::into(result), &mut __buf);
///                 __buf
///             };
///             pallet_revive_uapi::HostFnImpl::return_value(
///                 pallet_revive_uapi::ReturnFlags::empty(), &encoded);
///         }
///
///         // transfer(address,uint256) - Fallible method
///         __SEL_transfer => {
///             // Decode parameters
///             let to = <::pvm_contract_types::Address as ::pvm_contract_types::SolDecode>::decode(&input);
///             let amount = <ruint::aliases::U256 as ::pvm_contract_types::SolDecode>::decode_at(&input, 32);
///
///             // Call method and handle Result
///             match my_token::transfer(::core::convert::Into::into(to), ::core::convert::Into::into(amount)) {
///                 Ok(()) => return,
///                 Err(e) => {
///                     pallet_revive_uapi::HostFnImpl::return_value(
///                         pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
///                 }
///             }
///         }
///
///         _ => { /* fallback */ }
///     }
/// }
/// ```
///
/// ### allocator generated `call()` example
///
/// ```ignore
/// #[pvm_contract_macros::contract("MyToken.sol", allocator = "pico")]
/// mod my_token {
///     // methods...
/// }
///
/// // Generates:
/// #[polkavm_derive::polkavm_export]
/// pub extern "C" fn call() {
///     let call_data_len = pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
///     let mut call_data = alloc::vec![0u8; call_data_len];
///     pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data[..], 0);
///
///     if call_data_len < 4 { /* fallback handling */ }
///
///     let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
///     let input = &call_data[4..];
///
///     match selector {
///         [0x70, 0xa0, 0x82, 0x31] => { /* dispatch arm */ }
///         [0xa9, 0x05, 0x9c, 0xbb] => { /* dispatch arm */ }
///         _ => { /* fallback */ }
///     }
/// }
/// ```
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
/// ## Static return (U256)
///
/// Types implementing `StaticEncodedLen` use compile-time buffer sizing:
///
/// ```ignore
/// #[pvm_contract::method]
/// pub fn balance_of(account: Address) -> U256 { ... }
///
/// // Generated dispatch arm:
///
/// // 1) Decode input parameters
/// let account = <::pvm_contract_types::Address as ::pvm_contract_types::SolDecode>::decode(&input);
///
/// // 2) Call the method (args wrapped with Into::into)
/// let result = my_token::balance_of(::core::convert::Into::into(account));
///
/// // 3) Encode output (compile-time buffer via StaticEncodedLen)
/// let encoded = {
///     let mut __buf = [0u8; <ruint::aliases::U256 as ::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE];
///     <ruint::aliases::U256 as ::pvm_contract_types::SolEncode>::encode_to(
///         &::core::convert::Into::into(result), &mut __buf);
///     __buf
/// };
///
/// // 4) Return value to caller
/// pallet_revive_uapi::HostFnImpl::return_value(
///     pallet_revive_uapi::ReturnFlags::empty(), &encoded);
/// ```
///
/// ## Dynamic return (alloc mode)
///
/// In alloc mode, dynamic types (String, `Vec<T>`) automatically use runtime buffer sizing:
///
/// ```ignore
/// #[pvm_contract::method]
/// pub fn greeting() -> String { ... }
///
/// // Generated dispatch arm (in alloc mode):
///
/// // 1) Call the method
/// let result = my_token::greeting();
///
/// // 2) Encode output (runtime buffer size)
/// let len = ::pvm_contract_types::SolEncode::encode_len(&result);
/// let mut buf = alloc::vec![0u8; len];
/// ::pvm_contract_types::SolEncode::encode_to(&result, &mut buf);
///
/// // 3) Return value to caller
/// pallet_revive_uapi::HostFnImpl::return_value(
///     pallet_revive_uapi::ReturnFlags::empty(), &buf);
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
#[proc_macro_attribute]
pub fn fallback(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    match codegen::expand_fallback(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derives ABI encoding/decoding methods for a struct, enabling it to be used
/// as a parameter or return type in contract methods.
///
/// # Generated Traits
///
/// This derive macro generates implementations for both:
/// - `SolEncode` - Base trait with `encode_len()` and `encode_to()` methods
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
/// impl ::pvm_contract_types::SolEncode for Point {
///     const IS_DYNAMIC: bool = false;
///     const SOL_NAME: &'static str = "(uint256,uint256)";
///     const HEAD_SIZE: usize = 64;
///
///     fn encode_len(&self) -> usize { 64 }
///
///     fn encode_to(&self, buf: &mut [u8]) {
///         let mut __offset: usize = 0;
///         <ruint::aliases::U256 as ::pvm_contract_types::SolEncode>::encode_to(
///             &self.x, &mut buf[__offset..__offset + 32]);
///         __offset += 32;
///         <ruint::aliases::U256 as ::pvm_contract_types::SolEncode>::encode_to(
///             &self.y, &mut buf[__offset..__offset + 32]);
///         __offset += 32;
///     }
/// }
///
/// impl ::pvm_contract_types::StaticEncodedLen for Point {
///     const ENCODED_SIZE: usize = 64;
/// }
///
/// impl ::pvm_contract_types::SolDecode for Point {
///     fn decode_at(input: &[u8], offset: usize) -> Self {
///         Self {
///             x: <ruint::aliases::U256 as ::pvm_contract_types::SolDecode>::decode_at(input, offset),
///             y: <ruint::aliases::U256 as ::pvm_contract_types::SolDecode>::decode_at(input, offset + 32),
///         }
///     }
/// }
///
/// impl ::pvm_contract_types::SolArrayElement for Point {}
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
/// | `i128` | `int128` | 32 bytes |
/// | `i64` | `int64` | 32 bytes |
/// | `i32` | `int32` | 32 bytes |
/// | `i16` | `int16` | 32 bytes |
/// | `i8` | `int8` | 32 bytes |
/// | `bool` | `bool` | 32 bytes |
/// | `Address` | `address` | 32 bytes |
/// | `[u8; N]` (N <= 32) | `bytesN` | 32 bytes |
/// | `[T; N]` | `T[N]` | N * element size |
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
/// impl ::pvm_contract_types::SolEncode for User {
///     const IS_DYNAMIC: bool = true;
///     const SOL_NAME: &'static str = "(string,uint8)";
///     const HEAD_SIZE: usize = 64;  // 32 (offset pointer for String) + 32 (u8 slot)
///
///     fn encode_len(&self) -> usize {
///         64 + ::pvm_contract_types::SolEncode::tail_len(&self.name)
///     }
///
///     fn encode_to(&self, buf: &mut [u8]) {
///         let __head_size: usize = 64;
///         let mut __tail_offset: usize = __head_size;
///
///         // Field 0 (name: String) — dynamic, write offset pointer
///         buf[0..24].fill(0);
///         buf[24..32].copy_from_slice(&(__tail_offset as u64).to_be_bytes());
///         let __tail_len = ::pvm_contract_types::SolEncode::tail_len(&self.name);
///         ::pvm_contract_types::SolEncode::encode_tail_to(
///             &self.name,
///             &mut buf[__tail_offset..__tail_offset + __tail_len]
///         );
///         __tail_offset += __tail_len;
///
///         // Field 1 (age: u8) — static, write inline
///         <u8 as ::pvm_contract_types::SolEncode>::encode_to(
///             &self.age, &mut buf[32..64]);
///     }
/// }
///
/// impl ::pvm_contract_types::SolDecode for User {
///     fn decode_at(input: &[u8], offset: usize) -> Self { /* ... */ }
///     fn decode_tail(input: &[u8], offset: usize) -> Self {
///         Self::decode_at(input, offset)
///     }
/// }
///
/// impl ::pvm_contract_types::SolArrayElement for User {}
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

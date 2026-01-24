extern crate proc_macro;

mod codegen;
mod signature;
mod solidity;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput, ItemFn, ItemMod};

/// Marks a module as a PVM smart contract, generating dispatch logic and entry points.
///
/// # Attributes
///
/// - `"path/to/Interface.sol"` - Optional Solidity interface file defining method signatures
/// - `no_alloc` - Disables the allocator (uses fixed-size stack buffers)
/// - `buffer = N` - Sets the calldata buffer size for no_alloc mode (default: 256)
///
/// # Usage with Solidity Interface (Recommended)
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
///     pub fn total_supply() -> U256 { get_total_supply() }
///
///     #[pvm_contract::method]
///     pub fn balance_of(account: Address) -> U256 { get_balance(&account) }
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
/// An empty `Error` enum is generated inside the contract module. Add your own variants:
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
/// ## Dispatch Logic (alloc mode)
///
/// With allocation enabled (default), the `call()` function uses `Vec`:
///
/// ```ignore
/// #[polkavm_derive::polkavm_export]
/// pub extern "C" fn call() {
///     let call_data_len = pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
///     let mut call_data = vec![0u8; call_data_len];
///     pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data, 0);
///
///     let result: Result<Option<Vec<u8>>, Vec<u8>> = (|| {
///         if call_data.len() < 4 {
///             return my_token::fallback().map(|()| None).map_err(|e| e.as_ref().to_vec());
///         }
///         let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
///         let input = &call_data[4..];
///
///         match selector {
///             [0x18, 0x16, 0x0d, 0xdd] => {
///                 // totalSupply() -> uint256
///                 Ok(Some({
///                     let result = my_token::total_supply();
///                     result.to_be_bytes::<32>().to_vec()
///                 }))
///             }
///             [0x70, 0xa0, 0x82, 0x31] => {
///                 // balanceOf(address) -> uint256
///                 let mut account = [0u8; 20];
///                 account.copy_from_slice(&input[12..32]);
///                 Ok(Some({
///                     let result = my_token::balance_of(account);
///                     result.to_be_bytes::<32>().to_vec()
///                 }))
///             }
///             _ => my_token::fallback().map(|()| None).map_err(|e| e.as_ref().to_vec()),
///         }
///     })();
///
///     match result {
///         Ok(Some(data)) => {
///             pallet_revive_uapi::HostFnImpl::return_value(
///                 pallet_revive_uapi::ReturnFlags::empty(), &data);
///         }
///         Ok(None) => {}
///         Err(data) => {
///             pallet_revive_uapi::HostFnImpl::return_value(
///                 pallet_revive_uapi::ReturnFlags::REVERT, &data);
///         }
///     }
/// }
/// ```
///
/// ## Dispatch Logic (no_alloc mode)
///
/// With `no_alloc`, returns happen directly in selector arms (no Result wrapper):
///
/// ```ignore
/// #[pvm_contract_macros::contract("MyToken.sol", no_alloc, buffer = 512)]
/// mod my_token { /* ... */ }
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
///     if call_data_len < 4 {
///         // fallback handling
///     }
///
///     let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
///     let input = &call_data[4..call_data_len];
///
///     match selector {
///         [0x18, 0x16, 0x0d, 0xdd] => {
///             // totalSupply() -> returns directly
///             let result = my_token::total_supply();
///             let encoded = result.to_be_bytes::<32>();
///             pallet_revive_uapi::HostFnImpl::return_value(
///                 pallet_revive_uapi::ReturnFlags::empty(), &encoded);
///         }
///         _ => {
///             // fallback
///         }
///     }
/// }
/// ```
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
/// # Example
///
/// ```ignore
/// #[pvm_contract::constructor]
/// pub fn new() -> Result<(), Error> {
///     set_owner(pvm_contract::caller());
///     Ok(())
/// }
/// ```
///
/// Must return `Result<(), Error>`. Returning `Err` reverts the deployment.
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
/// # Generated Code
///
/// For this struct:
///
/// ```
/// # use ruint::aliases::U256;
/// #[derive(pvm_contract_macros::SolType)]
/// pub struct Point {
///     pub x: U256,
///     pub y: U256,
/// }
/// ```
///
/// The macro generates:
///
/// ```
/// # use ruint::aliases::U256;
/// # pub struct Point { pub x: U256, pub y: U256 }
/// impl Point {
///     /// Solidity tuple signature for ABI encoding
///     pub const SOL_NAME: &'static str = "(uint256,uint256)";
///
///     /// Total size in bytes when ABI-encoded (each uint256 = 32 bytes)
///     pub const ENCODED_SIZE: usize = 64;
///
///     /// Decode from ABI-encoded bytes at the given offset
///     pub fn abi_decode(input: &[u8], offset: usize) -> Self {
///         Self {
///             x: U256::from_be_slice(&input[offset..offset + 32]),
///             y: U256::from_be_slice(&input[offset + 32..offset + 64]),
///         }
///     }
///
///     /// Encode to fixed-size ABI bytes
///     pub fn abi_encode(&self) -> [u8; 64] {
///         let mut out = [0u8; 64];
///         out[0..32].copy_from_slice(&self.x.to_be_bytes::<32>());
///         out[32..64].copy_from_slice(&self.y.to_be_bytes::<32>());
///         out
///     }
/// }
/// ```
///
/// # Usage in Contract Methods
///
/// ```ignore
/// #[pvm_contract_macros::method]
/// pub fn set_point(point: Point) {
///     // Macro calls Point::abi_decode() automatically
/// }
///
/// #[pvm_contract_macros::method]
/// pub fn get_point() -> Point {
///     // Macro calls point.abi_encode() automatically
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
/// | `[u8; 20]` | `address` | 32 bytes |
/// | `[u8; N]` (N <= 32) | `bytesN` | 32 bytes |
/// | `[T; N]` | `T[N]` | N * element size |
/// | Other `SolType` struct | tuple | sum of field sizes |
///
/// # Limitations
///
/// Dynamic types are **not supported** and will cause a compile error:
/// - `Vec<T>` - use fixed arrays `[T; N]` instead
/// - `String` - not supported
/// - `&[u8]` / `&str` - not supported
#[proc_macro_derive(SolType)]
pub fn sol_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match codegen::expand_sol_type(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

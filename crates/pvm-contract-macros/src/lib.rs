extern crate proc_macro;

mod codegen;
mod signature;
mod solidity;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput, ItemFn, ItemMod, ItemStruct};

/// Marks a module as a PVM smart contract, generating dispatch logic and entry points.
///
/// # Attributes
///
/// - `"path/to/Interface.sol"` - Optional Solidity interface file defining method signatures
/// - `no_alloc` - Disables the allocator (uses fixed-size stack buffers)
/// - `buffer = N` - Sets the calldata buffer size for no_alloc mode (default: 256)
/// - `cdm = "@namespace/name"` - Enable CDM (Contract Deployment Manager) support
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
/// pub extern "C" fn call() {
///     let call_data_len = pvm_contract::api::call_data_size() as usize;
///     let mut call_data = vec![0u8; call_data_len];
///     pvm_contract::api::call_data_copy(&mut call_data, 0);
///
///     let result: Result<Option<Vec<u8>>, Vec<u8>> = (|| {
///         if call_data.len() < 4 { return Err(Vec::new()); }
///         let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
///
///         match selector {
///             [0x18, 0x16, 0x0d, 0xdd] => {  // totalSupply()
///                 Ok(Some(my_token::total_supply().to_be_bytes::<32>().to_vec()))
///             }
///             _ => Err(Vec::new()),
///         }
///     })();
///
///     match result {
///         Ok(Some(data)) => pvm_contract::api::return_value(pvm_contract::ReturnFlags::empty(), &data),
///         Ok(None) => {}
///         Err(data) => pvm_contract::api::return_value(pvm_contract::ReturnFlags::REVERT, &data),
///     }
/// }
/// ```
///
/// ## Dispatch Logic (no_alloc mode)
///
/// With `no_alloc`, fixed-size stack buffers are used instead:
///
/// ```ignore
/// #[pvm_contract::contract("MyToken.sol", no_alloc, buffer = 512)]
/// mod my_token { /* ... */ }
///
/// // Expands to:
/// pub extern "C" fn call() {
///     let call_data_len = pvm_contract::api::call_data_size() as usize;
///     let mut call_data = [0u8; 512];  // fixed buffer
///
///     if call_data_len > 512 {
///         pvm_contract::api::return_value(pvm_contract::ReturnFlags::REVERT, b"CalldataTooLarge");
///         return;
///     }
///     pvm_contract::api::call_data_copy(&mut call_data[..call_data_len], 0);
///
///     let result: Result<Option<&[u8]>, &[u8]> = (|| {
///         // ... dispatch using slices instead of Vec
///     })();
///     // ...
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

/// Generates storage accessor functions from a struct definition.
///
/// Transforms a struct with typed fields into a unit struct with associated
/// functions that return `Lazy<T>` or `Mapping<K, V>` wrappers.
///
/// # Example
///
/// ```ignore
/// use pvm_contract::storage::{Lazy, Mapping};
///
/// #[pvm_contract::storage]
/// struct Storage {
///     counter: u64,
///     total_supply: u128,
///     balances: Mapping<[u8; 20], u128>,
/// }
///
/// // Expands to:
/// struct Storage;
/// impl Storage {
///     fn counter() -> Lazy<u64> { Lazy::new(b"Storage::counter") }
///     fn total_supply() -> Lazy<u128> { Lazy::new(b"Storage::total_supply") }
///     fn balances() -> Mapping<[u8; 20], u128> { Mapping::new(b"Storage::balances") }
/// }
/// ```
///
/// # Usage
///
/// ```ignore
/// // Set a value
/// Storage::counter().set(&42u64);
///
/// // Get a value
/// let count = Storage::counter().get();  // Option<u64>
///
/// // Use mapping
/// Storage::balances().insert(&addr, &1000u128);
/// let balance = Storage::balances().get(&addr);  // Option<u128>
/// ```
///
/// # Field Types
///
/// - Plain types (e.g., `u64`, `u128`, `[u8; 32]`) are wrapped in `Lazy<T>`
/// - `Mapping<K, V>` fields are kept as-is
#[proc_macro_attribute]
pub fn storage(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);

    match codegen::expand_storage(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derive macro for implementing `SolAbi` on structs.
///
/// # Newtypes
///
/// For single-field tuple structs, delegates to the inner type:
///
/// ```ignore
/// #[derive(SolAbi)]
/// struct EntityId(pub [u8; 32]);
/// // EntityId now encodes as bytes32
/// ```
///
/// # Named Structs
///
/// For named structs, encodes as a Solidity tuple:
///
/// ```ignore
/// #[derive(SolAbi)]
/// struct Point {
///     x: u64,
///     y: u64,
/// }
/// // Point encodes as (uint64,uint64)
/// ```
///
/// All field types must be resolvable to Solidity types at macro expansion time.
#[proc_macro_derive(SolAbi)]
pub fn derive_sol_abi(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match codegen::expand_derive_sol_abi(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

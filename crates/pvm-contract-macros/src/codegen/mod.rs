pub mod abi_import;
mod allocator;
mod contract;
mod decode;
mod derive_sol_abi;
mod dispatch;
mod encode;
mod method;
mod storage;

pub use abi_import::{expand_abi_import, AbiImportArgs};
pub use contract::{expand_contract, ContractArgs};
pub use derive_sol_abi::expand_derive_sol_abi;
pub use method::{expand_constructor, expand_fallback, expand_method, MethodArgs};
pub use storage::expand_storage;

use proc_macro2::TokenStream;
use quote::quote;

use crate::signature::{compute_selector, SolType};
use decode::generate_decode;

/// Generate the `cdm_reference()` function that resolves the contract address at runtime
/// by calling the ContractRegistry's `getAddress(string)` method.
pub(super) fn generate_cdm_reference(cdm_name: &str) -> TokenStream {
    let selector = compute_selector("getAddress(string)");
    let [s0, s1, s2, s3] = selector;

    quote! {
        /// The address of the contracts registry, baked in at compile time from
        /// the `CONTRACTS_REGISTRY_ADDR` environment variable.
        const __CDM_REGISTRY_ADDR: [u8; 20] = {
            const fn hex(c: u8) -> u8 {
                match c {
                    b'0'..=b'9' => c - b'0',
                    b'a'..=b'f' => c - b'a' + 10,
                    b'A'..=b'F' => c - b'A' + 10,
                    _ => panic!("Invalid hex character in CONTRACTS_REGISTRY_ADDR"),
                }
            }

            match option_env!("CONTRACTS_REGISTRY_ADDR") {
                Some(s) => {
                    let b = s.as_bytes();
                    let off = if b.len() > 1 && b[0] == b'0' && (b[1] == b'x' || b[1] == b'X') {
                        2
                    } else {
                        0
                    };
                    assert!(b.len() - off == 40, "CONTRACTS_REGISTRY_ADDR must be 40 hex chars (with optional 0x prefix)");
                    let mut r = [0u8; 20];
                    let mut i = 0;
                    while i < 20 {
                        r[i] = hex(b[off + i * 2]) << 4 | hex(b[off + i * 2 + 1]);
                        i += 1;
                    }
                    r
                }
                None => [0u8; 20],
            }
        };

        /// Get a runtime-resolved reference to this contract via CDM.
        ///
        /// Looks up the contract address from the ContractRegistry at runtime
        /// using the CDM name registered at compile time. The registry address
        /// is baked in from the `CONTRACTS_REGISTRY_ADDR` environment variable.
        ///
        /// # Panics
        ///
        /// Panics if the cross-contract call to the registry fails or the
        /// contract is not registered.
        pub fn cdm_reference() -> Reference {
            extern crate alloc;

            let cdm_name: &str = #cdm_name;
            let name_len = cdm_name.len();
            let padded_len = (name_len + 31) / 32 * 32;

            // Build calldata: selector + ABI-encoded string
            // String ABI encoding: offset (32 bytes) + length (32 bytes) + data (padded to 32)
            let mut calldata = alloc::vec![0u8; 4 + 32 + 32 + padded_len];

            // Selector for getAddress(string)
            calldata[0] = #s0;
            calldata[1] = #s1;
            calldata[2] = #s2;
            calldata[3] = #s3;

            // Offset to string data (always 32 = 0x20)
            calldata[4 + 24..4 + 32].copy_from_slice(&(32u64).to_be_bytes());

            // String length
            calldata[4 + 32 + 24..4 + 32 + 32].copy_from_slice(&(name_len as u64).to_be_bytes());

            // String data
            calldata[4 + 64..4 + 64 + name_len].copy_from_slice(cdm_name.as_bytes());

            let result = pvm_contract::call::call_evm_collect(
                pvm_contract::CallFlags::ALLOW_REENTRY,
                &__CDM_REGISTRY_ADDR, u64::MAX, &[0u8; 32], &calldata,
            );

            match result {
                Ok(output) => {
                    // First word (0..32) is isSome bool, second word (32..64) is the address
                    // Address is 20 bytes right-aligned in the second 32-byte word
                    let is_some = output[31] != 0;
                    if !is_some {
                        panic!("CDM: contract not found in registry");
                    }
                    let mut addr = [0u8; 20];
                    addr.copy_from_slice(&output[44..64]);
                    Reference::at(pvm_contract::Address::from(addr))
                }
                Err(_) => {
                    panic!("CDM: registry call failed");
                }
            }
        }
    }
}

/// For fully-resolved SolTypes, produce the return type, output buffer setup,
/// decode expression, and whether there is output — for cross-contract call methods.
pub(super) fn generate_resolved_return_parts(
    outputs: &[SolType],
) -> (TokenStream, TokenStream, bool) {
    match outputs {
        [] => (quote! { () }, quote! { () }, false),
        [one] => {
            let rt = one.rust_type(true);
            let decode = generate_decode(one, quote!(output), 0, true);
            (quote! { #rt }, decode, true)
        }
        many => {
            let tys: Vec<TokenStream> = many.iter().map(|t| t.rust_type(true)).collect();
            let mut offset = 0usize;
            let decs: Vec<TokenStream> = many
                .iter()
                .map(|t| {
                    let d = generate_decode(t, quote!(output), offset, true);
                    offset += t.head_size();
                    d
                })
                .collect();
            (quote! { (#(#tys),*) }, quote! { (#(#decs),*) }, true)
        }
    }
}

/// If `ty` is `Option<Inner>`, return `Some(Inner)`. Otherwise `None`.
fn unwrap_option_inner(ty: &syn::Type) -> Option<&syn::Type> {
    if let syn::Type::Path(type_path) = ty {
        let segment = type_path.path.segments.last()?;
        if segment.ident == "Option" {
            if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                if args.args.len() == 1 {
                    if let syn::GenericArgument::Type(inner) = &args.args[0] {
                        return Some(inner);
                    }
                }
            }
        }
    }
    None
}

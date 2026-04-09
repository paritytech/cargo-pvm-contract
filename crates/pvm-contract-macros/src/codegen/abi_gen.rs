use proc_macro2::TokenStream;
use quote::quote;

use super::contract::ParsedContract;
use super::dispatch::MethodInfo;

/// Generate both the in-module ABI helper and the top-level `main()`.
///
/// The helper lives inside the user's module so all type imports are in scope.
/// The `main()` just calls the helper and prints the result.
pub fn generate_abi_gen(parsed: &ParsedContract, has_sol_path: bool) -> (TokenStream, TokenStream) {
    // When a .sol file is provided, the builder derives ABI from the Solidity
    // interface at build time (see cargo-pvm-contract-builder/src/abi.rs).
    // No macro-side ABI generation is needed.
    if has_sol_path {
        return (quote! {}, quote! {});
    }

    match generate_abi_gen_impl(parsed) {
        Ok((helper, main_fn)) => (helper, main_fn),
        Err(err) => {
            let err = err.to_compile_error();
            (quote! {}, err)
        }
    }
}

fn generate_abi_gen_impl(parsed: &ParsedContract) -> syn::Result<(TokenStream, TokenStream)> {
    let constructor_entry = if parsed.has_constructor {
        let ctor_params: Vec<TokenStream> = parsed
            .constructor_inputs
            .iter()
            .map(|(name, ty)| {
                let name_str = name.to_string();
                quote! {
                    <#ty as ::pvm_contract_types::SolEncode>::abi_param(#name_str)
                }
            })
            .collect();

        quote! {
            __items.push(::pvm_contract_types::AbiItem::Constructor {
                inputs: vec![#(#ctor_params),*],
                state_mutability: "nonpayable".into(),
            });
        }
    } else {
        quote! {}
    };

    let method_entries: Vec<TokenStream> = parsed
        .methods
        .iter()
        .map(generate_method_entry)
        .collect::<syn::Result<Vec<_>>>()?;

    let helper = quote! {
        #[cfg(feature = "abi-gen")]
        #[doc(hidden)]
        pub fn __abi_json() -> ::std::string::String {
            let mut __items: ::std::vec::Vec<::pvm_contract_types::AbiItem> = ::std::vec::Vec::new();

            #constructor_entry

            #(#method_entries)*

            ::pvm_contract_types::abi_to_json(&__items)
        }
    };

    let mod_name = &parsed.mod_name;
    let main_fn = quote! {
        #[cfg(feature = "abi-gen")]
        fn main() {
            ::std::println!("{}", #mod_name::__abi_json());
        }
    };

    Ok((helper, main_fn))
}

fn generate_method_entry(method: &MethodInfo) -> syn::Result<TokenStream> {
    let method_name = &method.sol_name;

    let input_params: Vec<TokenStream> = method
        .param_types
        .iter()
        .zip(method.param_names.iter())
        .map(|(ty, name)| {
            let name_str = name.to_string();
            quote! {
                <#ty as ::pvm_contract_types::SolEncode>::abi_param(#name_str)
            }
        })
        .collect();

    let output_params: Vec<TokenStream> = method
        .return_types
        .iter()
        .map(|ty| {
            quote! {
                <#ty as ::pvm_contract_types::SolEncode>::abi_param("")
            }
        })
        .collect();

    let has_return = !method.return_types.is_empty();
    let mutability = if has_return { "view" } else { "nonpayable" };

    Ok(quote! {
        __items.push(::pvm_contract_types::AbiItem::Function {
            name: #method_name.into(),
            inputs: vec![#(#input_params),*],
            outputs: vec![#(#output_params),*],
            state_mutability: #mutability.into(),
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_empty_for_sol_path_contract() {
        let parsed = ParsedContract {
            mod_name: syn::parse_str("contract").unwrap(),
            methods: vec![],
            has_constructor: false,
            has_fallback: false,
            constructor_name: None,
            constructor_returns_result: false,
            constructor_inputs: vec![],
            fallback_name: None,
        };

        let (helper, main_fn) = generate_abi_gen(&parsed, true);
        assert!(helper.is_empty());
        assert!(main_fn.is_empty());
    }
}

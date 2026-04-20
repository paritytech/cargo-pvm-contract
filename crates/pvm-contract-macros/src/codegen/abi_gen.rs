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
                state_mutability: Some("payable".into()),
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

    // Emit error ABI entries by calling error_signatures() on each error type.
    // Deduplication uses exact-match on the full signature ("Name(type1,type2)")
    // so that overloaded errors with different params are all emitted.
    let error_entries: Vec<TokenStream> = parsed
        .error_types
        .iter()
        .map(|err_ty| {
            quote! {
                for __sig in <#err_ty as ::pvm_contract_types::SolRevert>::error_signatures() {
                    let Some(__paren) = __sig.find('(') else { continue; };
                    if !__sig.ends_with(')') { continue; }
                    if __seen_errors.contains(__sig) {
                        continue;
                    }
                    __seen_errors.push(__sig);
                    let __err_name = &__sig[..__paren];
                    let __params_str = &__sig[__paren + 1..__sig.len() - 1];
                    let __inputs: ::std::vec::Vec<::pvm_contract_types::AbiParam> = if __params_str.is_empty() {
                        ::std::vec::Vec::new()
                    } else {
                        __split_params(__params_str)
                            .into_iter()
                            .map(|t| ::pvm_contract_types::parse_type_str("", t))
                            .collect()
                    };
                    __items.push(::pvm_contract_types::AbiItem::Error {
                        name: __err_name.into(),
                        inputs: __inputs,
                    });
                }
            }
        })
        .collect();

    let split_params_helper = if !parsed.error_types.is_empty() {
        quote! {
            fn __split_params(s: &str) -> ::std::vec::Vec<&str> {
                let mut params = ::std::vec::Vec::new();
                let mut depth = 0usize;
                let mut start = 0;
                for (i, ch) in s.char_indices() {
                    match ch {
                        '(' => depth += 1,
                        ')' => depth = depth.saturating_sub(1),
                        ',' if depth == 0 => {
                            params.push(s[start..i].trim());
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                let last = s[start..].trim();
                if !last.is_empty() {
                    params.push(last);
                }
                params
            }
        }
    } else {
        quote! {}
    };

    // Framework errors are parameterless (`Name()`). Only suppress when a
    // user-defined error has the exact same signature. A user-defined
    // `error InvalidCalldata(uint256)` has a different selector and must
    // coexist in the ABI so tools can decode both reverts.
    let framework_error_entries: Vec<TokenStream> = pvm_contract_types::framework_errors::NAMES
        .iter()
        .map(|name| {
            let sig = format!("{name}()");
            let name_str = name.to_string();
            quote! {
                if !__seen_errors.iter().any(|s| *s == #sig) {
                    __items.push(::pvm_contract_types::AbiItem::Error {
                        name: #name_str.into(),
                        inputs: ::std::vec::Vec::new(),
                    });
                }
            }
        })
        .collect();

    let helper = quote! {
        #[cfg(feature = "abi-gen")]
        #[doc(hidden)]
        pub fn __abi_json() -> ::std::string::String {
            #split_params_helper

            let mut __items: ::std::vec::Vec<::pvm_contract_types::AbiItem> = ::std::vec::Vec::new();
            let mut __seen_errors = ::std::vec::Vec::<&str>::new();

            #constructor_entry

            #(#method_entries)*

            #(#error_entries)*

            #(#framework_error_entries)*

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

    // All methods are emitted with `"stateMutability":"payable"` because we don't yet
    // support `payable`/`nonpayable`/`view`/`pure` attributes on Rust methods.
    // Once state mutability attributes are added, this should be derived from the
    // method annotation instead of hardcoded.
    Ok(quote! {
        __items.push(::pvm_contract_types::AbiItem::Function {
            name: #method_name.into(),
            inputs: vec![#(#input_params),*],
            outputs: vec![#(#output_params),*],
            state_mutability: Some("payable".into()),
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
            fallback_returns_result: false,
            error_types: vec![],
        };

        let (helper, main_fn) = generate_abi_gen(&parsed, true);
        assert!(helper.is_empty());
        assert!(main_fn.is_empty());
    }
}

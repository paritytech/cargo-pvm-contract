use proc_macro2::TokenStream;
use quote::quote;

use super::contract::ParsedContract;
use super::dispatch::{MethodInfo, StateMutability};

/// Generate both the in-module ABI helper and the top-level `main()`.
///
/// The helper lives inside the user's module so all type imports are in scope.
/// The `main()` just calls the helper and prints the result.
///
/// When a `.sol` file is provided, the builder derives the ABI from the Solidity
/// interface at build time. However, `storageLayout` is always Rust-side, so
/// `main()` is still generated when a `SolStorage` struct exists.
pub fn generate_abi_gen(
    parsed: &ParsedContract,
    has_sol_path: bool,
    storage_struct: Option<syn::Ident>,
) -> (TokenStream, TokenStream) {
    if has_sol_path && storage_struct.is_none() {
        // .sol path, no storage: builder gets everything from the .sol file.
        return (quote! {}, quote! {});
    }

    if has_sol_path {
        // .sol path with storage: builder gets ABI from .sol, but needs
        // main() to output storage layout from the Rust side.
        let mod_name = &parsed.mod_name;
        let helper = storage_layout_helper(storage_struct.as_ref().unwrap());
        let main_fn = quote! {
            #[cfg(feature = "abi-gen")]
            fn main() {
                ::std::println!("{}", #mod_name::__storage_layout_json());
            }
        };
        return (helper, main_fn);
    }

    // Non-.sol path: generate both ABI and optional storage layout.
    match generate_abi_gen_impl(parsed, storage_struct) {
        Ok((helper, main_fn)) => (helper, main_fn),
        Err(err) => {
            let err = err.to_compile_error();
            (quote! {}, err)
        }
    }
}

/// Generate a module-level `__storage_layout_json()` wrapper that delegates
/// to the (private) storage struct's method. Lives inside the module so it
/// can access the struct; declared `pub` so `main()` can call it from outside.
fn storage_layout_helper(struct_name: &syn::Ident) -> TokenStream {
    quote! {
        #[cfg(feature = "abi-gen")]
        #[doc(hidden)]
        pub fn __storage_layout_json() -> ::std::string::String {
            #struct_name::__storage_layout_json()
        }
    }
}

fn generate_abi_gen_impl(
    parsed: &ParsedContract,
    storage_struct: Option<syn::Ident>,
) -> syn::Result<(TokenStream, TokenStream)> {
    let constructor_entry = if parsed.has_constructor {
        let ctor_params: Vec<TokenStream> = parsed
            .constructor_inputs
            .iter()
            .map(|(name, ty)| {
                let name_str = name.to_string();
                quote! {
                    <#ty as ::pvm_contract_sdk::SolEncode>::abi_param(#name_str)
                }
            })
            .collect();

        let mutability = if parsed.constructor_is_payable {
            StateMutability::Payable
        } else {
            StateMutability::NonPayable
        }
        .as_abi_str();
        quote! {
            __items.push(::pvm_contract_sdk::AbiItem::Constructor {
                inputs: vec![#(#ctor_params),*],
                state_mutability: Some(#mutability.into()),
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
                for __sig in <#err_ty as ::pvm_contract_sdk::SolRevert>::error_signatures() {
                    let Some(__paren) = __sig.find('(') else { continue; };
                    if !__sig.ends_with(')') { continue; }
                    if __seen_errors.contains(__sig) {
                        continue;
                    }
                    __seen_errors.push(__sig);
                    let __err_name = &__sig[..__paren];
                    let __params_str = &__sig[__paren + 1..__sig.len() - 1];
                    let __inputs: ::std::vec::Vec<::pvm_contract_sdk::AbiParam> = if __params_str.is_empty() {
                        ::std::vec::Vec::new()
                    } else {
                        __split_params(__params_str)
                            .into_iter()
                            .map(|t| ::pvm_contract_sdk::parse_type_str("", t))
                            .collect()
                    };
                    __items.push(::pvm_contract_sdk::AbiItem::Error {
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
                    __items.push(::pvm_contract_sdk::AbiItem::Error {
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

            let mut __items: ::std::vec::Vec<::pvm_contract_sdk::AbiItem> = ::std::vec::Vec::new();
            let mut __seen_errors = ::std::vec::Vec::<&str>::new();

            #constructor_entry

            #(#method_entries)*

            #(#error_entries)*

            #(#framework_error_entries)*

            ::pvm_contract_sdk::abi_to_json(&__items)
        }
    };

    let mod_name = &parsed.mod_name;

    let combined_helper = match &storage_struct {
        Some(struct_name) => {
            let sh = storage_layout_helper(struct_name);
            quote! { #helper #sh }
        }
        None => helper,
    };

    let main_fn = match storage_struct {
        Some(_) => quote! {
            #[cfg(feature = "abi-gen")]
            fn main() {
                let abi = #mod_name::__abi_json();
                let layout = #mod_name::__storage_layout_json();
                // Wrap ABI array and storage layout into a JSON object.
                ::std::print!("{{\"abi\":");
                ::std::print!("{}", abi);
                ::std::print!(",\"storageLayout\":");
                ::std::print!("{}", layout);
                ::std::println!("}}");
            }
        },
        None => quote! {
            #[cfg(feature = "abi-gen")]
            fn main() {
                ::std::println!("{}", #mod_name::__abi_json());
            }
        },
    };

    Ok((combined_helper, main_fn))
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
                <#ty as ::pvm_contract_sdk::SolEncode>::abi_param(#name_str)
            }
        })
        .collect();

    let output_params: Vec<TokenStream> = method
        .return_types
        .iter()
        .map(|ty| {
            quote! {
                <#ty as ::pvm_contract_sdk::SolEncode>::abi_param("")
            }
        })
        .collect();

    let mutability = method.mutability.as_abi_str();

    Ok(quote! {
        __items.push(::pvm_contract_sdk::AbiItem::Function {
            name: #method_name.into(),
            inputs: vec![#(#input_params),*],
            outputs: vec![#(#output_params),*],
            state_mutability: Some(#mutability.into()),
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::contract::{ContractArgs, expand_contract};

    #[test]
    fn returns_empty_for_sol_path_contract() {
        let parsed = ParsedContract {
            mod_name: syn::parse_str("contract").unwrap(),
            struct_name: None,
            methods: vec![],
            has_constructor: false,
            has_fallback: false,
            constructor_name: None,
            constructor_returns_result: false,
            constructor_inputs: vec![],
            constructor_is_payable: false,
            fallback_name: None,
            fallback_returns_result: false,
            fallback_is_payable: false,
            error_types: vec![],
        };

        let (helper, main_fn) = generate_abi_gen(&parsed, true, None);
        assert!(helper.is_empty());
        assert!(main_fn.is_empty());
    }

    fn expand_to_string(input: syn::ItemMod) -> String {
        expand_contract(ContractArgs::default(), input)
            .unwrap()
            .to_string()
    }

    fn mutability_token(m: &str) -> String {
        format!(r#"Some ("{m}""#)
    }

    #[test]
    fn payable_method_abi_has_payable_mutability() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    #[pvm_contract_macros::payable]
                    pub fn deposit(&mut self) {}
                }
            }
        };
        let s = expand_to_string(input);
        assert!(
            s.contains(&mutability_token("payable")),
            "payable method ABI must declare stateMutability = payable; got:\n{s}"
        );
    }

    #[test]
    fn non_payable_method_abi_has_nonpayable_mutability() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn transfer(&mut self, to: Address) -> bool { false }
                }
            }
        };
        let s = expand_to_string(input);
        assert!(
            s.contains(&mutability_token("nonpayable")),
            "non-payable method ABI must declare stateMutability = nonpayable; got:\n{s}"
        );
        assert!(
            !s.contains(&mutability_token("payable")),
            "non-payable-only contract must not declare any payable mutability; got:\n{s}"
        );
    }

    #[test]
    fn payable_constructor_abi_has_payable_mutability() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    #[pvm_contract_macros::payable]
                    pub fn new(&mut self) {}
                }
            }
        };
        let s = expand_to_string(input);
        let ctor_marker = "AbiItem :: Constructor";
        assert!(
            s.contains(ctor_marker),
            "constructor entry marker missing; got:\n{s}"
        );
        let after_ctor = &s[s.find(ctor_marker).unwrap()..];
        assert!(
            after_ctor.contains(&mutability_token("payable")),
            "payable constructor must declare payable; got:\n{after_ctor}"
        );
    }

    #[test]
    fn state_mutability_abi_str() {
        assert_eq!(StateMutability::NonPayable.as_abi_str(), "nonpayable");
        assert_eq!(StateMutability::View.as_abi_str(), "view");
        assert_eq!(StateMutability::Pure.as_abi_str(), "pure");
        assert_eq!(StateMutability::Payable.as_abi_str(), "payable");
    }

    fn parsed_contract_with_method(method: MethodInfo) -> ParsedContract {
        ParsedContract {
            mod_name: syn::parse_str("contract").unwrap(),
            struct_name: None,
            methods: vec![method],
            has_constructor: false,
            has_fallback: false,
            constructor_name: None,
            constructor_returns_result: false,
            constructor_inputs: vec![],
            constructor_is_payable: false,
            fallback_name: None,
            fallback_returns_result: false,
            fallback_is_payable: false,
            error_types: vec![],
        }
    }

    #[test]
    fn view_method_abi_has_view_mutability() {
        let method = MethodInfo {
            fn_name: quote::format_ident!("balance"),
            sol_name: "balance".to_string(),
            param_names: vec![],
            param_types: vec![],
            return_types: vec![syn::parse_quote!(U256)],
            returns_result: false,
            mutability: StateMutability::View,
            precomputed_selector: None,
        };
        let parsed = parsed_contract_with_method(method);
        let (helper, _main_fn) = generate_abi_gen(&parsed, false, None);
        let s = helper.to_string();
        assert!(
            s.contains(&mutability_token("view")),
            "view method ABI must declare stateMutability = view; got:\n{s}"
        );
    }

    #[test]
    fn pure_method_abi_has_pure_mutability() {
        let method = MethodInfo {
            fn_name: quote::format_ident!("add"),
            sol_name: "add".to_string(),
            param_names: vec![quote::format_ident!("a"), quote::format_ident!("b")],
            param_types: vec![syn::parse_quote!(U256), syn::parse_quote!(U256)],
            return_types: vec![syn::parse_quote!(U256)],
            returns_result: false,
            mutability: StateMutability::Pure,
            precomputed_selector: None,
        };
        let parsed = parsed_contract_with_method(method);
        let (helper, _main_fn) = generate_abi_gen(&parsed, false, None);
        let s = helper.to_string();
        assert!(
            s.contains(&mutability_token("pure")),
            "pure method ABI must declare stateMutability = pure; got:\n{s}"
        );
    }

    #[test]
    fn non_payable_constructor_abi_has_nonpayable_mutability() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self, initial: U256) {}
                }
            }
        };
        let s = expand_to_string(input);
        let ctor_marker = "AbiItem :: Constructor";
        assert!(
            s.contains(ctor_marker),
            "constructor entry marker missing; got:\n{s}"
        );
        let after_ctor = &s[s.find(ctor_marker).unwrap()..];
        assert!(
            after_ctor.contains(&mutability_token("nonpayable")),
            "non-payable constructor must declare nonpayable; got:\n{after_ctor}"
        );
        assert!(
            !after_ctor.contains(&mutability_token("payable")),
            "non-payable-only contract must not emit stateMutability = payable; got:\n{after_ctor}"
        );
    }

    #[test]
    fn sol_path_with_storage_generates_main_for_layout() {
        let parsed = ParsedContract {
            mod_name: syn::parse_str("contract").unwrap(),
            struct_name: None,
            methods: vec![],
            has_constructor: false,
            has_fallback: false,
            constructor_name: None,
            constructor_returns_result: false,
            constructor_inputs: vec![],
            constructor_is_payable: false,
            fallback_name: None,
            fallback_returns_result: false,
            fallback_is_payable: false,
            error_types: vec![],
        };

        let storage_name: syn::Ident = syn::parse_str("Storage").unwrap();
        let (helper, main_fn) = generate_abi_gen(&parsed, true, Some(storage_name));
        // Helper contains the __storage_layout_json wrapper; main() calls it.
        assert!(
            !helper.is_empty(),
            "helper should contain __storage_layout_json wrapper for .sol + storage"
        );
        assert!(
            !main_fn.is_empty(),
            "main() should be generated for storage layout even with .sol path"
        );
        let helper_str = helper.to_string();
        assert!(
            helper_str.contains("__storage_layout_json"),
            "helper should contain __storage_layout_json. Got: {helper_str}"
        );
        let main_str = main_fn.to_string();
        assert!(
            main_str.contains("__storage_layout_json"),
            "main() should call __storage_layout_json(). Got: {main_str}"
        );
    }
}

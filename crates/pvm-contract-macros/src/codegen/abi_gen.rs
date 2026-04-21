use proc_macro2::TokenStream;
use quote::quote;

use super::contract::ParsedContract;
use super::dispatch::{MethodInfo, StateMutability};

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
        let constructor_input_entries: Vec<TokenStream> = parsed
            .constructor_inputs
            .iter()
            .map(|(name, ty)| {
                let name_str = name.to_string();
                Ok(quote! {
                    if !__first_ctor_input {
                        __abi.push(',');
                    } else {
                        __first_ctor_input = false;
                    }
                    __abi.push_str("{\"name\":\"");
                    __abi.push_str(#name_str);
                    __abi.push_str("\",\"type\":\"");
                    __abi.push_str(<#ty as ::pvm_contract_types::SolEncode>::SOL_NAME);
                    __abi.push_str("\"}");
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;

        let mutability = if parsed.constructor_is_payable {
            StateMutability::Payable
        } else {
            StateMutability::NonPayable
        }
        .as_abi_str();
        let closing = format!("],\"stateMutability\":\"{mutability}\"}}");
        quote! {
            if !__first_item {
                __abi.push(',');
            } else {
                __first_item = false;
            }
            __abi.push_str("{\"type\":\"constructor\",\"inputs\":[");
            let mut __first_ctor_input = true;
            #(#constructor_input_entries)*
            __abi.push_str(#closing);
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
                    if !__first_item {
                        __abi.push(',');
                    } else {
                        __first_item = false;
                    }
                    let __err_name = &__sig[..__paren];
                    let __params_str = &__sig[__paren + 1..__sig.len() - 1];
                    __abi.push_str("{\"type\":\"error\",\"name\":\"");
                    __abi.push_str(__err_name);
                    __abi.push_str("\",\"inputs\":[");
                    if !__params_str.is_empty() {
                        let mut __first_param = true;
                        for __param_type in __split_params(__params_str) {
                            if !__first_param {
                                __abi.push(',');
                            } else {
                                __first_param = false;
                            }
                            __abi.push_str("{\"name\":\"\",\"type\":\"");
                            __abi.push_str(__param_type);
                            __abi.push_str("\"}");
                        }
                    }
                    __abi.push_str("]}");
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
                        ')' => depth -= 1,
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

    let framework_error_entries: Vec<TokenStream> = pvm_contract_types::framework_errors::NAMES
        .iter()
        .map(|name| {
            let prefix = format!("{name}(");
            let entry = format!("{{\"type\":\"error\",\"name\":\"{name}\",\"inputs\":[]}}");
            quote! {
                if !__seen_errors.iter().any(|s| s.starts_with(#prefix)) {
                    if !__first_item {
                        __abi.push(',');
                    } else {
                        __first_item = false;
                    }
                    __abi.push_str(#entry);
                }
            }
        })
        .collect();

    let helper = quote! {
        #[cfg(feature = "abi-gen")]
        #[doc(hidden)]
        pub fn __abi_json() -> ::std::string::String {
            #split_params_helper

            let mut __abi = ::std::string::String::from("[");
            let mut __first_item = true;
            let mut __seen_errors = ::std::vec::Vec::<&str>::new();

            #constructor_entry

            #(#method_entries)*

            #(#error_entries)*

            #(#framework_error_entries)*

            __abi.push(']');
            __abi
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

    let input_entries: Vec<TokenStream> = method
        .param_types
        .iter()
        .zip(method.param_names.iter())
        .map(|(ty, name)| {
            let name_str = name.to_string();
            Ok(quote! {
                if !__first_input {
                    __abi.push(',');
                } else {
                    __first_input = false;
                }
                __abi.push_str("{\"name\":\"");
                __abi.push_str(#name_str);
                __abi.push_str("\",\"type\":\"");
                __abi.push_str(<#ty as ::pvm_contract_types::SolEncode>::SOL_NAME);
                __abi.push_str("\"}");
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    let output_entries: Vec<TokenStream> = method
        .return_types
        .iter()
        .map(|ty| {
            Ok(quote! {
                if !__first_output {
                    __abi.push(',');
                } else {
                    __first_output = false;
                }
                __abi.push_str("{\"name\":\"\",\"type\":\"");
                __abi.push_str(<#ty as ::pvm_contract_types::SolEncode>::SOL_NAME);
                __abi.push_str("\"}");
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    let mutability = method.mutability.as_abi_str();
    let closing = format!("],\"stateMutability\":\"{mutability}\"}}");

    Ok(quote! {
        if !__first_item {
            __abi.push(',');
        } else {
            __first_item = false;
        }

        __abi.push_str("{\"type\":\"function\",\"name\":\"");
        __abi.push_str(#method_name);
        __abi.push_str("\",\"inputs\":[");

        let mut __first_input = true;
        #(#input_entries)*

        __abi.push_str("],\"outputs\":[");

        let mut __first_output = true;
        #(#output_entries)*

        __abi.push_str(#closing);
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

        let (helper, main_fn) = generate_abi_gen(&parsed, true);
        assert!(helper.is_empty());
        assert!(main_fn.is_empty());
    }

    fn expand_to_string(input: syn::ItemMod) -> String {
        expand_contract(ContractArgs::default(), input)
            .unwrap()
            .to_string()
    }

    #[test]
    fn payable_method_abi_has_payable_mutability() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                #[pvm_contract_macros::method]
                #[pvm_contract_macros::payable]
                pub fn deposit() {}
            }
        };
        let s = expand_to_string(input);
        assert!(
            s.contains(r#"\"stateMutability\":\"payable\""#),
            "payable method ABI must declare stateMutability = payable; got:\n{s}"
        );
    }

    #[test]
    fn non_payable_method_abi_has_nonpayable_mutability() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                #[pvm_contract_macros::method]
                pub fn transfer(to: Address) -> bool { false }
            }
        };
        let s = expand_to_string(input);
        assert!(
            s.contains(r#"\"stateMutability\":\"nonpayable\""#),
            "non-payable method ABI must declare stateMutability = nonpayable; got:\n{s}"
        );
        assert!(
            !s.contains(r#"\"stateMutability\":\"payable\""#),
            "non-payable-only contract must not declare any payable mutability; got:\n{s}"
        );
    }

    #[test]
    fn payable_constructor_abi_has_payable_mutability() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                #[pvm_contract_macros::constructor]
                #[pvm_contract_macros::payable]
                pub fn new() {}
            }
        };
        let s = expand_to_string(input);
        let ctor_marker = r#"\"type\":\"constructor\""#;
        assert!(
            s.contains(ctor_marker),
            "constructor entry marker missing; got:\n{s}"
        );
        let after_ctor = &s[s.find(ctor_marker).unwrap()..];
        assert!(
            after_ctor.contains(r#"\"stateMutability\":\"payable\""#),
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
        let (helper, _main_fn) = generate_abi_gen(&parsed, false);
        let s = helper.to_string();
        assert!(
            s.contains(r#"\"stateMutability\":\"view\""#),
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
        let (helper, _main_fn) = generate_abi_gen(&parsed, false);
        let s = helper.to_string();
        assert!(
            s.contains(r#"\"stateMutability\":\"pure\""#),
            "pure method ABI must declare stateMutability = pure; got:\n{s}"
        );
    }

    #[test]
    fn non_payable_constructor_abi_has_nonpayable_mutability() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                #[pvm_contract_macros::constructor]
                pub fn new(initial: U256) {}
            }
        };
        let s = expand_to_string(input);
        let ctor_marker = r#"\"type\":\"constructor\""#;
        assert!(
            s.contains(ctor_marker),
            "constructor entry marker missing; got:\n{s}"
        );
        let after_ctor = &s[s.find(ctor_marker).unwrap()..];
        assert!(
            after_ctor.contains(r#"\"stateMutability\":\"nonpayable\""#),
            "non-payable constructor must declare nonpayable; got:\n{after_ctor}"
        );
        assert!(
            !after_ctor.contains(r#"\"stateMutability\":\"payable\""#),
            "non-payable-only contract must not emit stateMutability = payable; got:\n{after_ctor}"
        );
    }
}

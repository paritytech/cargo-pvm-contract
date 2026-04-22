use proc_macro2::TokenStream;
use quote::quote;

use super::contract::ParsedContract;
use super::dispatch::MethodInfo;

/// Generate both the in-module ABI helper and the top-level `main()`.
///
/// The helper lives inside the user's module so all type imports are in scope.
/// The `main()` prints a JSON wrapper: `{"abi":[...],"cdm":"..."}` (or
/// `{"abi":[...]}` when `cdm` is unset). The builder splits the wrapper into
/// `<bin>.abi.json` (raw ABI array) and `<bin>.cdm.json` (CDM package name).
///
/// When a `.sol` file is provided, the builder derives the ABI from the
/// Solidity interface — macro-side ABI generation is skipped. The CDM helper
/// is still emitted so CDM metadata rides the same abi-gen run.
pub fn generate_abi_gen(
    parsed: &ParsedContract,
    has_sol_path: bool,
    cdm: Option<&str>,
) -> (TokenStream, TokenStream) {
    match generate_abi_gen_impl(parsed, has_sol_path, cdm) {
        Ok((helper, main_fn)) => (helper, main_fn),
        Err(err) => {
            let err = err.to_compile_error();
            (quote! {}, err)
        }
    }
}

fn generate_abi_gen_impl(
    parsed: &ParsedContract,
    has_sol_path: bool,
    cdm: Option<&str>,
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

        quote! {
            __items.push(::pvm_contract_sdk::AbiItem::Constructor {
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

    // Skip the ABI helper when a .sol file is provided — the builder derives
    // ABI from Solidity directly. We still need the CDM helper so the wrapper
    // main can surface CDM metadata to the builder.
    let abi_helper = if has_sol_path {
        quote! {}
    } else {
        quote! {
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
        }
    };

    let cdm_value: TokenStream = match cdm {
        Some(name) => quote! { ::core::option::Option::Some(#name) },
        None => quote! { ::core::option::Option::None },
    };
    let cdm_helper = quote! {
        #[cfg(feature = "abi-gen")]
        #[doc(hidden)]
        pub fn __cdm_package() -> ::core::option::Option<&'static str> {
            #cdm_value
        }
    };

    let helper = quote! {
        #abi_helper
        #cdm_helper
    };

    let mod_name = &parsed.mod_name;
    // When .sol drives ABI generation, the abi-gen binary can't call
    // __abi_json() (it isn't emitted). It still needs to emit a wrapper so
    // the builder can pick up CDM metadata; the ABI field is left as `null`
    // and the builder falls back to its .sol-based generator for the array.
    let abi_body = if has_sol_path {
        quote! { ::std::string::String::from("null") }
    } else {
        quote! { #mod_name::__abi_json() }
    };
    let main_fn = quote! {
        #[cfg(feature = "abi-gen")]
        fn main() {
            let __abi: ::std::string::String = #abi_body;
            let __cdm: ::core::option::Option<&'static str> = #mod_name::__cdm_package();
            match __cdm {
                ::core::option::Option::Some(__name) => {
                    ::std::println!(
                        "{{\"abi\":{},\"cdm\":{}}}",
                        __abi,
                        ::pvm_contract_sdk::serde_json::to_string(__name)
                            .expect("cdm package name must serialize"),
                    );
                }
                ::core::option::Option::None => {
                    ::std::println!("{{\"abi\":{}}}", __abi);
                }
            }
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

    // All methods are emitted with `"stateMutability":"payable"` because we don't yet
    // support `payable`/`nonpayable`/`view`/`pure` attributes on Rust methods.
    // Once state mutability attributes are added, this should be derived from the
    // method annotation instead of hardcoded.
    Ok(quote! {
        __items.push(::pvm_contract_sdk::AbiItem::Function {
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

    fn empty_parsed() -> ParsedContract {
        ParsedContract {
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
        }
    }

    #[test]
    fn sol_path_skips_abi_helper_but_emits_cdm_helper_and_main() {
        let (helper, main_fn) = generate_abi_gen(&empty_parsed(), true, None);
        let helper_s = helper.to_string();
        let main_s = main_fn.to_string();
        assert!(
            !helper_s.contains("__abi_json"),
            "sol-path contracts should not emit __abi_json"
        );
        assert!(
            helper_s.contains("__cdm_package"),
            "__cdm_package must be emitted for every contract"
        );
        assert!(main_s.contains("fn main"));
    }

    #[test]
    fn emits_cdm_package_none_when_unset() {
        let (helper, _) = generate_abi_gen(&empty_parsed(), false, None);
        assert!(
            helper
                .to_string()
                .contains(":: core :: option :: Option :: None"),
            "helper must return Option::None when cdm is unset"
        );
    }

    #[test]
    fn emits_cdm_package_value_when_set() {
        let (helper, _) = generate_abi_gen(&empty_parsed(), false, Some("@polkadot/reputation"));
        let helper_s = helper.to_string();
        assert!(helper_s.contains("@polkadot/reputation"));
        assert!(helper_s.contains(":: core :: option :: Option :: Some"));
    }

    #[test]
    fn main_wrapper_prints_cdm_field() {
        let (_, main_fn) = generate_abi_gen(&empty_parsed(), false, Some("@ns/tok"));
        let main_s = main_fn.to_string();
        // Both branches of the match on __cdm should be present
        assert!(main_s.contains("\\\"abi\\\":"));
        assert!(main_s.contains("\\\"cdm\\\":"));
    }
}

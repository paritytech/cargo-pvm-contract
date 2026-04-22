use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params};

/// Generate the error revert encoding for an `Err(e)` arm.
/// In alloc mode, uses a dynamically-sized `Vec<u8>` so dynamic error fields
/// are safe regardless of payload size.
/// In stack mode, uses a fixed 256-byte buffer.
pub(super) fn generate_revert_encoding(use_alloc: bool) -> TokenStream {
    if use_alloc {
        quote! {
            let __revert_len = ::pvm_contract_sdk::SolRevert::revert_data_len(&e);
            let mut __revert_buf = alloc::vec![0u8; __revert_len];
            ::pvm_contract_sdk::SolRevert::revert_data(&e, &mut __revert_buf);
            ::pvm_contract_sdk::PolkaVmHost::return_value(
                ::pvm_contract_sdk::ReturnFlags::REVERT, &__revert_buf);
        }
    } else {
        quote! {
            let mut __revert_buf = [0u8; 256];
            let __revert_len = ::pvm_contract_sdk::SolRevert::revert_data(&e, &mut __revert_buf);
            ::pvm_contract_sdk::PolkaVmHost::return_value(
                ::pvm_contract_sdk::ReturnFlags::REVERT, &__revert_buf[..__revert_len]);
        }
    }
}

pub struct MethodInfo {
    pub fn_name: syn::Ident,
    pub sol_name: String,
    pub param_names: Vec<syn::Ident>,
    pub param_types: Vec<syn::Type>,
    pub return_types: Vec<syn::Type>,
    pub returns_result: bool,
    /// When set, the selector is precomputed (e.g. from a `.sol` file).
    /// Otherwise it is derived at compile time from trait `SOL_NAME` constants.
    pub precomputed_selector: Option<[u8; 4]>,
}

pub(super) struct ParamDecoding {
    pub size_check: TokenStream,
    pub decode_statements: Vec<TokenStream>,
    pub call_args: Vec<TokenStream>,
}

pub(super) fn generate_param_decoding(
    param_names: &[syn::Ident],
    param_types: &[syn::Type],
) -> ParamDecoding {
    let decodes = generate_decode_params(param_types);
    let min_size_expr = calculate_min_input_size(param_types);

    let size_check = if !param_types.is_empty() {
        quote! {
            if input.len() < (#min_size_expr) {
                ::pvm_contract_sdk::PolkaVmHost::return_value(
                    ::pvm_contract_sdk::ReturnFlags::REVERT,
                    &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA);
            }
        }
    } else {
        quote! {}
    };

    let offset_init = if !param_types.is_empty() {
        quote! { let mut __decode_offset: usize = 0; }
    } else {
        quote! {}
    };

    let decode_statements = std::iter::once(offset_init)
        .chain(
            param_names
                .iter()
                .zip(decodes.iter())
                .map(|(name, decode)| {
                    quote! { let #name = #decode; }
                }),
        )
        .collect();

    let call_args = param_names
        .iter()
        .map(|name| quote!(::core::convert::Into::into(#name)))
        .collect();

    ParamDecoding {
        size_check,
        decode_statements,
        call_args,
    }
}

fn build_selector_const(method: &MethodInfo) -> TokenStream {
    let sel_ident = quote::format_ident!("__SEL_{}", method.fn_name);

    if let Some(selector) = method.precomputed_selector {
        let [s0, s1, s2, s3] = selector;
        quote! {
            const #sel_ident: [u8; 4] = [#s0, #s1, #s2, #s3];
        }
    } else {
        let sig_expr = build_const_signature_expr(method);
        quote! {
            const #sel_ident: [u8; 4] = ::pvm_contract_sdk::const_selector(#sig_expr);
        }
    }
}

fn build_const_signature_expr(method: &MethodInfo) -> TokenStream {
    let fn_name = &method.sol_name;
    let mut parts: Vec<TokenStream> = Vec::new();
    let prefix = format!("{}(", fn_name);
    parts.push(quote! { #prefix });

    for (i, ty) in method.param_types.iter().enumerate() {
        if i > 0 {
            parts.push(quote! { "," });
        }
        parts.push(quote! { <#ty as ::pvm_contract_sdk::SolEncode>::SOL_NAME });
    }

    parts.push(quote! { ")" });
    quote! { ::pvm_contract_sdk::const_format::concatcp!(#(#parts),*) }
}

pub fn generate_dispatch_arm(method: &MethodInfo, use_alloc: bool) -> (TokenStream, TokenStream) {
    let sel_ident = quote::format_ident!("__SEL_{}", method.fn_name);
    let const_def = build_selector_const(method);

    let fn_name = &method.fn_name;
    let decoding = generate_param_decoding(&method.param_names, &method.param_types);
    let ParamDecoding {
        size_check,
        decode_statements,
        call_args,
    } = decoding;
    let has_return = !method.return_types.is_empty();
    let encode_and_return = generate_encode_and_return(&method.return_types, use_alloc);

    let revert_err = generate_revert_encoding(use_alloc);

    let body = if method.returns_result {
        if has_return {
            quote! {
                match #fn_name(#(#call_args),*) {
                    Ok(result) => { #encode_and_return }
                    Err(e) => {
                        #revert_err
                    }
                }
            }
        } else {
            quote! {
                match #fn_name(#(#call_args),*) {
                    Ok(()) => return Some(()),
                    Err(e) => {
                        #revert_err
                    }
                }
            }
        }
    } else if has_return {
        quote! {
            let result = #fn_name(#(#call_args),*);
            #encode_and_return
        }
    } else {
        quote! {
            #fn_name(#(#call_args),*);
            return Some(());
        }
    };

    let match_arm = quote! {
        #sel_ident => {
            #size_check
            #(#decode_statements)*
            #body
        }
    };

    (const_def, match_arm)
}

/// Items generated inside the contract module for routing.
pub struct RouteItems {
    /// Unit struct used as the `Router` trait target.
    pub contract_struct: TokenStream,
    /// The `route(selector, input) -> Option<()>` function.
    pub route_fn: TokenStream,
}

/// `impl Router for mod_name::Contract` block, placed outside the module.
pub struct RouterImpl {
    pub tokens: TokenStream,
}

/// Generate the `route` function and `Router` trait impl for a contract module.
pub fn generate_router(
    methods: &[MethodInfo],
    mod_name: &syn::Ident,
    use_alloc: bool,
) -> (RouteItems, RouterImpl) {
    let (selector_consts, dispatch_arms): (Vec<_>, Vec<_>) = methods
        .iter()
        .map(|m| generate_dispatch_arm(m, use_alloc))
        .unzip();

    let route_items = RouteItems {
        contract_struct: quote! {
            /// Unit struct that implements [`::pvm_contract_sdk::Router`] for this contract.
            pub struct Contract;
        },
        route_fn: quote! {
            #[allow(non_upper_case_globals)]
            pub fn route(selector: [u8; 4], input: &[u8]) -> Option<()> {
                #(#selector_consts)*

                match selector {
                    #(#dispatch_arms)*
                    _ => None,
                }
            }
        },
    };

    let router_impl = RouterImpl {
        tokens: quote! {
            impl ::pvm_contract_sdk::Router for #mod_name::Contract {
                fn route(selector: [u8; 4], input: &[u8]) -> Option<()> {
                    #mod_name::route(selector, input)
                }
            }
        },
    };

    (route_items, router_impl)
}

fn generate_encode_and_return(outputs: &[syn::Type], use_alloc: bool) -> TokenStream {
    if outputs.is_empty() {
        return quote! { return Some(()); };
    }

    if use_alloc {
        generate_alloc_encode_and_return(outputs)
    } else {
        generate_static_encode_and_return(outputs)
    }
}

fn generate_static_encode_and_return(outputs: &[syn::Type]) -> TokenStream {
    if outputs.len() == 1 {
        let ty = &outputs[0];
        // Single static return: encode_to handles wrapping (no-op for static).
        // Use StaticEncodedLen for stack buffer since IS_DYNAMIC is const-false.
        return quote! {{
            const { assert!(
                !<#ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC,
                "dynamic types (String, Vec, Bytes) require allocator = \"pico\" or \"bump\""
            ) };
            let mut __buf = [0u8; <#ty as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE];
            <#ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf);
            ::pvm_contract_sdk::PolkaVmHost::return_value(
                ::pvm_contract_sdk::ReturnFlags::empty(), &__buf);
        }};
    }

    // Multi-return: result is a tuple. Use the tuple's encode_to (IS_TUPLE=true → flat body).
    let tuple_ty = quote! { (#(#outputs,)*) };
    quote! {{
        const { assert!(
            !<#tuple_ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC,
            "dynamic return types require allocator = \"pico\" or \"bump\""
        ) };
        let mut __buf = [0u8; <#tuple_ty as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE];
        <#tuple_ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf);
        ::pvm_contract_sdk::PolkaVmHost::return_value(
            ::pvm_contract_sdk::ReturnFlags::empty(), &__buf);
    }}
}

fn generate_alloc_encode_and_return(outputs: &[syn::Type]) -> TokenStream {
    if outputs.len() == 1 {
        let ty = &outputs[0];
        // IS_DYNAMIC is a const bool — the compiler eliminates the dead branch.
        // Static types use a stack buffer; dynamic types use a heap buffer.
        // The else branch includes a runtime guard to prevent buffer overflow
        // for the (unreachable) case where a dynamic type reaches the static path.
        // Single return: encode_to handles smart wrapping (IS_TUPLE + IS_DYNAMIC).
        return quote! {{
            let __len = <#ty as ::pvm_contract_sdk::SolEncode>::encode_len(&result);
            if <#ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC {
                let mut __buf = alloc::vec![0u8; __len];
                <#ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf);
                ::pvm_contract_sdk::PolkaVmHost::return_value(
                    ::pvm_contract_sdk::ReturnFlags::empty(), &__buf);
            } else {
                let mut __buf = [0u8; <#ty as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE];
                <#ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf[..__len]);
                ::pvm_contract_sdk::PolkaVmHost::return_value(
                    ::pvm_contract_sdk::ReturnFlags::empty(), &__buf[..__len]);
            }
        }};
    }

    // Multi-return: result is a tuple. Use the tuple's encode_to (IS_TUPLE=true → flat body).
    let tuple_ty = quote! { (#(#outputs,)*) };
    quote! {{
        let __len = <#tuple_ty as ::pvm_contract_sdk::SolEncode>::encode_len(&result);
        let mut __buf = alloc::vec![0u8; __len];
        <#tuple_ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf);
        ::pvm_contract_sdk::PolkaVmHost::return_value(
            ::pvm_contract_sdk::ReturnFlags::empty(), &__buf);
    }}
}

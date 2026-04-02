use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params};
use super::encode::generate_encode;

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
                pallet_revive_uapi::HostFnImpl::return_value(
                    pallet_revive_uapi::ReturnFlags::REVERT, b"InvalidCalldata");
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
            const #sel_ident: [u8; 4] = ::pvm_contract_types::const_selector(#sig_expr);
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
        parts.push(quote! { <#ty as ::pvm_contract_types::SolEncode>::SOL_NAME });
    }

    parts.push(quote! { ")" });
    quote! { ::pvm_contract_types::const_format::concatcp!(#(#parts),*) }
}

fn build_output_size_expr(outputs: &[syn::Type]) -> TokenStream {
    let size_exprs: Vec<TokenStream> = outputs
        .iter()
        .map(|ty| quote! { <#ty as ::pvm_contract_types::SolEncode>::HEAD_SIZE })
        .collect();
    quote! { 0 #(+ #size_exprs)* }
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

    let body = if method.returns_result {
        if has_return {
            quote! {
                match #fn_name(#(#call_args),*) {
                    Ok(result) => { #encode_and_return }
                    Err(e) => {
                        pallet_revive_uapi::HostFnImpl::return_value(
                            pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
                    }
                }
            }
        } else {
            quote! {
                match #fn_name(#(#call_args),*) {
                    Ok(()) => return Some(()),
                    Err(e) => {
                        pallet_revive_uapi::HostFnImpl::return_value(
                            pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
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
            /// Unit struct that implements [`::pvm_contract_types::Router`] for this contract.
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
            impl ::pvm_contract_types::Router for #mod_name::Contract {
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
        let encode = generate_encode(ty, quote!(result));
        return quote! {
            let encoded = #encode;
            pallet_revive_uapi::HostFnImpl::return_value(
                pallet_revive_uapi::ReturnFlags::empty(), &encoded);
        };
    }

    let encodes: Vec<_> = outputs
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let idx = syn::Index::from(i);
            generate_encode(ty, quote!(result.#idx))
        })
        .collect();

    let total_size_expr = build_output_size_expr(outputs);

    quote! {{
        const __OUT_SIZE: usize = #total_size_expr;
        let mut out = [0u8; __OUT_SIZE];
        let mut offset = 0;
        #(
            let encoded = #encodes;
            out[offset..offset + encoded.len()].copy_from_slice(&encoded);
            offset += encoded.len();
        )*
        pallet_revive_uapi::HostFnImpl::return_value(
            pallet_revive_uapi::ReturnFlags::empty(), &out);
    }}
}

fn generate_alloc_encode_and_return(outputs: &[syn::Type]) -> TokenStream {
    if outputs.len() == 1 {
        let ty = &outputs[0];
        // Use IS_DYNAMIC const to let the compiler eliminate the dead branch.
        // Static types (u64, U256, …) get a stack buffer; dynamic types (String, …) get a heap
        // buffer. This avoids pulling allocator code into contracts that only return static types.
        return quote! {{
            if <#ty as ::pvm_contract_types::SolEncode>::IS_DYNAMIC {
                let __len = <#ty as ::pvm_contract_types::SolEncode>::encode_len(&result);
                let mut __buf = alloc::vec![0u8; __len];
                <#ty as ::pvm_contract_types::SolEncode>::encode_to(&result, &mut __buf);
                pallet_revive_uapi::HostFnImpl::return_value(
                    pallet_revive_uapi::ReturnFlags::empty(), &__buf);
            } else {
                let mut __buf = [0u8; <#ty as ::pvm_contract_types::SolEncode>::HEAD_SIZE];
                <#ty as ::pvm_contract_types::SolEncode>::encode_to(&result, &mut __buf);
                pallet_revive_uapi::HostFnImpl::return_value(
                    pallet_revive_uapi::ReturnFlags::empty(), &__buf);
            }
        }};
    }

    let head_size_expr = build_output_size_expr(outputs);

    let encodes: Vec<_> = outputs
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let idx = syn::Index::from(i);
            let value_expr = quote!(result.#idx);

            quote! {
                if <#ty as ::pvm_contract_types::SolEncode>::IS_DYNAMIC {
                    let __off = __head_size + tail.len();
                    let mut __off_buf = [0u8; 32];
                    __off_buf[24..32].copy_from_slice(&(__off as u64).to_be_bytes());
                    head.extend_from_slice(&__off_buf);
                    let __tl = <#ty as ::pvm_contract_types::SolEncode>::tail_len(&#value_expr);
                    let mut __tbuf = alloc::vec![0u8; __tl];
                    <#ty as ::pvm_contract_types::SolEncode>::encode_tail_to(&#value_expr, &mut __tbuf);
                    tail.extend_from_slice(&__tbuf);
                } else {
                    let __hs: usize = <#ty as ::pvm_contract_types::SolEncode>::HEAD_SIZE;
                    let __start = head.len();
                    head.resize(__start + __hs, 0);
                    <#ty as ::pvm_contract_types::SolEncode>::encode_to(
                        &#value_expr,
                        &mut head[__start..__start + __hs],
                    );
                }
            }
        })
        .collect();

    quote! {{
        let __head_size: usize = #head_size_expr;
        let mut head = alloc::vec::Vec::with_capacity(__head_size);
        let mut tail = alloc::vec::Vec::new();
        #(#encodes)*
        head.extend_from_slice(&tail);
        pallet_revive_uapi::HostFnImpl::return_value(
            pallet_revive_uapi::ReturnFlags::empty(), &head);
    }}
}

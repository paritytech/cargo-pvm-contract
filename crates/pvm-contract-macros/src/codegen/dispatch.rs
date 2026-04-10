use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params, has_custom_types};
use super::encode::{generate_dynamic_value_encode, generate_encode};
use super::sol_type::{sol_type_head_size_expr, sol_type_is_dynamic_expr, sol_type_name_parts};
use crate::signature::{FunctionSignature, SolType, compute_selector};

pub struct MethodInfo {
    pub fn_name: syn::Ident,
    pub signature: FunctionSignature,
    pub param_names: Vec<syn::Ident>,
    pub returns_result: bool,
}

pub(super) struct ParamDecoding {
    pub size_check: TokenStream,
    pub decode_statements: Vec<TokenStream>,
    pub call_args: Vec<TokenStream>,
}

pub(super) fn generate_param_decoding(
    param_names: &[syn::Ident],
    sol_types: &[SolType],
    use_alloc: bool,
) -> ParamDecoding {
    let decodes = generate_decode_params(sol_types, use_alloc);
    let min_size_expr = calculate_min_input_size(sol_types);

    let size_check = if !sol_types.is_empty() {
        quote! {
            if input.len() < (#min_size_expr) {
                pallet_revive_uapi::HostFnImpl::return_value(
                    pallet_revive_uapi::ReturnFlags::REVERT, b"InvalidCalldata");
            }
        }
    } else {
        quote! {}
    };

    let needs_runtime_offset = has_custom_types(sol_types);
    let offset_init = if needs_runtime_offset {
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

fn build_const_signature_expr(method: &MethodInfo) -> TokenStream {
    let fn_name = &method.signature.name;
    let mut parts: Vec<TokenStream> = Vec::new();
    let prefix = format!("{}(", fn_name);
    parts.push(quote! { #prefix });

    for (i, input_type) in method.signature.inputs.iter().enumerate() {
        if i > 0 {
            parts.push(quote! { "," });
        }
        sol_type_name_parts(input_type, &mut parts);
    }

    parts.push(quote! { ")" });
    quote! { ::pvm_contract_types::const_format::concatcp!(#(#parts),*) }
}

fn build_output_size_expr(outputs: &[SolType]) -> TokenStream {
    let has_custom = outputs.iter().any(|t| t.has_custom_types());

    if !has_custom {
        let total: usize = outputs
            .iter()
            .map(|t| {
                t.head_size()
                    .expect("build_output_size_expr called on unresolved custom type")
            })
            .sum();
        return quote! { #total };
    }

    let size_exprs: Vec<TokenStream> = outputs.iter().map(sol_type_head_size_expr).collect();

    quote! { 0 #(+ #size_exprs)* }
}

pub fn generate_dispatch_arm(
    method: &MethodInfo,
    mod_name: &syn::Ident,
    use_alloc: bool,
) -> (TokenStream, TokenStream) {
    let sel_ident = quote::format_ident!("__SEL_{}", method.fn_name);
    let has_custom_inputs = method.signature.inputs.iter().any(|t| t.has_custom_types());

    let const_def = if has_custom_inputs {
        let sig_expr = build_const_signature_expr(method);
        quote! {
            const #sel_ident: [u8; 4] = ::pvm_contract_types::const_selector(#sig_expr);
        }
    } else {
        let selector = compute_selector(&method.signature.canonical_signature());
        let [s0, s1, s2, s3] = selector;
        quote! {
            const #sel_ident: [u8; 4] = [#s0, #s1, #s2, #s3];
        }
    };

    let fn_name = &method.fn_name;
    let decoding =
        generate_param_decoding(&method.param_names, &method.signature.inputs, use_alloc);
    let ParamDecoding {
        size_check,
        decode_statements,
        call_args,
    } = decoding;
    let has_return = !method.signature.outputs.is_empty();
    let encode_and_return = generate_encode_and_return(&method.signature.outputs, use_alloc);

    let body = if method.returns_result {
        if has_return {
            quote! {
                match #mod_name::#fn_name(#(#call_args),*) {
                    Ok(result) => { #encode_and_return }
                    Err(e) => {
                        pallet_revive_uapi::HostFnImpl::return_value(
                            pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
                    }
                }
            }
        } else {
            quote! {
                match #mod_name::#fn_name(#(#call_args),*) {
                    Ok(()) => return,
                    Err(e) => {
                        pallet_revive_uapi::HostFnImpl::return_value(
                            pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
                    }
                }
            }
        }
    } else if has_return {
        quote! {
            let result = #mod_name::#fn_name(#(#call_args),*);
            #encode_and_return
        }
    } else {
        quote! {
            #mod_name::#fn_name(#(#call_args),*);
            return;
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

fn has_known_dynamic_outputs(outputs: &[SolType]) -> bool {
    outputs.iter().any(|t| t.is_dynamic() == Some(true))
}

fn has_custom_outputs(outputs: &[SolType]) -> bool {
    outputs.iter().any(|t| t.has_custom_types())
}

fn generate_encode_and_return(outputs: &[SolType], use_alloc: bool) -> TokenStream {
    if outputs.is_empty() {
        return quote! { return; };
    }

    let has_known_dynamic = has_known_dynamic_outputs(outputs);
    let has_custom = has_custom_outputs(outputs);

    if has_known_dynamic && !use_alloc {
        let type_name = outputs
            .iter()
            .find(|t| t.is_dynamic() == Some(true))
            .map(|t| t.canonical_name())
            .unwrap_or_else(|| "dynamic".to_string());
        let msg = format!(
            "Return type `{type_name}` is dynamic and requires an explicit allocator. Set `allocator = \"pico\"` or `allocator = \"bump\"` in `#[contract]`, or use static types."
        );
        return quote! {
            compile_error!(#msg);
        };
    }

    if (has_known_dynamic || has_custom) && use_alloc {
        return generate_dynamic_encode_and_return(outputs);
    }

    if outputs.len() == 1 {
        let value_expr = match &outputs[0] {
            SolType::FixedArray(..) | SolType::Tuple(..) => quote!(result),
            _ => quote!(::core::convert::Into::into(result)),
        };
        let encode = generate_encode(&outputs[0], value_expr, use_alloc);
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
            generate_encode(
                ty,
                quote!(::core::convert::Into::into(result.#idx)),
                use_alloc,
            )
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

fn generate_dynamic_encode_and_return(outputs: &[SolType]) -> TokenStream {
    if outputs.len() == 1 {
        // DynBytes needs special handling: Vec<u8> as `bytes` must be encoded as
        // raw bytes (offset + length + data), not as uint8[] array of 32-byte words.
        if matches!(&outputs[0], SolType::DynBytes) {
            let encode_tail = generate_dynamic_value_encode(&outputs[0], quote!(result));
            return quote! {{
                let tail_data = #encode_tail;
                let mut buf = alloc::vec::Vec::with_capacity(32 + tail_data.len());
                let mut __off_buf = [0u8; 32];
                __off_buf[24..32].copy_from_slice(&(32u64).to_be_bytes());
                buf.extend_from_slice(&__off_buf);
                buf.extend_from_slice(&tail_data);
                pallet_revive_uapi::HostFnImpl::return_value(
                    pallet_revive_uapi::ReturnFlags::empty(), &buf);
            }};
        }
        return quote! {{
            let len = ::pvm_contract_types::SolEncode::encode_len(&result);
            let mut buf = alloc::vec![0u8; len];
            ::pvm_contract_types::SolEncode::encode_to(&result, &mut buf);
            pallet_revive_uapi::HostFnImpl::return_value(
                pallet_revive_uapi::ReturnFlags::empty(), &buf);
        }};
    }

    let head_size_expr = build_output_size_expr(outputs);

    let encodes: Vec<_> = outputs
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let idx = syn::Index::from(i);
            let value_expr = quote!(result.#idx);

            if ty.is_dynamic() == Some(true) {
                let encode_tail = generate_dynamic_value_encode(ty, value_expr);
                quote! {
                    let __off = __head_size + tail.len();
                    let mut __off_buf = [0u8; 32];
                    __off_buf[24..32].copy_from_slice(&(__off as u64).to_be_bytes());
                    head.extend_from_slice(&__off_buf);
                    let encoded = #encode_tail;
                    tail.extend_from_slice(&encoded);
                }
            } else if ty.is_dynamic() == Some(false) {
                let encode = generate_encode(ty, value_expr, true);
                quote! {
                    let encoded = #encode;
                    head.extend_from_slice(&encoded);
                }
            } else {
                let is_dyn_expr = sol_type_is_dynamic_expr(ty);
                let hs_expr = sol_type_head_size_expr(ty);
                quote! {
                    if #is_dyn_expr {
                        let __off = __head_size + tail.len();
                        let mut __off_buf = [0u8; 32];
                        __off_buf[24..32].copy_from_slice(&(__off as u64).to_be_bytes());
                        head.extend_from_slice(&__off_buf);
                        let __tl = ::pvm_contract_types::SolEncode::tail_len(&#value_expr);
                        let mut __tbuf = alloc::vec![0u8; __tl];
                        ::pvm_contract_types::SolEncode::encode_tail_to(&#value_expr, &mut __tbuf);
                        tail.extend_from_slice(&__tbuf);
                    } else {
                        let __hs: usize = #hs_expr;
                        let __start = head.len();
                        head.resize(__start + __hs, 0);
                        ::pvm_contract_types::SolEncode::encode_to(
                            &#value_expr,
                            &mut head[__start..__start + __hs],
                        );
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn normalize_tokens(ts: TokenStream) -> String {
        ts.to_string().split_whitespace().collect::<String>()
    }

    #[test]
    fn generate_dispatch_arm_uses_dynamic_encoding_for_string_return_in_alloc_mode() {
        let method = MethodInfo {
            fn_name: syn::parse_str("greeting").unwrap(),
            signature: FunctionSignature {
                name: "greeting".to_string(),
                inputs: vec![],
                outputs: vec![SolType::String],
            },
            param_names: vec![],
            returns_result: false,
        };
        let mod_name: syn::Ident = syn::parse_str("contract").unwrap();

        let (_const_def, match_arm) = generate_dispatch_arm(&method, &mod_name, true);

        let expected = quote! {
            __SEL_greeting => {
                let result = contract::greeting();
                {
                    let len = ::pvm_contract_types::SolEncode::encode_len(&result);
                    let mut buf = alloc::vec![0u8; len];
                    ::pvm_contract_types::SolEncode::encode_to(&result, &mut buf);
                    pallet_revive_uapi::HostFnImpl::return_value(
                        pallet_revive_uapi::ReturnFlags::empty(),
                        &buf
                    );
                }
            }
        };

        assert_eq!(normalize_tokens(match_arm), normalize_tokens(expected));
    }

    #[test]
    fn custom_output_routes_to_dynamic_path_with_alloc() {
        // Custom types may be dynamic, so with alloc they use the dynamic
        // encoding path (encode_len + encode_to) instead of StaticEncodedLen.
        let outputs = vec![SolType::Custom("NamedPoint".to_string())];
        assert!(has_custom_outputs(&outputs));
    }

    #[test]
    fn generate_dispatch_arm_emits_compile_error_for_string_return_in_stack_mode() {
        let method = MethodInfo {
            fn_name: syn::parse_str("greeting").unwrap(),
            signature: FunctionSignature {
                name: "greeting".to_string(),
                inputs: vec![],
                outputs: vec![SolType::String],
            },
            param_names: vec![],
            returns_result: false,
        };
        let mod_name: syn::Ident = syn::parse_str("contract").unwrap();

        let (_const_def, match_arm) = generate_dispatch_arm(&method, &mod_name, false);

        let expected = quote! {
            __SEL_greeting => {
                let result = contract::greeting();
                compile_error!(
                    "Return type `string` is dynamic and requires an explicit allocator. Set `allocator = \"pico\"` or `allocator = \"bump\"` in `#[contract]`, or use static types."
                );
            }
        };

        assert_eq!(normalize_tokens(match_arm), normalize_tokens(expected));
    }

    #[test]
    fn has_known_dynamic_returns_false_for_custom_only() {
        let outputs = vec![SolType::Custom("Point".to_string())];
        assert!(
            !has_known_dynamic_outputs(&outputs),
            "Custom-only outputs: known-dynamic is false"
        );
        assert!(
            has_custom_outputs(&outputs),
            "Custom-only outputs: has_custom is true"
        );
    }

    #[test]
    fn has_known_dynamic_returns_true_for_string() {
        let outputs = vec![SolType::String];
        assert!(has_known_dynamic_outputs(&outputs));
    }
}

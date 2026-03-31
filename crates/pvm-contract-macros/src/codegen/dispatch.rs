use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params, has_custom_types};
use super::encode::{generate_dynamic_value_encode, generate_encode};
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
    let min_size = calculate_min_input_size(sol_types);

    let size_check = if min_size > 0 {
        quote! {
            if input.len() < #min_size {
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

pub fn generate_dispatch_arm(
    method: &MethodInfo,
    mod_name: &syn::Ident,
    use_alloc: bool,
) -> TokenStream {
    let selector = compute_selector(&method.signature.canonical_signature());
    let [s0, s1, s2, s3] = selector;

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

    quote! {
        [#s0, #s1, #s2, #s3] => {
            #size_check
            #(#decode_statements)*
            #body
        }
    }
}

fn has_dynamic_outputs(outputs: &[SolType]) -> bool {
    outputs.iter().any(|t| t.is_dynamic())
}

fn generate_encode_and_return(outputs: &[SolType], use_alloc: bool) -> TokenStream {
    if outputs.is_empty() {
        return quote! { return; };
    }

    let has_dynamic = has_dynamic_outputs(outputs);

    if has_dynamic && !use_alloc {
        let type_name = outputs
            .iter()
            .find(|t| t.is_dynamic())
            .map(|t| t.canonical_name())
            .unwrap_or_else(|| "dynamic".to_string());
        let msg = format!(
            "Return type `{type_name}` is dynamic and requires an explicit allocator. Set `allocator = \"pico\"` or `allocator = \"bump\"` in `#[contract]`, or use static types."
        );
        return quote! {
            compile_error!(#msg);
        };
    }

    if has_dynamic {
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

    let total_size: usize = outputs.iter().map(|t| t.head_size()).sum();

    quote! {{
        let mut out = [0u8; #total_size];
        let mut offset = 0;
        #(
            let encoded = #encodes;
            out[offset..offset + 32].copy_from_slice(&encoded);
            offset += 32;
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
                let offset_value = ruint::aliases::U256::from(32u64);
                let mut offset_buf = [0u8; 32];
                <ruint::aliases::U256 as ::pvm_contract_types::SolEncode>::encode_to(
                    &offset_value, &mut offset_buf);
                buf.extend_from_slice(&offset_buf);
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

    let head_size: usize = outputs.iter().map(|t| t.head_size()).sum();

    let encodes: Vec<_> = outputs
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let value_expr = if outputs.len() == 1 {
                quote!(result)
            } else {
                let idx = syn::Index::from(i);
                quote!(result.#idx)
            };

            if ty.is_dynamic() {
                let encode_tail = generate_dynamic_value_encode(ty, value_expr);
                quote! {
                    let offset = #head_size + tail.len();
                    let offset_value = ruint::aliases::U256::from(offset);
                    let mut offset_buf = [0u8; 32];
                    <ruint::aliases::U256 as ::pvm_contract_types::SolEncode>::encode_to(
                        &offset_value,
                        &mut offset_buf,
                    );
                    head.extend_from_slice(&offset_buf);
                    let encoded = #encode_tail;
                    tail.extend_from_slice(&encoded);
                }
            } else {
                let encode = generate_encode(ty, value_expr, true);
                quote! {
                    let encoded = #encode;
                    head.extend_from_slice(&encoded);
                }
            }
        })
        .collect();

    quote! {{
        let mut head = alloc::vec::Vec::with_capacity(#head_size);
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

        let arm = generate_dispatch_arm(&method, &mod_name, true).to_string();

        assert!(arm.contains("encode_len"));
        assert!(!arm.contains("compile_error"));
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

        let arm = generate_dispatch_arm(&method, &mod_name, false).to_string();

        assert!(arm.contains("compile_error"));
        assert!(arm.contains("requires an explicit allocator"));
    }
}

use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params};
use super::encode::{generate_encode, generate_encode_return};
use crate::signature::{compute_selector, FunctionSignature};

pub struct MethodInfo {
    pub fn_name: syn::Ident,
    pub signature: FunctionSignature,
    pub param_names: Vec<syn::Ident>,
    pub returns_result: bool,
}

pub fn generate_dispatch_arm(
    method: &MethodInfo,
    mod_name: &syn::Ident,
    use_alloc: bool,
) -> TokenStream {
    let selector = compute_selector(&method.signature.canonical_signature());
    let [s0, s1, s2, s3] = selector;

    let fn_name = &method.fn_name;
    let param_names = &method.param_names;
    let decodes = generate_decode_params(&method.signature.inputs, use_alloc);

    let min_size = calculate_min_input_size(&method.signature.inputs);

    let size_check = if min_size > 0 {
        let min_size_lit = min_size;
        if use_alloc {
            quote! {
                if input.len() < #min_size_lit {
                    return Err(b"InvalidCalldata".to_vec());
                }
            }
        } else {
            quote! {
                if input.len() < #min_size_lit {
                    pallet_revive_uapi::HostFnImpl::return_value(pallet_revive_uapi::ReturnFlags::REVERT, b"InvalidCalldata");
                }
            }
        }
    } else {
        quote! {}
    };

    let decode_statements: Vec<_> = param_names
        .iter()
        .zip(decodes.iter())
        .map(|(name, decode)| quote! { let #name = #decode; })
        .collect();

    let call_args: Vec<_> = param_names.iter().map(|name| quote!(#name)).collect();

    let has_return = !method.signature.outputs.is_empty();

    if use_alloc {
        let encode_return = generate_encode_return(&method.signature.outputs, use_alloc);
        let result_handling = if method.returns_result {
            if has_return {
                quote! {
                    #mod_name::#fn_name(#(#call_args),*).map(|result| {
                        Some(#encode_return)
                    }).map_err(|e| e.as_ref().to_vec())
                }
            } else {
                quote! {
                    #mod_name::#fn_name(#(#call_args),*).map(|()| None).map_err(|e| e.as_ref().to_vec())
                }
            }
        } else {
            if has_return {
                quote! {
                    Ok(Some({
                        let result = #mod_name::#fn_name(#(#call_args),*);
                        #encode_return
                    }))
                }
            } else {
                quote! {{
                    #mod_name::#fn_name(#(#call_args),*);
                    Ok(None)
                }}
            }
        };

        quote! {
            [#s0, #s1, #s2, #s3] => {
                #size_check
                #(#decode_statements)*
                #result_handling
            }
        }
    } else {
        generate_no_alloc_dispatch_arm(
            method,
            mod_name,
            selector,
            &size_check,
            &decode_statements,
            &call_args,
        )
    }
}

fn generate_no_alloc_dispatch_arm(
    method: &MethodInfo,
    mod_name: &syn::Ident,
    selector: [u8; 4],
    size_check: &TokenStream,
    decode_statements: &[TokenStream],
    call_args: &[TokenStream],
) -> TokenStream {
    let [s0, s1, s2, s3] = selector;
    let fn_name = &method.fn_name;
    let has_return = !method.signature.outputs.is_empty();

    let body = if method.returns_result {
        if has_return {
            let encode_and_return = generate_no_alloc_encode_and_return(&method.signature.outputs);
            quote! {
                match #mod_name::#fn_name(#(#call_args),*) {
                    Ok(result) => {
                        #encode_and_return
                    }
                    Err(e) => {
                        pallet_revive_uapi::HostFnImpl::return_value(pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
                    }
                }
            }
        } else {
            quote! {
                match #mod_name::#fn_name(#(#call_args),*) {
                    Ok(()) => return,
                    Err(e) => {
                        pallet_revive_uapi::HostFnImpl::return_value(pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
                    }
                }
            }
        }
    } else {
        if has_return {
            let encode_and_return = generate_no_alloc_encode_and_return(&method.signature.outputs);
            quote! {
                let result = #mod_name::#fn_name(#(#call_args),*);
                #encode_and_return
            }
        } else {
            quote! {
                #mod_name::#fn_name(#(#call_args),*);
                return;
            }
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

fn generate_no_alloc_encode_and_return(outputs: &[crate::signature::SolType]) -> TokenStream {
    if outputs.is_empty() {
        return quote! {};
    }

    if outputs.len() == 1 {
        let encode = generate_encode(&outputs[0], quote!(result), false);
        return quote! {
            let encoded = #encode;
            pallet_revive_uapi::HostFnImpl::return_value(pallet_revive_uapi::ReturnFlags::empty(), &encoded);
        };
    }

    let encodes: Vec<_> = outputs
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let idx = syn::Index::from(i);
            generate_encode(ty, quote!(result.#idx), false)
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
        pallet_revive_uapi::HostFnImpl::return_value(pallet_revive_uapi::ReturnFlags::empty(), &out);
    }}
}

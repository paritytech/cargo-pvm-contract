use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params};
use super::encode::generate_encode_return;
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
                    return Err(b"InvalidCalldata");
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
    let encode_return = generate_encode_return(&method.signature.outputs, use_alloc);

    let result_handling = if method.returns_result {
        if use_alloc {
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
                    #mod_name::#fn_name(#(#call_args),*).map(|result| {
                        Some(#encode_return)
                    }).map_err(|e| e.as_ref())
                }
            } else {
                quote! {
                    #mod_name::#fn_name(#(#call_args),*).map(|()| None).map_err(|e| e.as_ref())
                }
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
}

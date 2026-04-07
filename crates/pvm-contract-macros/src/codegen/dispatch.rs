use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params};
use super::encode::generate_encode_return;
use crate::signature::{compute_selector, FunctionSignature};

pub struct MethodInfo {
    pub fn_name: syn::Ident,
    pub signature: FunctionSignature,
    pub param_names: Vec<syn::Ident>,
    pub param_types: Vec<syn::Type>,
    pub return_type: Option<syn::Type>,
    pub returns_result: bool,
    pub all_inputs_resolved: bool,
    pub return_resolved: bool,
}

pub fn generate_dispatch_arm(
    method: &MethodInfo,
    mod_name: &syn::Ident,
    use_alloc: bool,
) -> TokenStream {
    if !method.all_inputs_resolved {
        if !use_alloc {
            let fn_name_str = method.fn_name.to_string();
            return quote! {
                compile_error!(concat!("Trait-based dispatch requires alloc for method: ", #fn_name_str));
            };
        }
        return generate_trait_dispatch_arm(method, mod_name);
    }

    // All inputs resolved — legacy decode path
    let fn_name = &method.fn_name;
    let selector = compute_selector(&method.signature.canonical_signature());
    let [s0, s1, s2, s3] = selector;

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

    let result_handling = if method.return_resolved {
        // Legacy result handling — types are fully known at macro time
        let has_return = !method.signature.outputs.is_empty();
        let encode_return = generate_encode_return(&method.signature.outputs, use_alloc);

        if method.returns_result {
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
        }
    } else {
        // Trait-based return encoding
        generate_trait_result_handling(method, mod_name, &call_args)
    };

    quote! {
        [#s0, #s1, #s2, #s3] => {
            #size_check
            #(#decode_statements)*
            #result_handling
        }
    }
}

/// Generate an if-else dispatch block for a method where input types are not all resolvable.
/// This produces a block (not a match arm) that uses `return` to exit early when matched.
pub fn generate_trait_dispatch_arm(
    method: &MethodInfo,
    mod_name: &syn::Ident,
) -> TokenStream {
    let param_names = &method.param_names;
    let param_types = &method.param_types;
    let sol_name = &method.signature.name;

    // Build the selector computation using SolAbi::SOL_NAME for each param type
    let sol_name_exprs: Vec<TokenStream> = param_types.iter().map(|ty| {
        quote! { <#ty as pvm_contract::SolAbi>::SOL_NAME }
    }).collect();

    // Build min size computation using the tuple slot size for each parameter.
    let head_size_exprs: Vec<TokenStream> = param_types.iter().map(|ty| {
        quote! { <#ty as pvm_contract::SolAbi>::SLOT_SIZE }
    }).collect();

    // Build decode statements using SolAbi::abi_decode
    let decode_statements: Vec<TokenStream> = param_names.iter().zip(param_types.iter()).map(|(name, ty)| {
        quote! {
            let #name = <#ty as pvm_contract::SolAbi>::abi_decode(input, __offset);
            __offset += <#ty as pvm_contract::SolAbi>::SLOT_SIZE;
        }
    }).collect();

    let call_args: Vec<_> = param_names.iter().map(|name| quote!(#name)).collect();

    let result_handling = generate_trait_result_handling(method, mod_name, &call_args);

    let min_size_calc = if head_size_exprs.is_empty() {
        quote! { let __min_size: usize = 0; }
    } else {
        quote! {
            let __min_size: usize = 0usize #(+ #head_size_exprs)*;
        }
    };

    let size_check = if !param_types.is_empty() {
        quote! {
            if input.len() < __min_size {
                return Err(b"InvalidCalldata".to_vec());
            }
        }
    } else {
        quote! {}
    };

    quote! {
        {
            let __sel = pvm_contract::compute_selector(#sol_name, &[
                #(#sol_name_exprs),*
            ]);
            if selector == __sel {
                #min_size_calc
                #size_check
                let mut __offset: usize = 0;
                #(#decode_statements)*
                #result_handling
            }
        }
    }
}

/// Shared helper: generate result handling for trait-based return encoding.
fn generate_trait_result_handling(
    method: &MethodInfo,
    mod_name: &syn::Ident,
    call_args: &[TokenStream],
) -> TokenStream {
    let fn_name = &method.fn_name;

    match (&method.return_type, method.returns_result) {
        // No return type, returns Result<(), Error>
        (None, true) => {
            quote! {
                return #mod_name::#fn_name(#(#call_args),*).map(|()| None).map_err(|e| e.as_ref().to_vec());
            }
        }
        // No return type, no Result
        (None, false) => {
            quote! {
                #mod_name::#fn_name(#(#call_args),*);
                return Ok(None);
            }
        }
        // Has return type, returns Result<T, Error>
        (Some(_ret_ty), true) => {
            quote! {
                return #mod_name::#fn_name(#(#call_args),*).map(|result| {
                    Some(pvm_contract::abi::encode_return_value(&result))
                }).map_err(|e| e.as_ref().to_vec());
            }
        }
        // Has return type, no Result
        (Some(_ret_ty), false) => {
            quote! {
                let result = #mod_name::#fn_name(#(#call_args),*);
                return Ok(Some(pvm_contract::abi::encode_return_value(&result)));
            }
        }
    }
}

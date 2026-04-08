use proc_macro2::TokenStream;
use quote::quote;

/// Generate decode expressions for each parameter type.
///
/// Each expression reads from `input` at `__decode_offset` and advances
/// `__decode_offset` by the type's `HEAD_SIZE`. The caller must emit
/// `let mut __decode_offset: usize = 0;` before using these.
pub fn generate_decode_params(types: &[syn::Type]) -> Vec<TokenStream> {
    types
        .iter()
        .map(|ty| {
            // Dynamic types occupy a 32-byte offset slot in the head,
            // even if their internal HEAD_SIZE is larger (e.g. dynamic structs).
            quote! {{
                let __value = <#ty as ::pvm_contract_types::SolDecode>::decode_at(&input, __decode_offset);
                __decode_offset += if <#ty as ::pvm_contract_types::SolEncode>::IS_DYNAMIC {
                    32
                } else {
                    <#ty as ::pvm_contract_types::SolEncode>::HEAD_SIZE
                };
                __value
            }}
        })
        .collect()
}

/// Build a compile-time expression for the minimum required input size.
pub fn calculate_min_input_size(types: &[syn::Type]) -> TokenStream {
    if types.is_empty() {
        return quote! { 0 };
    }
    let size_exprs: Vec<TokenStream> = types
        .iter()
        .map(|ty| {
            quote! {
                if <#ty as ::pvm_contract_types::SolEncode>::IS_DYNAMIC {
                    32usize
                } else {
                    <#ty as ::pvm_contract_types::SolEncode>::HEAD_SIZE
                }
            }
        })
        .collect();
    quote! { 0 #(+ #size_exprs)* }
}

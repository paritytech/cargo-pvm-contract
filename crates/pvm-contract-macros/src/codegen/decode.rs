use proc_macro2::TokenStream;
use quote::quote;

/// Generate decode expressions for each parameter type.
///
/// Each expression reads from `input` at `__decode_offset` and advances
/// `__decode_offset` by the type's `HEAD_SIZE`. The caller must emit
/// `let mut __decode_offset: usize = 0;` before using these.
pub fn generate_decode_params(
    names: &[syn::Ident],
    types: &[syn::Type],
    unit_return: bool,
) -> TokenStream {
    if names.is_empty() {
        quote! {}
    } else if unit_return {
        quote! {
            let Ok((#(#names),*)) = <(#(#types),*) as ::pvm_contract_sdk::SolDecode>::decode(&input,0) else {
                 revert(this);
                 return();
            };
        }
    } else {
        quote! {
            let Ok((#(#names),*)) = <(#(#types),*) as ::pvm_contract_sdk::SolDecode>::decode_at(&input,0) else {
                return revert(this);
            };
        }
    }
}

/// Build a compile-time expression for the minimum required input size.
pub fn calculate_min_input_size(types: &[syn::Type]) -> TokenStream {
    if types.is_empty() {
        return quote! { 0 };
    }
    let size_exprs: Vec<TokenStream> = types
        .iter()
        .map(|ty| {
            quote! { <#ty as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE }
        })
        .collect();
    quote! { 0 #(+ #size_exprs)* }
}
